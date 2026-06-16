//! Routine scope extraction — parameters, record variables, scalar variables,
//! type index, and routine-id computation.
//!
//! Ports: `intraprocedural-refs.ts` (extractParameters / extractRecordVariables),
//! `variable-indexer.ts` (extractVariables / extractObjectGlobals),
//! `variable-type-normalizer.ts`, `variable-initializer-extractor.ts`,
//! `receiver-classification.ts` (buildVariableTypeIndex), and the routine-id
//! parts of `routine-indexer.ts`.

use super::features::{PTempState, PVariableSymbol};
use super::node_util::{child_of_kind, named_children, node_text, strip_quotes};
use crate::engine::ids::{encode_routine_id, CanonicalRoutineKey, ParamSpec};
use std::collections::HashMap;
use tree_sitter::Node;

/// Every `name`-field child of a `variable_declaration` — ONE for a regular decl,
/// MANY for a grouped multi-name decl (`A, B, C : Type;`, grammar
/// `variable_declaration` multi-name arm). `child_by_field_name` returns only the
/// first, silently dropping `B`/`C`; this returns them all so each declared name
/// becomes its own variable symbol. Falls back to the first named child when the
/// `name` field is absent (defensive; the regular/multi-name arms always set it).
fn decl_name_nodes(var_decl: Node) -> Vec<Node> {
    let mut cursor = var_decl.walk();
    let names: Vec<Node> = var_decl
        .children_by_field_name("name", &mut cursor)
        .collect();
    if names.is_empty() {
        var_decl.named_child(0).into_iter().collect()
    } else {
        names
    }
}

/// The declared name of a `name`-field node, lowercased, with surrounding quotes
/// stripped for a `quoted_identifier`. This matches `simple_receiver_name` (which
/// returns the INNER, unquoted name) and the record-variable extraction, so a quoted
/// scalar variable `"File Blob"` is keyed by `file blob` and a member call
/// `"File Blob".M()` finds it — otherwise the receiver lookup misses and the call
/// degrades to `Unknown{UntrackedReceiver}`.
fn decl_name_lc(name_node: Node, source: &str) -> String {
    if name_node.kind() == "quoted_identifier" {
        strip_quotes(node_text(name_node, source)).to_lowercase()
    } else {
        node_text(name_node, source).to_lowercase()
    }
}

/// A procedure's NAMED return value (`procedure F() Name: Type`) as
/// `(name, name_node, return_type type_specification node)`. AL exposes the named
/// return value as a usable local variable inside the body — e.g.
/// `SendCode.Insert()` where `SendCode` is the named return of
/// `procedure CreateDefaulteDocsSendCode() SendCode: Record "CDO Send Code"`.
/// Grammar: `_procedure_named_return` sets `field('return_value')` and inlines
/// `_procedure_return_specification` which sets `field('return_type')`, so both are
/// direct fields of the routine node; a plain (unnamed) return has `return_type`
/// but no `return_value`. `None` for triggers, unnamed returns, and no-return
/// routines.
fn named_return_value<'a>(
    routine_node: Node<'a>,
    source: &str,
) -> Option<(String, Node<'a>, Node<'a>)> {
    let name_node = routine_node.child_by_field_name("return_value")?;
    let type_node = routine_node.child_by_field_name("return_type")?;
    let name = if name_node.kind() == "quoted_identifier" {
        strip_quotes(node_text(name_node, source)).to_string()
    } else {
        node_text(name_node, source).to_string()
    };
    if name.is_empty() {
        return None;
    }
    Some((name, name_node, type_node))
}

#[derive(Clone)]
pub struct ParameterSymbol {
    pub index: u32,
    pub name: String,
    pub type_text: String,
    pub is_var: bool,
    pub is_record: bool,
    pub table_name: Option<String>,
}

#[derive(Clone)]
pub struct RecordVariable {
    pub id: String,
    pub name: String,
    pub table_name: Option<String>,
    pub temp_state: PTempState,
    pub is_parameter: bool,
    pub parameter_index: Option<u32>,
}

/// `{ kind: "known", value }` — the single shared PTempState "known" constructor.
/// `pub(crate)` so the L3 record-type override (`record_types.rs`) and the ABI→L3
/// projection (`deps/cross_app_l3.rs`) reuse ONE definition (compiler-enforced on
/// any future `PTempState` shape change), instead of duplicating the literal.
pub(crate) fn ts_known(value: bool) -> PTempState {
    PTempState {
        kind: "known".to_string(),
        value: Some(value),
        parameter_index: None,
    }
}

