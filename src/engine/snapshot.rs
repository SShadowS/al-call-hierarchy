//! R0 identity-subset extraction — the structural walker behind `aldump`.
//!
//! This module parses an AL workspace and derives the object/routine *identity
//! subset* that the R0 differential harness (Task 5) diffs against al-sem's
//! committed "golden" files. It deliberately reproduces al-sem's identity
//! derivation EXACTLY:
//!
//! - objects: `src/index/object-indexer.ts` (OBJECT_TYPE_MAP, extractObjectNumber, extractObjectName, indexObjects)
//! - routines: `src/index/routine-indexer.ts` (classifyAndCollectAttributes, getReturnTypeText, collectDescendants prune-at-match)
//! - params: `src/index/intraprocedural-refs.ts` (extractParameters)
//! - strip: `src/parser/ast.ts` (stripQuotes)
//! - attr name: `src/index/attribute-from-node.ts`
//!
//! DESIGN DEVIATION (deliberate, decided by the R0 controller): the R0 plan's
//! Task 4 wording says "emit a v3-shaped CapabilitySnapshot with L1+ arrays
//! empty." We do NOT do that. R0 compares the *identity subset* (plan REVIEW #9:
//! "compare parsed structures, not byte-identical JSON"), and that subset carries
//! fields that the production v3 snapshot does not even have — routine sub-kind
//! and `canonicalSignatureText`. A v3 envelope could not carry them, and building
//! the full v3 serde type-zoo just to leave it empty is work that belongs to the
//! final byte-identical-snapshot phase. So `aldump` emits the identity-subset
//! JSON directly, in the golden's exact shape (see `IdentitySnapshot`).
//!
//! GRAMMAR NOTE: this parses with the currently bundled fork grammar
//! (`crate::language::language()`). The swap to the canonical grammar is R0
//! Task 6 (deliberately LAST). If the fork grammar yields a different AST shape
//! than al-sem's WASM grammar for some construct, the resulting identity may
//! diverge from the golden — that is expected pre-Task-6 and is reconciled there,
//! not papered over here.

use crate::engine::ids::{
    canonical_routine_signature, encode_object_id, normalized_signature_hash,
    object_signature_fingerprint, routine_signature_fingerprint, to_stable_object_id,
    to_stable_routine_id_from_parts, ParamSpec,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tree_sitter::{Node, Parser};

/// The identity subset of a single AL object declaration, in the golden's shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObjectIdentity {
    #[serde(rename = "stableObjectId")]
    pub stable_object_id: String,
    pub name: String,
    /// The display object-type string (e.g. "Codeunit", "XMLport").
    pub kind: String,
    #[serde(rename = "signatureFingerprint")]
    pub signature_fingerprint: String,
}

/// The identity subset of a single routine, in the golden's shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutineIdentity {
    #[serde(rename = "stableRoutineId")]
    pub stable_routine_id: String,
    pub name: String,
    /// "procedure" | "trigger" | "event-publisher" | "event-subscriber".
    pub kind: String,
    #[serde(rename = "signatureFingerprint")]
    pub signature_fingerprint: String,
    #[serde(rename = "normalizedSignatureHash")]
    pub normalized_signature_hash: String,
    #[serde(rename = "canonicalSignatureText")]
    pub canonical_signature_text: String,
}

/// The complete identity-subset snapshot for a workspace.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IdentitySnapshot {
    pub objects: Vec<ObjectIdentity>,
    pub routines: Vec<RoutineIdentity>,
}

/// Map a V2-grammar object declaration node type to its display object-type
/// name. Mirrors al-sem's `OBJECT_TYPE_MAP` EXACTLY — note `xmlport_declaration`
/// → "XMLport" and the deliberate absence of `permissionsetextension`
/// (al-sem skips that object type, so we must too).
fn object_type_for(node_type: &str) -> Option<&'static str> {
    Some(match node_type {
        "codeunit_declaration" => "Codeunit",
        "table_declaration" => "Table",
        "tableextension_declaration" => "TableExtension",
        "page_declaration" => "Page",
        "pageextension_declaration" => "PageExtension",
        "report_declaration" => "Report",
        "reportextension_declaration" => "ReportExtension",
        "query_declaration" => "Query",
        "xmlport_declaration" => "XMLport",
        "enum_declaration" => "Enum",
        "enumextension_declaration" => "EnumExtension",
        "interface_declaration" => "Interface",
        "controladdin_declaration" => "ControlAddIn",
        "permissionset_declaration" => "PermissionSet",
        _ => return None,
    })
}

