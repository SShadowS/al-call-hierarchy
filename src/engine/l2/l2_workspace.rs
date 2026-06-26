//! Workspace-level L2 feature emitter (R1a Task 3).
//!
//! Drives the Task-2 single-DFS body walker (`project_routine_features`) over an
//! entire AL workspace and produces the ALLOWLISTED L2 projection
//! ([`features::L2Projection`]) — objects + routines with metadata + per-routine
//! `features`. This is the producer behind `aldump --l2`; it mirrors the golden
//! shape of `scripts/r1a-goldens/<fixture>.l2.golden.json` EXACTLY.
//!
//! Discovery + fail-closed layout detection + BOM strip + `.al` sort reproduce
//! R0's `snapshot_workspace` (`engine::snapshot`): a sound workspace is exactly
//! ONE AL app (a readable root `app.json` with a non-empty string `id`, deps
//! under skipped dirs). An unsound layout yields an EMPTY projection.
//!
//! Metadata derivation mirrors al-sem EXACTLY:
//!   - routine `attributes` / `attributesParsed` / `accessModifier` /
//!     `bodyAvailable` / `parseIncomplete`: `src/index/routine-indexer.ts`
//!     (`classifyAndCollectAttributes`, `classifyAccessModifier`) +
//!     `src/index/attribute-from-node.ts` (`attributeInfoFromNode`).
//!   - object `objectSubtype` / `pageType` / `sourceTableName` /
//!     `inherentCommitBehavior`: `src/index/object-indexer.ts` (`indexObjects`,
//!     `readObjectProperty`).
//!
//! R1b/R1c: `controlContext` + `order` + `scopeFrames` ARE now emitted (absent
//! when the CFN walker assigned none; scopeFrames present-with-root when a body
//! tree exists, omitted for TryFunction / no body). FORBIDDEN fields (capability /
//! resourceId / tableId / calleeParameterIsVar / bindingResolution /
//! sourceTableId) remain STRUCTURALLY ABSENT from the serde projection types
//! (`features.rs`), so they can never appear in this output.
//!
//! Output discipline: ONLY JSON goes to stdout (the binary prints it); all
//! logs/warnings go to stderr. The projection carries no absolute paths.

use super::features::{L2Projection, PFeatures, PObject, PRoutine};
use super::node_util::{named_children, node_text, strip_quotes, Utf16Cols};
use super::{extract_object_number, find_code_block, project_routine_features, IdentityCtx};
use crate::engine::ids::{
    encode_object_id, normalized_signature_hash, to_stable_object_id,
    to_stable_routine_id_from_parts, ParamSpec,
};
use crate::engine::l2::scope::{self, extract_object_globals, extract_parameters};
use serde_json::json;
use std::collections::HashSet;
use std::path::Path;
use tree_sitter::{Node, Parser};

/// The intentional stable corpus/model-instance label (matches the golden's id
/// prefixes `r0/…`). It does not enter the R1a stable comparison subset.
const MODEL_INSTANCE_ID: &str = "r0";

/// A discovered AL source file: its normalized workspace-relative POSIX path
/// (used both for deterministic sort AND as the `ws:<path>` sourceUnitId) and
/// the absolute path on disk.
#[derive(Debug, Clone)]
pub(crate) struct AlFile {
    /// Lowercased? NO — the sourceUnitId keeps original case; we sort on it
    /// lexicographically (matching al-sem's `ws:`-prefixed unit ids).
    pub(crate) rel_posix: String,
    pub(crate) abs_path: std::path::PathBuf,
}