/// `{ kind: "parameter-dependent", parameterIndex }` — the single shared PD
/// constructor. `pub(crate)` for the same reuse reason as [`ts_known`].
pub(crate) fn ts_param_dependent(index: u32) -> PTempState {
    PTempState {
        kind: "parameter-dependent".to_string(),
        value: None,
        parameter_index: Some(index),
    }
}

/// `extractParameters` — ParameterSymbol[] from a routine's parameter_list.
pub fn extract_parameters(proc_node: Node, source: &str) -> Vec<ParameterSymbol> {
    let mut parameters = Vec::new();
    let Some(param_list) = child_of_kind(proc_node, "parameter_list") else {
        return parameters;
    };
    let mut p_index: u32 = 0;
    for param in named_children(param_list) {
        if param.kind() != "parameter" {
            continue;
        }
        let is_var = named_children(param)
            .iter()
            .any(|c| c.kind() == "var_keyword");
        let name_node = named_children(param)
            .into_iter()
            .find(|c| c.kind() == "identifier" || c.kind() == "quoted_identifier");
        let type_spec_node = child_of_kind(param, "type_specification");
        let Some(name_node) = name_node else {
            p_index += 1;
            continue;
        };
        let name = if name_node.kind() == "quoted_identifier" {
            strip_quotes(node_text(name_node, source)).to_string()
        } else {
            node_text(name_node, source).to_string()
        };
        let type_text = type_spec_node
            .map(|t| node_text(t, source).to_string())
            .unwrap_or_default();
        let record_type_node = type_spec_node.and_then(|t| child_of_kind(t, "record_type"));
        let is_record = record_type_node.is_some();
        let mut table_name = None;
        if let Some(rt) = record_type_node {
            let quoted = child_of_kind(rt, "quoted_identifier");
            if let Some(q) = quoted {
                table_name = Some(strip_quotes(node_text(q, source)).to_string());
            } else if let Some(id) = child_of_kind(rt, "identifier") {
                table_name = Some(node_text(id, source).to_string());
            }
        }
        parameters.push(ParameterSymbol {
            index: p_index,
            name,
            type_text,
            is_var,
            is_record,
            table_name,
        });
        p_index += 1;
    }
    parameters
}