/// Strip surrounding double quotes — matches al-sem's `stripQuotes` exactly:
/// only strips when the text is >= 2 chars AND starts with `"` AND ends with
/// `"`. (A lone `"` or unbalanced quotes are returned verbatim.) This counts
/// length in `char`s, but the predicate is on ASCII `"` so byte vs char length
/// is immaterial for the >= 2 guard except for the degenerate single-char case,
/// which neither side strips.
fn strip_quotes(text: &str) -> &str {
    let mut chars = text.chars();
    let first = chars.next();
    let last = chars.next_back();
    // `chars.next()` then `chars.next_back()` having both yielded a char means
    // the string has >= 2 chars.
    if first == Some('"') && last == Some('"') {
        // Slice off the leading and trailing ASCII quote (1 byte each).
        &text[1..text.len() - 1]
    } else {
        text
    }
}

/// Source text of a node.
fn node_text<'a>(node: Node, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}

/// Iterate a node's NAMED children (mirrors al-sem `namedChildren`).
fn named_children<'a>(node: Node<'a>) -> impl Iterator<Item = Node<'a>> {
    (0..node.named_child_count() as u32).filter_map(move |i| node.named_child(i))
}

/// `extractObjectNumber`: first `integer` named child parsed as int, else 0.
fn extract_object_number(decl: Node, source: &str) -> i64 {
    for child in named_children(decl) {
        if child.kind() == "integer" {
            return node_text(child, source).trim().parse::<i64>().unwrap_or(0);
        }
    }
    0
}

/// `extractObjectName`: first `quoted_identifier` (stripQuotes) or `identifier`
/// (verbatim) named child, else "".
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

/// Parse an `attribute_item` node's attribute NAME (lowercased comparison done
/// by the caller). Mirrors `attributeInfoFromNode`: the name is the `name` field
/// of the `attribute` (attribute_content) field child. Returns None when the
/// shape is unrecognizable (parse error) — matching al-sem's null fallback.
fn attribute_name<'a>(item: Node, source: &'a str) -> Option<&'a str> {
    let content = item.child_by_field_name("attribute")?;
    let name_node = content.child_by_field_name("name")?;
    let name = node_text(name_node, source);
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// `classifyAndCollectAttributes` (kind only). Base kind is "trigger" for a
/// `trigger_declaration` node else "procedure". Walk PREVIOUS siblings while
/// they are `attribute_item`; the FIRST (closest-sibling) event attribute wins:
///   name-lc == "eventsubscriber"            → "event-subscriber"
///   name-lc == "integrationevent"|"businessevent" → "event-publisher"
/// All other attributes (incl. InternalEvent) leave the kind unchanged — this
/// matches AL-SEM, NOT the LSP parser (which treats InternalEvent as publisher).
fn classify_kind(node: Node, source: &str) -> String {
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
        if let Some(name) = attribute_name(sib, source) {
            let name_lc = name.to_lowercase();
            if name_lc == "eventsubscriber" {
                kind = "event-subscriber";
                break;
            } else if name_lc == "integrationevent" || name_lc == "businessevent" {
                kind = "event-publisher";
                break;
            }
        }
        sibling = sib.prev_sibling();
    }
    kind.to_string()
}

/// `extractParameters`: for each `parameter` named child of the routine's
/// `parameter_list`, produce a ParamSpec carrying only the bits the signature
/// needs — `type_text` (the `type_specification` named child's text, or "") and
/// `is_var` (presence of a `var_keyword` named child).
///
/// ORACLE BEHAVIOR (intraprocedural-refs.ts:44-86, GAP 1 fix): al-sem locates the
/// parameter NAME node as the FIRST named child of kind `identifier` OR
/// `quoted_identifier`, and only SKIPS a parameter when it has NEITHER (no name
/// node at all → a parse artifact). A parameter whose name is a *quoted*
/// identifier (e.g. `"Sales Header": Record "Sales Header"`) is therefore KEPT,
/// so its `type_specification` text enters the canonical signature. We don't need
/// the name itself for the identity subset (only `type_text` + `is_var` matter),
/// but we must mirror the skip predicate exactly so the param set matches the
/// oracle's (driven by ws-r0-canon-stress `DoWork`'s `"Sales Header"` param).
fn extract_parameters(node: Node, source: &str) -> Vec<ParamSpec> {
    let mut params = Vec::new();
    let Some(param_list) = named_children(node).find(|c| c.kind() == "parameter_list") else {
        return params;
    };
    for param in named_children(param_list) {
        if param.kind() != "parameter" {
            continue;
        }
        // Mirror al-sem GAP 1: skip only when there is NO name node at all
        // (neither `identifier` nor `quoted_identifier`). Quoted-name params ARE
        // kept so their type enters the signature.
        let has_name = named_children(param)
            .any(|c| c.kind() == "identifier" || c.kind() == "quoted_identifier");
        if !has_name {
            continue;
        }
        let is_var = named_children(param).any(|c| c.kind() == "var_keyword");
        let type_text = named_children(param)
            .find(|c| c.kind() == "type_specification")
            .map(|c| node_text(c, source).to_string())
            .unwrap_or_default();
        params.push(ParamSpec { type_text, is_var });
    }
    params
}