/// Recursively discover `*.al` files under `workspace`, excluding dependency
/// dirs (`.alpackages`, `.git`). Mirrors R0's `discover_al_files`.
pub(crate) fn discover_al_files(workspace: &Path) -> std::io::Result<Vec<AlFile>> {
    let mut files = Vec::new();
    let mut stack = vec![workspace.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                // Mirror al-sem `workspace.ts` SKIP_DIR_EXACT EXACTLY: skip
                // `node_modules` and `.alpackages` by case-SENSITIVE exact match.
                // `.git` is NOT skipped (al-sem walks it; the `.al` extension filter
                // ignores its contents anyway). Matches the sibling `count_app_json`
                // skip pair (modulo case — both walkers must agree on the pair).
                let dname = entry.file_name().to_string_lossy().into_owned();
                if dname == "node_modules" || dname == ".alpackages" {
                    continue;
                }
                stack.push(path);
            } else if file_type.is_file() {
                let is_al = path
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase() == "al")
                    .unwrap_or(false);
                if is_al {
                    let rel = path.strip_prefix(workspace).unwrap_or(&path);
                    let rel_posix = rel
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join("/");
                    files.push(AlFile {
                        rel_posix,
                        abs_path: path,
                    });
                }
            }
        }
    }
    // Deterministic order — by the workspace-relative POSIX path.
    files.sort_by(|a, b| a.rel_posix.cmp(&b.rel_posix));
    Ok(files)
}

/// True when `dir` directly contains an `app.json` (case-insensitive) — i.e. it is
/// the root of a SEPARATE AL project. Used to stop discovery at nested-app
/// boundaries.
fn dir_has_app_json(dir: &Path) -> bool {
    match std::fs::read_dir(dir) {
        Ok(entries) => entries.flatten().any(|e| {
            e.file_name().to_string_lossy().to_lowercase() == "app.json"
                && e.file_type().map(|t| t.is_file()).unwrap_or(false)
        }),
        Err(_) => false,
    }
}

/// Like [`discover_al_files`] but scoped to ONE app: a child directory that carries
/// its own `app.json` is a separate AL project (the AL compiler treats each
/// `app.json` as a project root), so discovery does NOT descend into it. The
/// `workspace` root's own `app.json` does not stop the walk (it IS this app). This
/// lets a root app whose tree contains nested sub-apps (a monorepo / `Modules/`
/// layout) be analyzed in isolation, and lets each nested app be analyzed by
/// pointing at its own root. `node_modules` / `.alpackages` are still skipped.
pub(crate) fn discover_al_files_app_scoped(workspace: &Path) -> std::io::Result<Vec<AlFile>> {
    let mut files = Vec::new();
    let mut stack = vec![workspace.to_path_buf()];
    let mut is_root = true;
    while let Some(dir) = stack.pop() {
        // A nested app.json (anywhere but the scoped root) is a project boundary.
        if !is_root && dir_has_app_json(&dir) {
            continue;
        }
        is_root = false;
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                let dname = entry.file_name().to_string_lossy().into_owned();
                if dname == "node_modules" || dname == ".alpackages" {
                    continue;
                }
                stack.push(path);
            } else if file_type.is_file() {
                let is_al = path
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase() == "al")
                    .unwrap_or(false);
                if is_al {
                    let rel = path.strip_prefix(workspace).unwrap_or(&path);
                    let rel_posix = rel
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join("/");
                    files.push(AlFile {
                        rel_posix,
                        abs_path: path,
                    });
                }
            }
        }
    }
    files.sort_by(|a, b| a.rel_posix.cmp(&b.rel_posix));
    Ok(files)
}