/// `extractRecordVariables` — record-typed params + local record decls.
pub fn extract_record_variables(
    proc_node: Node,
    routine_id: &str,
    parameters: &[ParameterSymbol],
    source: &str,
) -> Vec<RecordVariable> {
    let mut out = Vec::new();
    let by_index: HashMap<u32, &ParameterSymbol> =
        parameters.iter().map(|p| (p.index, p)).collect();

    // Record-typed parameters.
    if let Some(param_list) = child_of_kind(proc_node, "parameter_list") {
        let mut p_index: i64 = -1;
        for param in named_children(param_list) {
            if param.kind() != "parameter" {
                continue;
            }
            p_index += 1;
            let Some(meta) = by_index.get(&(p_index as u32)) else {
                continue;
            };
            if !meta.is_record {
                continue;
            }
            let type_spec_node = child_of_kind(param, "type_specification");
            let record_type_node = type_spec_node.and_then(|t| child_of_kind(t, "record_type"));
            let Some(record_type_node) = record_type_node else {
                continue;
            };
            let is_temporary_param = named_children(record_type_node)
                .iter()
                .any(|c| c.kind() == "temporary_keyword");
            let temp_state = if is_temporary_param {
                ts_known(true)
            } else if meta.is_var {
                ts_param_dependent(meta.index)
            } else {
                ts_known(false)
            };
            out.push(RecordVariable {
                id: format!("{}/rv/{}", routine_id, meta.name.to_lowercase()),
                name: meta.name.clone(),
                table_name: meta.table_name.clone(),
                temp_state,
                is_parameter: true,
                parameter_index: Some(meta.index),
            });
        }
    }

    // Local variable declarations.
    if let Some(var_section) = child_of_kind(proc_node, "var_section") {
        for var_decl in named_children(var_section) {
            if var_decl.kind() != "variable_declaration" {
                continue;
            }
            let Some(type_spec_node) = child_of_kind(var_decl, "type_specification") else {
                continue;
            };
            let Some(record_type_node) = child_of_kind(type_spec_node, "record_type") else {
                continue;
            };
            let mut table_name = None;
            if let Some(q) = child_of_kind(record_type_node, "quoted_identifier") {
                table_name = Some(strip_quotes(node_text(q, source)).to_string());
            } else if let Some(id) = child_of_kind(record_type_node, "identifier") {
                table_name = Some(node_text(id, source).to_string());
            }
            let is_temporary = named_children(record_type_node)
                .iter()
                .any(|c| c.kind() == "temporary_keyword");
            // One record variable per declared name (grouped `A, B, C : Record "X"`).
            for name_node in decl_name_nodes(var_decl) {
                let name = if name_node.kind() == "quoted_identifier" {
                    strip_quotes(node_text(name_node, source)).to_string()
                } else {
                    node_text(name_node, source).to_string()
                };
                out.push(RecordVariable {
                    id: format!("{}/rv/{}", routine_id, name.to_lowercase()),
                    name,
                    table_name: table_name.clone(),
                    temp_state: ts_known(is_temporary),
                    is_parameter: false,
                    parameter_index: None,
                });
            }
        }
    }

    // Named return value typed as a Record — a usable record var in the body
    // (`procedure X() SendCode: Record "CDO Send Code"` → `SendCode.Insert()`).
    if let Some((name, _name_node, type_node)) = named_return_value(proc_node, source) {
        if let Some(record_type_node) = child_of_kind(type_node, "record_type") {
            let lc = name.to_lowercase();
            if !out.iter().any(|rv| rv.name.to_lowercase() == lc) {
                let table_name =
                    if let Some(q) = child_of_kind(record_type_node, "quoted_identifier") {
                        Some(strip_quotes(node_text(q, source)).to_string())
                    } else {
                        child_of_kind(record_type_node, "identifier")
                            .map(|id| node_text(id, source).to_string())
                    };
                let is_temporary = named_children(record_type_node)
                    .iter()
                    .any(|c| c.kind() == "temporary_keyword");
                out.push(RecordVariable {
                    id: format!("{}/rv/{}", routine_id, lc),
                    name,
                    table_name,
                    temp_state: ts_known(is_temporary),
                    is_parameter: false,
                    parameter_index: None,
                });
            }
        }
    }

    out
}

/// Collapse internal whitespace runs while preserving quoted AL identifiers.
pub fn canonicalize_type_text(raw: &str) -> String {
    let mut out = String::new();
    let mut in_quotes = false;
    let mut last_was_space = false;
    for ch in raw.trim().chars() {
        if ch == '"' {
            in_quotes = !in_quotes;
            out.push(ch);
            last_was_space = false;
            continue;
        }
        if in_quotes {
            out.push(ch);
            continue;
        }
        if ch == ' ' || ch == '\t' || ch == '\n' || ch == '\r' {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
            continue;
        }
        out.push(ch);
        last_was_space = false;
    }
    out
}

/// `normalizeDeclaredType` — type_specification text (field "type") canonicalized.
fn normalize_declared_type(var_decl: Node, source: &str) -> String {
    let Some(type_node) = var_decl.child_by_field_name("type") else {
        return "unknown".to_string();
    };
    let raw = node_text(type_node, source);
    if raw.is_empty() {
        return "unknown".to_string();
    }
    canonicalize_type_text(raw)
}