/// `getReturnTypeText`: the FIRST direct named child of the routine node whose
/// type is `type_specification`. Parameter type_specs are nested inside
/// `parameter` nodes, so the first direct one is unambiguously the return type.
/// None for triggers / void procedures.
fn extract_return_type(node: Node, source: &str) -> Option<String> {
    named_children(node)
        .find(|c| c.kind() == "type_specification")
        .map(|c| node_text(c, source).to_string())
}

/// Depth-first collect descendant nodes matching `pred`, pruning at a match
/// (do not descend into a matched node). Mirrors al-sem's `collectDescendants`
/// with `pruneAtMatch = true`. Traversal order is NOT document order (al-sem
/// re-sorts routines by source position; identity does not depend on order, and
/// the snapshot re-sorts by id anyway).
fn collect_descendants_pruned<'a>(
    root: Node<'a>,
    pred: &dyn Fn(Node<'a>) -> bool,
) -> Vec<Node<'a>> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if pred(node) {
            out.push(node);
            continue; // prune: do not descend into a matched routine
        }
        for child in named_children(node) {
            stack.push(child);
        }
    }
    out
}

/// Build the identity subset for one parsed source file's syntax tree.
fn extract_from_tree(root: Node, source: &str, app_guid: &str, out: &mut IdentitySnapshot) {
    // Object declarations = the root's NAMED children whose type is a known
    // object-decl type (al-sem `findObjectDeclarations`).
    for decl in named_children(root) {
        let Some(object_type) = object_type_for(decl.kind()) else {
            continue;
        };
        let object_number = extract_object_number(decl, source);
        let name = extract_object_name(decl, source);

        let internal_object_id = encode_object_id(app_guid, object_type, object_number);
        let stable_object_id = to_stable_object_id(&internal_object_id);
        let signature_fingerprint = object_signature_fingerprint(object_type, object_number, &name);

        out.objects.push(ObjectIdentity {
            stable_object_id: stable_object_id.clone(),
            name,
            kind: object_type.to_string(),
            signature_fingerprint,
        });

        // Routines = `procedure` / `trigger_declaration` descendants of the
        // object decl (prune-at-match).
        let routine_nodes = collect_descendants_pruned(decl, &|n| {
            n.kind() == "procedure" || n.kind() == "trigger_declaration"
        });

        for rnode in routine_nodes {
            let Some(name_node) = rnode.child_by_field_name("name") else {
                continue;
            };
            let rname = strip_quotes(node_text(name_node, source)).to_string();
            if rname.is_empty() {
                continue;
            }

            let kind = classify_kind(rnode, source);
            let parameters = extract_parameters(rnode, source);
            let return_type = extract_return_type(rnode, source);
            let return_type_ref = return_type.as_deref();

            let canonical_signature_text =
                canonical_routine_signature(&rname, &parameters, return_type_ref);
            let norm_hash = normalized_signature_hash(&rname, &parameters, return_type_ref);
            let sig_fp = routine_signature_fingerprint(&rname, &parameters, return_type_ref);
            let stable_routine_id = to_stable_routine_id_from_parts(&stable_object_id, &norm_hash);

            out.routines.push(RoutineIdentity {
                stable_routine_id,
                name: rname,
                kind,
                signature_fingerprint: sig_fp,
                normalized_signature_hash: norm_hash,
                canonical_signature_text,
            });
        }
    }
}

/// A discovered AL source file: its normalized workspace-relative POSIX path
/// (used for deterministic sort) and absolute path on disk.
#[derive(Debug, Clone)]
struct AlFile {
    /// Lowercased, forward-slash, workspace-relative path — the sort key.
    sort_key: String,
    abs_path: std::path::PathBuf,
}