/// Read a file as UTF-8, stripping a leading UTF-8 BOM if present (matches TS).
pub(crate) fn read_al_source(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

/// Read the workspace ROOT's `app.json` `id` field VERBATIM when it is a
/// non-empty string. Mirrors `providers/workspace.ts` (GAP 2).
pub(crate) fn read_root_app_guid(workspace: &Path) -> Option<String> {
    let text = std::fs::read_to_string(workspace.join("app.json")).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&text).ok()?;
    let id = value.get("id")?.as_str()?;
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

/// Count `app.json` files anywhere under `workspace`, EXCLUDING `node_modules`
/// and `.alpackages` (case-insensitive). Mirrors al-sem `SKIP_DIR_EXACT`.
pub(crate) fn count_app_json(workspace: &Path) -> usize {
    count_app_json_paths(workspace).len()
}

/// Collect the absolute paths of every `app.json` anywhere under `workspace`,
/// EXCLUDING `node_modules` and `.alpackages` (case-insensitive). Mirrors al-sem
/// `SKIP_DIR_EXACT`. Used by the gate's `workspace_diagnostics` to reproduce the
/// provider's multi-app fail-closed message (which sorts these paths).
pub(crate) fn count_app_json_paths(workspace: &Path) -> Vec<std::path::PathBuf> {
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    let mut stack = vec![workspace.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ftype) = entry.file_type() else {
                continue;
            };
            if ftype.is_dir() {
                let dname_lc = entry.file_name().to_string_lossy().to_lowercase();
                if dname_lc == "node_modules" || dname_lc == ".alpackages" {
                    continue;
                }
                stack.push(entry.path());
            } else if ftype.is_file()
                && entry.file_name().to_string_lossy().to_lowercase() == "app.json"
            {
                paths.push(entry.path());
            }
        }
    }
    paths
}

// ---------------------------------------------------------------------------
// Metadata extraction — mirrors al-sem object-indexer.ts / routine-indexer.ts /
// attribute-from-node.ts.
// ---------------------------------------------------------------------------

/// `extractObjectName` — first quoted_identifier (stripped) or identifier, else "".
fn extract_object_name(decl: Node, source: &str) -> String {
    for child in named_children(decl) {
        match child.kind() {
            "quoted_identifier" => return strip_quotes(node_text(child, source)).to_string(),
            "identifier" => return node_text(child, source).to_string(),
            _ => {}
        }
    }
    String::new()
}

/// `readObjectProperty` — first direct `property` named child whose `name` field
/// matches (case-insensitive); returns the raw `value` field text. Mirrors
/// object-indexer.ts (DIRECT children only — never descends).
fn read_object_property(decl: Node, property_name: &str, source: &str) -> Option<String> {
    let want = property_name.to_lowercase();
    // tree-sitter-al v3 wraps the object body in a `declaration_body` (the
    // `body` field), so object-level properties (Subtype, SourceTable, ...) are
    // no longer direct children of the declaration. Look inside the body when
    // present; fall back to the declaration for older grammars.
    let container = decl.child_by_field_name("body").unwrap_or(decl);
    for child in named_children(container) {
        if child.kind() != "property" {
            continue;
        }
        let Some(name_node) = child.child_by_field_name("name") else {
            continue;
        };
        if node_text(name_node, source).to_lowercase() != want {
            continue;
        }
        return child
            .child_by_field_name("value")
            .map(|v| node_text(v, source).to_string());
    }
    None
}

/// Strip a single layer of surrounding double OR single quotes (mirrors
/// attribute-from-node.ts `stripQuoteChars`).
fn strip_quote_chars(text: &str) -> &str {
    let mut chars = text.chars();
    let first = chars.next();
    let last = chars.next_back();
    if (first == Some('"') && last == Some('"')) || (first == Some('\'') && last == Some('\'')) {
        &text[1..text.len() - 1]
    } else {
        text
    }
}

/// Classify a grammar node type into the AttributeArgKind string (else "unknown").
fn attr_arg_kind(node_type: &str) -> &'static str {
    match node_type {
        "boolean" => "boolean",
        "integer" => "integer",
        "string_literal" => "string_literal",
        "identifier" => "identifier",
        "quoted_identifier" => "quoted_identifier",
        "qualified_enum_value" => "qualified_enum_value",
        "database_reference" => "database_reference",
        "member_expression" => "member_expression",
        _ => "unknown",
    }
}