/// Capture object-global record variables with their `temporary_keyword` flag.
///
/// Returns one [`crate::engine::l2::features::PRecordVariable`] for every
/// `variable_declaration` inside an object-level `var_section` whose
/// `type_specification` contains a `record_type` child.  Non-record declarations
/// (Integer, Text, …) are silently skipped.
///
/// The `temporary_keyword` child presence on the `record_type` node is the ONLY
/// allowed temp signal — string-sniffing "temporary" from raw type text is
/// intentionally avoided because table names such as `Record "My temporary stuff"`
/// would produce false positives.
///
/// **RV-8 conservative gaps** — the following sections are NOT walked and fall
/// through to `Unknown` (fires a miss) rather than producing incorrect results:
///
/// - `preproc_conditional_var_block` — object-level var sections inside `#if`
///   preprocessing directives.  Their content is conditional on the build
///   environment and cannot be reliably evaluated at analysis time.
/// - Dataitem-scoped var sections in Report and Query objects.  These sit at a
///   deeper nesting level than a top-level `var_section` child and are therefore
///   not reached by this function's single-level child scan.
///
/// Task 3 will re-key global record variables per-routine when promoting them
/// into the routine's `record_variables` vector.
pub fn extract_object_global_record_vars(
    object_node: Node,
    object_id: &str,
    source: &str,
) -> Vec<super::features::PRecordVariable> {
    let mut out = Vec::new();
    for child in named_children(object_node) {
        if child.kind() != "var_section" {
            continue;
        }
        for var_decl in named_children(child) {
            if var_decl.kind() != "variable_declaration" {
                continue;
            }
            let Some(type_spec_node) = child_of_kind(var_decl, "type_specification") else {
                continue;
            };
            let Some(record_type_node) = child_of_kind(type_spec_node, "record_type") else {
                continue; // Not a record — skip.
            };
            let mut table_name = None;
            if let Some(q) = child_of_kind(record_type_node, "quoted_identifier") {
                table_name = Some(strip_quotes(node_text(q, source)).to_string());
            } else if let Some(id) = child_of_kind(record_type_node, "identifier") {
                table_name = Some(node_text(id, source).to_string());
            }
            let is_temporary = named_children(record_type_node)
                .iter()
                .any(|c| c.kind() == "temporary_keyword");
            // One global record var per declared name (grouped `A, B : Record "X"`).
            for name_node in decl_name_nodes(var_decl) {
                let name = if name_node.kind() == "quoted_identifier" {
                    strip_quotes(node_text(name_node, source)).to_string()
                } else {
                    node_text(name_node, source).to_string()
                };
                out.push(super::features::PRecordVariable {
                    id: format!("{}/grv/{}", object_id, name.to_lowercase()),
                    name,
                    table_name: table_name.clone(),
                    temp_state: ts_known(is_temporary),
                    is_parameter: false,
                    parameter_index: None,
                    scope: Some("global".to_string()),
                });
            }
        }
    }
    out
}

/// `extractObjectGlobals` — object-level var_section declarations (scope global).
pub fn extract_object_globals(
    object_node: Node,
    source_unit_id: &str,
    source: &str,
) -> Vec<PVariableSymbol> {
    let mut out = Vec::new();
    for child in named_children(object_node) {
        if child.kind() != "var_section" {
            continue;
        }
        for decl in named_children(child) {
            if decl.kind() != "variable_declaration" {
                continue;
            }
            let declared_type = normalize_declared_type(decl, source);
            // One global per declared name (grouped `A, B, C : Type;`).
            for name_node in decl_name_nodes(decl) {
                let lc_name = decl_name_lc(name_node, source);
                out.push(PVariableSymbol {
                    name: lc_name,
                    declared_type: declared_type.clone(),
                    scope: "global".to_string(),
                    is_parameter: false,
                    parameter_index: None,
                    initializer: None,
                    source_anchor: anchor_from_decl(decl, source_unit_id),
                });
            }
        }
    }
    out
}

fn anchor_from_decl(decl: Node, source_unit_id: &str) -> super::features::PAnchor {
    // global anchors keep BYTE columns? No — globals are anchored at object scope,
    // but the projection still UTF-16-normalizes. We rebuild this in mod.rs where
    // the Utf16Cols is available; here we store byte cols as a placeholder that
    // mod.rs overwrites is overkill — instead globals are rarely emitted (shadowed
    // out in single-routine vectors). Use UTF-8 byte cols converted at call site.
    let sp = decl.start_position();
    let ep = decl.end_position();
    super::features::PAnchor {
        source_unit_id: source_unit_id.to_string(),
        start_line: sp.row as u32,
        start_column: sp.column as u32,
        end_line: ep.row as u32,
        end_column: ep.column as u32,
        syntax_kind: "variable_declaration".to_string(),
    }
}

