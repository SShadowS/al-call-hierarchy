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
/// ORACLE QUIRK (intraprocedural-refs.ts:47-54): al-sem locates the parameter
/// name by finding the FIRST `identifier` named child, and if a parameter has
/// NO `identifier` child it is SKIPPED entirely (`continue`). A parameter whose
/// name is a *quoted* identifier (e.g. `"Sales Header": Record "Sales Header"`)
/// therefore has only a `quoted_identifier` name node, no `identifier`, and is
/// dropped from the canonical signature. We reproduce that exactly so the Rust
/// signature matches the oracle's (driven by ws-r0-canon-stress `DoWork`).
fn extract_parameters(node: Node, source: &str) -> Vec<ParamSpec> {
    let mut params = Vec::new();
    let Some(param_list) = named_children(node).find(|c| c.kind() == "parameter_list") else {
        return params;
    };
    for param in named_children(param_list) {
        if param.kind() != "parameter" {
            continue;
        }
        // Mirror al-sem: a parameter with no `identifier` name child is skipped.
        if !named_children(param).any(|c| c.kind() == "identifier") {
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
/// case change). al-sem uses this as the StableObjectId prefix (appGuid).
///
/// ORACLE BEHAVIOR (providers/workspace.ts:50-73): the appGuid is read ONLY from
/// `<root>/app.json`. al-sem does NOT search subdirectories for app.json, and on
/// ANY failure — file missing, unparseable JSON, or a missing/non-string `id`
/// field — it defaults the appGuid to the literal string `"unknown"` (pushing a
/// warning diagnostic, never throwing). Multi-app fixtures that keep their
/// app.json under subdirs (e.g. `a/app.json`, `b/app.json`) therefore resolve to
/// `"unknown"` for every discovered object — `ws-diff-coverage-narrowed` is the
/// canonical example. We reproduce that fallback exactly rather than erroring.
fn read_app_guid(workspace: &Path) -> String {
    let app_json_path = workspace.join("app.json");
    let Ok(text) = std::fs::read_to_string(&app_json_path) else {
        return "unknown".to_string();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return "unknown".to_string();
    };
    value
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| "unknown".to_string())
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
    let app_guid = read_app_guid(workspace);
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

    /// app.json fallback: a workspace with no root `app.json` (e.g. a multi-app
    /// fixture whose app.json files live under subdirs) resolves the appGuid to
    /// the literal `"unknown"`, mirroring providers/workspace.ts. Never errors.
    #[test]
    fn read_app_guid_falls_back_to_unknown_without_root_app_json() {
        let tmp = std::env::temp_dir().join(format!("alch-r0-noappjson-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        assert_eq!(read_app_guid(&tmp), "unknown");
        // Unparseable app.json → also "unknown".
        std::fs::write(tmp.join("app.json"), "{ not json").unwrap();
        assert_eq!(read_app_guid(&tmp), "unknown");
        // Valid JSON but no string `id` → "unknown".
        std::fs::write(tmp.join("app.json"), "{\"name\":\"x\"}").unwrap();
        assert_eq!(read_app_guid(&tmp), "unknown");
        // Valid `id` → verbatim.
        std::fs::write(tmp.join("app.json"), "{\"id\":\"ABC-123\"}").unwrap();
        assert_eq!(read_app_guid(&tmp), "ABC-123");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