/// Build a single `AttributeArg` JSON object (mirrors `argFromNode`).
fn attr_arg_from_node(node: Node, source: &str) -> serde_json::Value {
    let kind = attr_arg_kind(node.kind());
    let text = node_text(node, source).to_string();
    let mut obj = serde_json::Map::new();
    obj.insert("kind".to_string(), json!(kind));
    obj.insert("text".to_string(), json!(text));

    // deriveValueParts.
    match kind {
        "boolean" | "integer" | "identifier" => {
            obj.insert("value".to_string(), json!(text));
        }
        "string_literal" | "quoted_identifier" => {
            obj.insert("value".to_string(), json!(strip_quote_chars(&text)));
        }
        "qualified_enum_value" => {
            let qualifier = node
                .child_by_field_name("enum_type")
                .map(|n| node_text(n, source).to_string());
            let member = node
                .child_by_field_name("value")
                .map(|n| strip_quote_chars(node_text(n, source)).to_string());
            if let Some(m) = &member {
                obj.insert("value".to_string(), json!(m));
            }
            if let Some(q) = qualifier {
                obj.insert("qualifier".to_string(), json!(q));
            }
            if let Some(m) = member {
                obj.insert("member".to_string(), json!(m));
            }
        }
        "database_reference" => {
            let qualifier = node
                .child_by_field_name("keyword")
                .map(|n| node_text(n, source).to_string());
            let member = node
                .child_by_field_name("table_name")
                .map(|n| strip_quote_chars(node_text(n, source)).to_string());
            if let Some(m) = &member {
                obj.insert("value".to_string(), json!(m));
            }
            if let Some(q) = qualifier {
                obj.insert("qualifier".to_string(), json!(q));
            }
            if let Some(m) = member {
                obj.insert("member".to_string(), json!(m));
            }
        }
        // member_expression / unknown → no value parts.
        _ => {}
    }
    serde_json::Value::Object(obj)
}

/// `attributeInfoFromNode` — structured `{name, args, raw}` JSON, or None when
/// the attribute shape is unrecognizable (parse error).
pub fn attribute_info_from_node(item: Node, source: &str) -> Option<serde_json::Value> {
    let content = item.child_by_field_name("attribute")?;
    let name = content
        .child_by_field_name("name")
        .map(|n| node_text(n, source).to_string())
        .unwrap_or_default();
    if name.is_empty() {
        return None;
    }
    let mut args = Vec::new();
    if let Some(args_node) = content.child_by_field_name("arguments") {
        if let Some(list) = named_children(args_node)
            .into_iter()
            .find(|c| c.kind() == "attribute_argument_list")
        {
            for child in named_children(list) {
                args.push(attr_arg_from_node(child, source));
            }
        }
    }
    Some(json!({
        "name": name,
        "args": args,
        "raw": node_text(item, source),
    }))
}

/// `classifyAndCollectAttributes` — raw `attributes` (document order) +
/// structured `attributesParsed` (document order) by walking preceding
/// `attribute_item` siblings.
pub fn collect_attributes(node: Node, source: &str) -> (Vec<String>, Vec<serde_json::Value>) {
    let mut attributes: Vec<String> = Vec::new();
    let mut parsed: Vec<serde_json::Value> = Vec::new();
    let mut sibling = node.prev_sibling();
    while let Some(sib) = sibling {
        if sib.kind() != "attribute_item" {
            break;
        }
        // unshift to keep document order.
        attributes.insert(0, node_text(sib, source).to_string());
        if let Some(info) = attribute_info_from_node(sib, source) {
            parsed.insert(0, info);
        }
        sibling = sib.prev_sibling();
    }
    (attributes, parsed)
}

/// `classifyAccessModifier` — the `modifier` field on a `procedure` node
/// (`local`/`internal`/`protected`); None for triggers / default-access.
/// `pub(crate)` so the L3 assembly path can call it directly (d32 scope gate).
pub(crate) fn classify_access_modifier(node: Node, source: &str) -> Option<String> {
    if node.kind() != "procedure" {
        return None;
    }
    let modifier = node.child_by_field_name("modifier")?;
    let text = node_text(modifier, source).trim().to_lowercase();
    match text.as_str() {
        "local" => Some("local".to_string()),
        "internal" => Some("internal".to_string()),
        "protected" => Some("protected".to_string()),
        _ => None,
    }
}