/// Recursively discover `*.al` files under `workspace`, excluding dependency
/// dirs (`.alpackages`, `.app` payloads). R0 is source-symbol parity, so only
/// the workspace's own AL source is in scope.
fn discover_al_files(workspace: &Path) -> std::io::Result<Vec<AlFile>> {
    let mut files = Vec::new();
    let mut stack = vec![workspace.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                let dname = entry.file_name();
                let dname_lc = dname.to_string_lossy().to_lowercase();
                // Skip dependency / package dirs.
                if dname_lc == ".alpackages" || dname_lc == ".git" {
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
                    let sort_key = rel
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
                        .collect::<Vec<_>>()
                        .join("/");
                    files.push(AlFile {
                        sort_key,
                        abs_path: path,
                    });
                }
            }
        }
    }
    // Deterministic order — identity doesn't depend on file order, but
    // determinism is the contract.
    files.sort_by(|a, b| a.sort_key.cmp(&b.sort_key));
    Ok(files)
}

/// Read a file as UTF-8, stripping a leading UTF-8 BOM if present (matches TS).
fn read_al_source(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

/// Read the workspace ROOT's `app.json` and return its `id` field VERBATIM (no
/// case change) when present as a non-empty string. al-sem uses this as the
/// StableObjectId prefix (appGuid).
///
/// ORACLE BEHAVIOR (providers/workspace.ts:62-77): the appGuid is read ONLY from
/// `<root>/app.json`, and only when `id` is a non-empty string. Missing file,
/// unparseable JSON, or a missing/empty/non-string `id` → `None`. The fail-closed
/// guard in `snapshot_workspace` turns a `None` here into an EMPTY snapshot.
fn read_root_app_guid(workspace: &Path) -> Option<String> {
    let app_json_path = workspace.join("app.json");
    let text = std::fs::read_to_string(&app_json_path).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&text).ok()?;
    let id = value.get("id")?.as_str()?;
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

/// Count `app.json` files anywhere under `workspace`, EXCLUDING dirs named
/// `node_modules` and `.alpackages` (case-insensitive). Mirrors al-sem's
/// `WorkspaceProvider.collect` layout check: `walk` skips `SKIP_DIR_EXACT`
/// (`node_modules`, `.alpackages`) and counts every remaining file whose basename
/// lowercases to `app.json`. More than one ⇒ multi-app source tree (unsound).
fn count_app_json(workspace: &Path) -> usize {
    let mut count = 0usize;
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
                // Mirror al-sem SKIP_DIR_EXACT (node_modules / .alpackages).
                if dname_lc == "node_modules" || dname_lc == ".alpackages" {
                    continue;
                }
                stack.push(entry.path());
            } else if ftype.is_file()
                && entry.file_name().to_string_lossy().to_lowercase() == "app.json"
            {
                count += 1;
            }
        }
    }
    count
}