/// `extractVariables` — params → locals (with initializer) → globals (shadowing).
#[allow(clippy::too_many_arguments)]
pub fn extract_variables(
    routine_node: Node,
    source_unit_id: &str,
    parameters: &[ParameterSymbol],
    globals: &[PVariableSymbol],
    source: &str,
    cols: &super::node_util::Utf16Cols,
) -> Vec<PVariableSymbol> {
    let mut out: Vec<PVariableSymbol> = Vec::new();

    // 1. Parameters.
    for p in parameters {
        let lc_name = p.name.to_lowercase();
        out.push(PVariableSymbol {
            name: lc_name,
            declared_type: canonicalize_type_text(&p.type_text),
            scope: "parameter".to_string(),
            is_parameter: true,
            parameter_index: Some(p.index),
            initializer: None,
            // synthetic param anchor: all zeros.
            source_anchor: super::features::PAnchor {
                source_unit_id: source_unit_id.to_string(),
                start_line: 0,
                start_column: 0,
                end_line: 0,
                end_column: 0,
                syntax_kind: "parameter".to_string(),
            },
        });
    }

    // 2. Locals.
    let body_node = find_code_block(routine_node);
    for decl in find_variable_declarations(routine_node) {
        let declared_type = normalize_declared_type(decl, source);
        let sp = decl.start_position();
        let ep = decl.end_position();
        // One local per declared name (grouped `A, B, C : Type;`).
        for name_node in decl_name_nodes(decl) {
            let lc_name = decl_name_lc(name_node, source);
            if out.iter().any(|v| v.is_parameter && v.name == lc_name) {
                continue;
            }
            let initializer = body_node.and_then(|b| extract_initializer(b, &lc_name, source));
            out.push(PVariableSymbol {
                name: lc_name,
                declared_type: declared_type.clone(),
                scope: "local".to_string(),
                is_parameter: false,
                parameter_index: None,
                initializer,
                source_anchor: super::features::PAnchor {
                    source_unit_id: source_unit_id.to_string(),
                    start_line: sp.row as u32,
                    start_column: cols.col(sp.row, sp.column),
                    end_line: ep.row as u32,
                    end_column: cols.col(ep.row, ep.column),
                    syntax_kind: "variable_declaration".to_string(),
                },
            });
        }
    }

    // 2b. Named return value — a usable local variable in the body, of ANY type
    // (`procedure X() Result: Codeunit Y` → `Result.Method()`, record returns →
    // their intrinsics). Mirrors a local declaration; skipped if a param/local of the
    // same name already shadows it.
    if let Some((name, name_node, type_node)) = named_return_value(routine_node, source) {
        let lc_name = name.to_lowercase();
        if !out.iter().any(|v| v.name == lc_name) {
            let sp = name_node.start_position();
            let ep = type_node.end_position();
            out.push(PVariableSymbol {
                name: lc_name,
                declared_type: canonicalize_type_text(node_text(type_node, source)),
                scope: "local".to_string(),
                is_parameter: false,
                parameter_index: None,
                initializer: None,
                source_anchor: super::features::PAnchor {
                    source_unit_id: source_unit_id.to_string(),
                    start_line: sp.row as u32,
                    start_column: cols.col(sp.row, sp.column),
                    end_line: ep.row as u32,
                    end_column: cols.col(ep.row, ep.column),
                    syntax_kind: "return_value".to_string(),
                },
            });
        }
    }

    // 3. Object globals (first-match-wins shadowing). Convert their byte cols to UTF-16.
    if !globals.is_empty() {
        let mut emitted: std::collections::HashSet<String> =
            out.iter().map(|v| v.name.clone()).collect();
        for g in globals {
            if emitted.contains(&g.name) {
                continue;
            }
            let mut g = g.clone();
            // Re-normalize the global's anchor columns to UTF-16 (it was stored as bytes).
            g.source_anchor.start_column = cols.col(
                g.source_anchor.start_line as usize,
                g.source_anchor.start_column as usize,
            );
            g.source_anchor.end_column = cols.col(
                g.source_anchor.end_line as usize,
                g.source_anchor.end_column as usize,
            );
            emitted.insert(g.name.clone());
            out.push(g);
        }
    }

    out
}

fn find_code_block(routine_node: Node) -> Option<Node> {
    named_children(routine_node)
        .into_iter()
        .find(|c| c.kind() == "code_block")
}

fn find_variable_declarations(routine_node: Node) -> Vec<Node> {
    let mut out = Vec::new();
    fn visit<'a>(n: Node<'a>, out: &mut Vec<Node<'a>>) {
        if n.kind() == "variable_declaration" {
            out.push(n);
            return;
        }
        if n.kind() == "code_block" {
            return;
        }
        for c in named_children(n) {
            visit(c, out);
        }
    }
    visit(routine_node, &mut out);
    out
}

/// `buildVariableTypeIndex` — lc name (quote-stripped) → declaredType (first wins).
pub fn build_variable_type_index(variables: &[PVariableSymbol]) -> HashMap<String, String> {
    let mut by_name = HashMap::new();
    for v in variables {
        let key = strip_quotes(&v.name).to_string();
        by_name
            .entry(key)
            .or_insert_with(|| v.declared_type.clone());
    }
    by_name
}