/// `classifyAndCollectAttributes` (kind only) — base kind + first event attr.
pub fn classify_kind(node: Node, source: &str) -> &'static str {
    let mut kind = if node.kind() == "trigger_declaration" {
        "trigger"
    } else {
        "procedure"
    };
    let mut sibling = node.prev_sibling();
    while let Some(sib) = sibling {
        if sib.kind() != "attribute_item" {
            break;
        }
        if let Some(info) = attribute_info_from_node(sib, source) {
            if let Some(name) = info.get("name").and_then(|n| n.as_str()) {
                let name_lc = name.to_lowercase();
                if name_lc == "eventsubscriber" {
                    kind = "event-subscriber";
                    break;
                } else if name_lc == "integrationevent" || name_lc == "businessevent" {
                    kind = "event-publisher";
                    break;
                }
            }
        }
        sibling = sib.prev_sibling();
    }
    kind
}

/// `(objectSubtype, pageType, sourceTableName, inherentCommitBehavior)` for an
/// object decl — mirrors object-indexer.ts `indexObjects`.
fn extract_object_metadata(
    decl: Node,
    object_type: &str,
    source: &str,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    let mut object_subtype = None;
    let mut page_type = None;
    let mut source_table_name = None;
    let mut inherent_commit_behavior = None;

    if object_type == "Codeunit" {
        object_subtype = read_object_property(decl, "Subtype", source);
    }
    if object_type == "Page" || object_type == "PageExtension" {
        page_type = read_object_property(decl, "PageType", source);
        source_table_name =
            read_object_property(decl, "SourceTable", source).map(|s| strip_quotes(&s).to_string());
    }
    if object_type == "Codeunit" || object_type == "Table" || object_type == "TableExtension" {
        if let Some(icb_raw) = read_object_property(decl, "InherentCommitBehavior", source) {
            let member = match icb_raw.rfind("::") {
                Some(sep) => icb_raw[sep + 2..].to_lowercase(),
                None => icb_raw.to_lowercase(),
            };
            inherent_commit_behavior = match member.as_str() {
                "ignore" => Some("ignore".to_string()),
                "error" => Some("error".to_string()),
                "allow" => Some("allow".to_string()),
                _ => None,
            };
        }
    }

    (
        object_subtype,
        page_type,
        source_table_name,
        inherent_commit_behavior,
    )
}

/// Return-type text — first direct `type_specification` named child (parameter
/// types nest inside `parameter` nodes). Mirrors `getReturnTypeText`.
fn return_type_text(node: Node, source: &str) -> Option<String> {
    named_children(node)
        .into_iter()
        .find(|c| c.kind() == "type_specification")
        .map(|c| node_text(c, source).to_string())
}

/// `collectDescendants(prune-at-match)` for procedure / trigger_declaration.
fn collect_routine_nodes(decl: Node) -> Vec<Node> {
    let mut out = Vec::new();
    let mut stack = vec![decl];
    while let Some(node) = stack.pop() {
        if node.kind() == "procedure" || node.kind() == "trigger_declaration" {
            out.push(node);
            continue;
        }
        for child in named_children(node) {
            stack.push(child);
        }
    }
    out
}