/// Build the identity-subset snapshot for a workspace directory.
///
/// Errors (non-throwing-contract note): this is a CLI helper, so a missing /
/// unreadable workspace or app.json surfaces as an `Err` for a clean non-zero
/// exit. The al-sem engine itself never throws; that contract lives in the
/// pipeline, not in this thin extraction CLI.
pub fn snapshot_workspace(workspace: &Path) -> anyhow::Result<IdentitySnapshot> {
    if !workspace.is_dir() {
        anyhow::bail!("workspace is not a directory: {}", workspace.display());
    }

    // --- fail-closed layout detection (identity soundness, al-sem GAP 2) -------
    // A sound workspace is exactly ONE AL app: a readable root app.json with a
    // non-empty string `id`, plus deps under skipped dirs. Anything else mints
    // colliding object identities (one appGuid stamped onto files belonging to
    // distinct apps), so al-sem's WorkspaceProvider.collect emits NO source units.
    // We mirror that: an unsound layout yields an EMPTY identity snapshot.
    //   (a) no readable <root>/app.json with a string `id`, OR
    //   (b) MORE THAN ONE app.json under <root> (excl. node_modules/.alpackages).
    let app_guid = match read_root_app_guid(workspace) {
        Some(g) => g,
        None => {
            eprintln!(
                "fail-closed: no readable root app.json with a string `id` at {} — emitting empty snapshot",
                workspace.display()
            );
            return Ok(IdentitySnapshot {
                objects: Vec::new(),
                routines: Vec::new(),
            });
        }
    };
    let app_json_count = count_app_json(workspace);
    if app_json_count > 1 {
        eprintln!(
            "fail-closed: multi-app source workspace at {} ({app_json_count} app.json files, excl. node_modules/.alpackages) — emitting empty snapshot",
            workspace.display()
        );
        return Ok(IdentitySnapshot {
            objects: Vec::new(),
            routines: Vec::new(),
        });
    }

    let files = discover_al_files(workspace)
        .map_err(|e| anyhow::anyhow!("failed to discover .al files: {e}"))?;

    let mut parser = Parser::new();
    parser
        .set_language(&crate::language::language())
        .map_err(|e| anyhow::anyhow!("failed to set tree-sitter language: {e}"))?;

    let mut snapshot = IdentitySnapshot {
        objects: Vec::new(),
        routines: Vec::new(),
    };

    for file in &files {
        let source = match read_al_source(&file.abs_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("warning: skipping {} (read error: {e})", file.sort_key);
                continue;
            }
        };
        let Some(tree) = parser.parse(&source, None) else {
            eprintln!(
                "warning: skipping {} (parse returned no tree)",
                file.sort_key
            );
            continue;
        };
        extract_from_tree(tree.root_node(), &source, &app_guid, &mut snapshot);
    }

    // `objects` sorted by stableObjectId; `routines` by stableRoutineId. Plain
    // lexicographic (byte) sort — for these ASCII strings it matches the JS
    // default sort used to produce the goldens.
    snapshot
        .objects
        .sort_by(|a, b| a.stable_object_id.cmp(&b.stable_object_id));
    snapshot
        .routines
        .sort_by(|a, b| a.stable_routine_id.cmp(&b.stable_routine_id));

    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_quotes_matches_al_sem_semantics() {
        assert_eq!(strip_quotes("\"D2 Publisher\""), "D2 Publisher");
        assert_eq!(strip_quotes("Customer"), "Customer");
        assert_eq!(strip_quotes("\""), "\""); // single char — not stripped
        assert_eq!(strip_quotes(""), "");
        assert_eq!(strip_quotes("\"unbalanced"), "\"unbalanced");
        assert_eq!(strip_quotes("unbalanced\""), "unbalanced\"");
    }

    /// Root app.json identity: read the `id` only when it is a non-empty string;
    /// any other state (missing file, unparseable, missing/empty/non-string `id`)
    /// yields `None`, which the fail-closed guard turns into an empty snapshot.
    /// Mirrors providers/workspace.ts GAP 2.
    #[test]
    fn read_root_app_guid_requires_nonempty_string_id() {
        let tmp = std::env::temp_dir().join(format!("alch-r0-noappjson-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        // No root app.json → None.
        assert_eq!(read_root_app_guid(&tmp), None);
        // Unparseable app.json → None.
        std::fs::write(tmp.join("app.json"), "{ not json").unwrap();
        assert_eq!(read_root_app_guid(&tmp), None);
        // Valid JSON but no string `id` → None.
        std::fs::write(tmp.join("app.json"), "{\"name\":\"x\"}").unwrap();
        assert_eq!(read_root_app_guid(&tmp), None);
        // Empty `id` string → None.
        std::fs::write(tmp.join("app.json"), "{\"id\":\"\"}").unwrap();
        assert_eq!(read_root_app_guid(&tmp), None);
        // Valid non-empty `id` → verbatim.
        std::fs::write(tmp.join("app.json"), "{\"id\":\"ABC-123\"}").unwrap();
        assert_eq!(read_root_app_guid(&tmp), Some("ABC-123".to_string()));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Layout count: `count_app_json` counts app.json anywhere under the root but
    /// skips `node_modules` and `.alpackages` (mirrors al-sem SKIP_DIR_EXACT). A
    /// two-app tree (`a/app.json` + `b/app.json`) counts 2 ⇒ multi-app (unsound).
    #[test]
    fn count_app_json_skips_node_modules_and_alpackages() {
        let tmp = std::env::temp_dir().join(format!("alch-r0-countapp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("a")).unwrap();
        std::fs::create_dir_all(tmp.join("b")).unwrap();
        std::fs::create_dir_all(tmp.join(".alpackages")).unwrap();
        std::fs::create_dir_all(tmp.join("node_modules").join("pkg")).unwrap();
        std::fs::write(tmp.join("a").join("app.json"), "{\"id\":\"a\"}").unwrap();
        std::fs::write(tmp.join("b").join("app.json"), "{\"id\":\"b\"}").unwrap();
        // These two must NOT be counted (under skipped dirs).
        std::fs::write(tmp.join(".alpackages").join("app.json"), "{}").unwrap();
        std::fs::write(tmp.join("node_modules").join("pkg").join("app.json"), "{}").unwrap();
        assert_eq!(count_app_json(&tmp), 2);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