// --- Initializer extraction (variable-initializer-extractor.ts) ---

fn strip_single_quotes(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 2 && t.starts_with('\'') && t.ends_with('\'') {
        t[1..t.len() - 1].to_string()
    } else {
        t.to_string()
    }
}

fn strip_double_quotes(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        t[1..t.len() - 1].to_string()
    } else {
        t.to_string()
    }
}

fn extract_initializer(
    body_node: Node,
    var_name_lc: &str,
    source: &str,
) -> Option<serde_json::Value> {
    let assignment = find_first_assignment_to(body_node, var_name_lc, source)?;
    let rhs = assignment
        .child_by_field_name("right")
        .or_else(|| assignment.child_by_field_name("value"))
        .or_else(|| {
            let c = assignment.named_child_count();
            if c > 0 {
                assignment.named_child(c as u32 - 1)
            } else {
                None
            }
        })?;
    Some(classify_rhs(rhs, source))
}

fn find_first_assignment_to<'a>(
    node: Node<'a>,
    var_name_lc: &str,
    source: &str,
) -> Option<Node<'a>> {
    if node.kind() == "assignment_statement" {
        let target = node
            .child_by_field_name("left")
            .or_else(|| node.child_by_field_name("target"))
            .or_else(|| node.named_child(0));
        if let Some(target) = target {
            if node_text(target, source).to_lowercase() == var_name_lc {
                return Some(node);
            }
        }
    }
    for c in named_children(node) {
        if let Some(found) = find_first_assignment_to(c, var_name_lc, source) {
            return Some(found);
        }
    }
    None
}

fn classify_rhs(rhs: Node, source: &str) -> serde_json::Value {
    use serde_json::json;
    let t = rhs.kind();
    let text = node_text(rhs, source);
    if t == "string_literal" || t == "string_literal_value" || t == "text_literal" {
        return json!({ "kind": "literal", "value": strip_single_quotes(text) });
    }
    if t == "integer"
        || t == "integer_literal"
        || t == "decimal_literal"
        || t == "number_literal"
        || t == "decimal"
    {
        return json!({ "kind": "literal", "value": text.trim() });
    }
    if t == "boolean_literal" || t == "true" || t == "false" {
        return json!({ "kind": "literal", "value": text.trim() });
    }
    if t == "qualified_enum_value" {
        let enum_name_node = rhs.named_child(0);
        let count = rhs.named_child_count();
        let member_node = if count > 0 {
            rhs.named_child(count as u32 - 1)
        } else {
            None
        };
        if let (Some(en), Some(mn)) = (enum_name_node, member_node) {
            let enum_name = strip_double_quotes(node_text(en, source));
            let member = node_text(mn, source).to_string();
            return json!({ "kind": "enum", "enumName": enum_name, "member": member });
        }
        return json!({ "kind": "expression" });
    }
    if t == "identifier" {
        return json!({
            "kind": "constant-var",
            "varName": text.to_lowercase(),
            "initializer": { "kind": "unknown" }
        });
    }
    json!({ "kind": "expression" })
}

// --- Routine id computation (routine-indexer.ts parts) ---

/// Map an object-decl node type to al-sem's display object-type string.
pub fn object_type_for(node_type: &str) -> Option<&'static str> {
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

/// Compute the internal RoutineId (`{modelInstanceId}/{canonicalKeyHash}`).
#[allow(clippy::too_many_arguments)]
pub fn compute_routine_id(
    app_guid: &str,
    object_type: &str,
    object_number: i64,
    routine_kind: &str,
    routine_name: &str,
    parameters: &[ParameterSymbol],
    return_type_text: Option<&str>,
    model_instance_id: &str,
) -> String {
    let param_specs: Vec<ParamSpec> = parameters
        .iter()
        .map(|p| ParamSpec {
            type_text: p.type_text.clone(),
            is_var: p.is_var,
        })
        .collect();
    let normalized_signature_hash =
        crate::engine::ids::normalized_signature_hash(routine_name, &param_specs, return_type_text);
    let key = CanonicalRoutineKey {
        app_guid: app_guid.to_string(),
        object_type: object_type.to_string(),
        object_number,
        routine_kind: routine_kind.to_string(),
        routine_name: routine_name.to_string(),
        normalized_signature_hash,
    };
    encode_routine_id(&key, model_instance_id)
}