/// Build the full L2 projection for one parsed source file's tree.
#[allow(clippy::too_many_arguments)]
fn project_file(
    root: Node,
    source: &str,
    app_guid: &str,
    source_unit_id: &str,
    cols: &Utf16Cols,
    objects: &mut Vec<PObject>,
    routines: &mut Vec<PRoutine>,
) {
    for decl in named_children(root) {
        let Some(object_type) = scope::object_type_for(decl.kind()) else {
            continue;
        };
        let object_number = extract_object_number(decl, source);
        let name = extract_object_name(decl, source);

        let internal_object_id = encode_object_id(app_guid, object_type, object_number);
        let stable_object_id = to_stable_object_id(&internal_object_id);

        let (object_subtype, page_type, source_table_name, inherent_commit_behavior) =
            extract_object_metadata(decl, object_type, source);

        objects.push(PObject {
            stable_object_id: stable_object_id.clone(),
            name,
            object_type: object_type.to_string(),
            object_subtype,
            page_type,
            source_table_name: source_table_name.clone(),
            inherent_commit_behavior,
        });

        // Object globals (shared across routines).
        let object_globals = extract_object_globals(decl, source_unit_id, source);

        // Routine nodes (prune-at-match).
        let routine_nodes = collect_routine_nodes(decl);

        // Object procedure-name collision set (implicit-receiver §3.3).
        let mut object_procedure_names = HashSet::new();
        for n in &routine_nodes {
            if let Some(nm) = n.child_by_field_name("name") {
                object_procedure_names.insert(strip_quotes(node_text(nm, source)).to_lowercase());
            }
        }

        let id_ctx = IdentityCtx {
            app_guid,
            model_instance_id: MODEL_INSTANCE_ID,
            source_unit_id,
        };

        for routine in routine_nodes {
            let Some(nm) = routine.child_by_field_name("name") else {
                continue;
            };
            let rname = strip_quotes(node_text(nm, source)).to_string();
            if rname.is_empty() {
                continue;
            }

            let kind = classify_kind(routine, source);
            let (attributes, attributes_parsed) = collect_attributes(routine, source);
            let access_modifier = classify_access_modifier(routine, source);
            let body_available = find_code_block(routine).is_some();
            let parse_incomplete = routine.has_error();

            // Stable routine id — its normalizedSignatureHash is the canonical
            // (return-type-aware) signature hash, identical on the ABI side.
            let parameters = extract_parameters(routine, source);
            let param_specs: Vec<ParamSpec> = parameters
                .iter()
                .map(|p| ParamSpec {
                    type_text: p.type_text.clone(),
                    is_var: p.is_var,
                })
                .collect();
            let return_type_text = return_type_text(routine, source);
            let norm_hash =
                normalized_signature_hash(&rname, &param_specs, return_type_text.as_deref());
            let stable_routine_id = to_stable_routine_id_from_parts(&stable_object_id, &norm_hash);

            let mut features: PFeatures = match project_routine_features(
                decl,
                routine,
                object_type,
                object_number,
                source_table_name.as_deref(),
                &object_procedure_names,
                &object_globals,
                &id_ctx,
                source,
                cols,
            ) {
                Some((_, f)) => f,
                None => continue,
            };

            // R1b: control-context lattice over the CFN skeleton (+ metadata).
            // Populates `controlContext` on each op/callsite (absent when none),
            // including the error-call source-range post-pass. `attributesParsed`
            // names drive the TryFunction guard; `parameters` the by-var Boolean
            // IsHandled eligibility.
            let attr_names_lc: Vec<String> = attributes_parsed
                .iter()
                .filter_map(|a| a.get("name").and_then(|n| n.as_str()))
                .map(|n| n.to_lowercase())
                .collect();
            crate::engine::l2::control_context::apply_control_contexts(
                &mut features,
                &attr_names_lc,
                &parameters,
            );

            // R1c: operation-order index over the CFN skeleton (+ TryFunction
            // guard). Populates `order` on each op/callsite (absent when the walk
            // produced none) — including the error-call source-range post-pass over
            // the op/callsite records — and the routine's `scopeFrames`.
            crate::engine::l2::operation_order::apply_operation_order(
                &mut features,
                &attr_names_lc,
            );

            let mut routine = PRoutine {
                stable_routine_id,
                name: rname,
                kind: kind.to_string(),
                attributes,
                attributes_parsed,
                access_modifier,
                body_available,
                parse_incomplete,
                features,
                capability_facts_direct: Vec::new(),
                capability_status: crate::engine::l2::capability::CoverageStatus::Complete,
                capability_reasons: Vec::new(),
                capability_diagnostics: Vec::new(),
            };

            // R1d: direct capability facts. MUST run AFTER controlContext is set
            // (the unreachable filter in `extract_capabilities` reads it).
            apply_capabilities(&mut routine);

            routines.push(routine);
        }
    }
}

/// R1d emitter wiring: run `extract_capabilities` on the (control-context-set)
/// routine and populate the four sibling-of-`features` capability fields, ordered
/// to match the al-sem golden projection (`r1a-l2-projection.ts`):
///   - `capabilityFactsDirect`: extraction order (positional) — NO sort.
///   - `capabilityReasons`: dedupe + LEXICOGRAPHIC sort on the kebab string
///     (al-sem `Array.from(new Set(reasons)).sort()` — JS string sort, NOT the
///     `CoverageReason` declaration order).
///   - `capabilityDiagnostics`: sort by `(sourceRef, message)`.
fn apply_capabilities(routine: &mut PRoutine) {
    let result = crate::engine::l2::capability::extract_capabilities(routine);

    let mut reasons = result.reasons;
    // Match al-sem's `.sort()` (lexicographic on the serialized kebab string),
    // not the enum's declaration-order `Ord`.
    reasons.sort_by(|a, b| a.as_str().cmp(b.as_str()));

    let mut diagnostics = result.diagnostics;
    diagnostics.sort_by(|a, b| {
        a.source_ref
            .cmp(&b.source_ref)
            .then_with(|| a.message.cmp(&b.message))
    });

    routine.capability_facts_direct = result.facts;
    routine.capability_status = result.status;
    routine.capability_reasons = reasons;
    routine.capability_diagnostics = diagnostics;
}

/// Build the full L2 projection for a workspace directory.
///
/// Errors: a missing / unreadable workspace surfaces as `Err` for a clean
/// non-zero exit (thin CLI helper — the engine pipeline itself never throws).
/// An UNSOUND layout (no root app.json `id`, or multi-app source tree) is NOT an
/// error — it yields an EMPTY projection (fail-closed), matching R0.
pub fn project_workspace(workspace: &Path) -> anyhow::Result<L2Projection> {
    if !workspace.is_dir() {
        anyhow::bail!("workspace is not a directory: {}", workspace.display());
    }

    let empty = || L2Projection {
        objects: Vec::new(),
        routines: Vec::new(),
    };

    // --- fail-closed layout detection (mirrors R0 snapshot_workspace) ----------
    let app_guid = match read_root_app_guid(workspace) {
        Some(g) => g,
        None => {
            eprintln!(
                "fail-closed: no readable root app.json with a string `id` at {} — emitting empty projection",
                workspace.display()
            );
            return Ok(empty());
        }
    };
    let app_json_count = count_app_json(workspace);
    if app_json_count > 1 {
        eprintln!(
            "fail-closed: multi-app source workspace at {} ({app_json_count} app.json files, excl. node_modules/.alpackages) — emitting empty projection",
            workspace.display()
        );
        return Ok(empty());
    }

    let files = discover_al_files(workspace)
        .map_err(|e| anyhow::anyhow!("failed to discover .al files: {e}"))?;

    let mut parser = Parser::new();
    parser
        .set_language(&crate::language::language())
        .map_err(|e| anyhow::anyhow!("failed to set tree-sitter language: {e}"))?;

    let mut projection = empty();

    for file in &files {
        let source = match read_al_source(&file.abs_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("warning: skipping {} (read error: {e})", file.rel_posix);
                continue;
            }
        };
        let Some(tree) = parser.parse(&source, None) else {
            eprintln!(
                "warning: skipping {} (parse returned no tree)",
                file.rel_posix
            );
            continue;
        };
        let source_unit_id = format!("ws:{}", file.rel_posix);
        let cols = Utf16Cols::new(&source);
        project_file(
            tree.root_node(),
            &source,
            &app_guid,
            &source_unit_id,
            &cols,
            &mut projection.objects,
            &mut projection.routines,
        );
    }

    // Top-level: objects sorted by StableObjectId, routines by StableRoutineId.
    projection
        .objects
        .sort_by(|a, b| a.stable_object_id.cmp(&b.stable_object_id));
    projection
        .routines
        .sort_by(|a, b| a.stable_routine_id.cmp(&b.stable_routine_id));

    Ok(projection)
}

/// Project a single NAMED routine from a single-file `source` into a full
/// [`PRoutine`] (features + control-context + operation-order applied, plus the
/// routine-level metadata: `attributes`/`attributesParsed`/`accessModifier`/
/// `bodyAvailable`/`parseIncomplete`). Mirrors the per-routine body of
/// [`project_file`] EXACTLY — it is the single-routine entry point used by the
/// R1d capability vector tests (which need a fully-populated `PRoutine`, with
/// `controlContext` set so the unreachable filter fires, plus the internal-id
/// `op*`/`cs*` witness references that the capability facts carry).
///
/// Returns `None` when the named routine isn't found in any object.
pub fn project_named_routine(
    source: &str,
    routine_name: &str,
    app_guid: &str,
    source_unit_id: &str,
    tree: &tree_sitter::Tree,
) -> Option<PRoutine> {
    let root = tree.root_node();
    let cols = Utf16Cols::new(source);

    for decl in named_children(root) {
        let Some(object_type) = scope::object_type_for(decl.kind()) else {
            continue;
        };
        let object_number = extract_object_number(decl, source);
        let internal_object_id = encode_object_id(app_guid, object_type, object_number);
        let stable_object_id = to_stable_object_id(&internal_object_id);

        let (_, _, source_table_name, _) = extract_object_metadata(decl, object_type, source);

        let object_globals = extract_object_globals(decl, source_unit_id, source);
        let routine_nodes = collect_routine_nodes(decl);

        let mut object_procedure_names = HashSet::new();
        for n in &routine_nodes {
            if let Some(nm) = n.child_by_field_name("name") {
                object_procedure_names.insert(strip_quotes(node_text(nm, source)).to_lowercase());
            }
        }

        let id_ctx = IdentityCtx {
            app_guid,
            model_instance_id: MODEL_INSTANCE_ID,
            source_unit_id,
        };

        for routine in routine_nodes {
            let Some(nm) = routine.child_by_field_name("name") else {
                continue;
            };
            let rname = strip_quotes(node_text(nm, source)).to_string();
            if rname != routine_name {
                continue;
            }

            let kind = classify_kind(routine, source);
            let (attributes, attributes_parsed) = collect_attributes(routine, source);
            let access_modifier = classify_access_modifier(routine, source);
            let body_available = find_code_block(routine).is_some();
            let parse_incomplete = routine.has_error();

            let parameters = extract_parameters(routine, source);
            let param_specs: Vec<ParamSpec> = parameters
                .iter()
                .map(|p| ParamSpec {
                    type_text: p.type_text.clone(),
                    is_var: p.is_var,
                })
                .collect();
            let return_type_text = return_type_text(routine, source);
            let norm_hash =
                normalized_signature_hash(&rname, &param_specs, return_type_text.as_deref());
            let stable_routine_id = to_stable_routine_id_from_parts(&stable_object_id, &norm_hash);

            let mut features: PFeatures = project_routine_features(
                decl,
                routine,
                object_type,
                object_number,
                source_table_name.as_deref(),
                &object_procedure_names,
                &object_globals,
                &id_ctx,
                source,
                &cols,
            )
            .map(|(_, f)| f)?;

            let attr_names_lc: Vec<String> = attributes_parsed
                .iter()
                .filter_map(|a| a.get("name").and_then(|n| n.as_str()))
                .map(|n| n.to_lowercase())
                .collect();
            crate::engine::l2::control_context::apply_control_contexts(
                &mut features,
                &attr_names_lc,
                &parameters,
            );
            crate::engine::l2::operation_order::apply_operation_order(
                &mut features,
                &attr_names_lc,
            );

            let mut routine = PRoutine {
                stable_routine_id,
                name: rname,
                kind: kind.to_string(),
                attributes,
                attributes_parsed,
                access_modifier,
                body_available,
                parse_incomplete,
                features,
                capability_facts_direct: Vec::new(),
                capability_status: crate::engine::l2::capability::CoverageStatus::Complete,
                capability_reasons: Vec::new(),
                capability_diagnostics: Vec::new(),
            };
            apply_capabilities(&mut routine);
            return Some(routine);
        }
    }
    None
}
