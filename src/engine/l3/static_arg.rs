//! Symbol-table-dependent static-type helpers (R2b Task 3) — faithful ports of
//! al-sem's `staticArgType`, `inferRecordFieldType`, and `inferCallExprReturnType`
//! from `src/resolve/call-resolver.ts`.
//!
//! Unlike the Task-2 scalar primitives (pure string ops), these read the L3
//! symbol table + the resolved L2 features (record variables' tableId, the table
//! field index, same-object overload lists). They feed the overload arg-type
//! disambiguation tiebreak (`disambiguate_by_arg_types`).
//!
//! All functions are read-only: they NEVER mutate argumentBindings or trigger
//! re-entrant call resolution.

use super::l3_workspace::L3Routine;
use super::symbol_table::SymbolTable;
use crate::engine::l2::features::{PCallSite, PExpressionInfo};

/// Read-only: declared return type of a same-object call-expression argument, only
/// when the inner callee resolves to exactly one same-name routine in the caller's
/// object with a defined returnType. Else `None`.
///
/// Faithful port of al-sem `inferCallExprReturnType`.
pub fn infer_call_expr_return_type(
    caller: &L3Routine,
    info: &PExpressionInfo,
    symbols: &SymbolTable,
) -> Option<String> {
    // Bare callee name = text before the first '(' (trimmed), else the whole text.
    let name = match info.text.find('(') {
        Some(idx) => info.text[..idx].trim(),
        None => info.text.trim(),
    };
    if name.is_empty() {
        return None;
    }
    let matches = symbols.routines_in_object_by_name(&caller.object_id, name);
    if matches.len() != 1 {
        return None;
    }
    matches[0].return_type.clone()
}

/// Read-only: dataType of a `Rec.Field` member-expression argument, only when Rec
/// resolves to a record variable with a known tableId, the table is in the index,
/// and EXACTLY one field matches the name (case-insensitive). Else `None`.
///
/// Faithful port of al-sem `inferRecordFieldType`. Resolution priority:
/// `recordVariables` (tableId already resolved by record-types) first, then the
/// `features.variables` + declaredType fallback via `tableByName`.
pub fn infer_record_field_type(
    caller: &L3Routine,
    info: &PExpressionInfo,
    symbols: &SymbolTable,
) -> Option<String> {
    // Extract receiver + field name from raw text like "Rec.Amount" or
    // "Rec.\"My Field\"".
    let dot_idx = info.text.find('.')?;
    let rec_name = info.text[..dot_idx].trim().to_lowercase();
    let mut field_raw = info.text[dot_idx + 1..].trim().to_string();
    // Strip surrounding double-quotes from the field name.
    if field_raw.starts_with('"') && field_raw.ends_with('"') && field_raw.chars().count() > 2 {
        field_raw = field_raw[1..field_raw.len() - 1].to_string();
    }
    let field_name = field_raw.to_lowercase();
    if rec_name.is_empty() || field_name.is_empty() {
        return None;
    }

    // Resolve the table: prefer recordVariables (tableId already resolved).
    let rec_var = caller
        .record_variables
        .iter()
        .find(|v| v.name.to_lowercase() == rec_name);
    let mut table_id: Option<String> = rec_var.and_then(|v| v.table_id.clone());

    // Fallback: resolve via features.variables + tableByName.
    if table_id.is_none() {
        if let Some(v) = caller
            .variables
            .iter()
            .find(|x| x.name.to_lowercase() == rec_name)
        {
            if !v.declared_type.is_empty() {
                if let Some(t_name) = extract_record_table_name(&v.declared_type) {
                    if let Some(table) = symbols.table_by_name(&t_name) {
                        table_id = Some(table.id.clone());
                    }
                }
            }
        }
    }

    let table_id = table_id?;
    let table = symbols.table_by_id(&table_id)?;
    let matches: Vec<&str> = table
        .fields
        .iter()
        .filter(|f| f.name.to_lowercase() == field_name)
        .map(|f| f.data_type.as_str())
        .collect();
    if matches.len() == 1 {
        Some(matches[0].to_string())
    } else {
        None
    }
}

/// Extract a table name from a declaredType like `Record "FD Rec"` or
/// `Record Customer` (optionally `temporary`). Faithful port of al-sem's regex
/// `/^record\b(.*?)(\s+temporary)?\s*$/i` applied to the trimmed declaredType.
fn extract_record_table_name(declared_type: &str) -> Option<String> {
    let trimmed = declared_type.trim();
    let lower = trimmed.to_lowercase();
    // Must start with the word "record" followed by a word boundary.
    if !lower.starts_with("record") {
        return None;
    }
    // `\b` after "record": next char (if any) must not be a word char.
    let after = &trimmed["record".len()..];
    if let Some(c) = after.chars().next() {
        if c.is_alphanumeric() || c == '_' {
            return None; // e.g. "RecordRef" — no word boundary
        }
    }
    // Drop a trailing " temporary" (case-insensitive), then trim.
    let mut name_part = after.trim();
    let name_lower = name_part.to_lowercase();
    // The regex's optional `(\s+temporary)?` is greedy-anchored at end: a trailing
    // whitespace + "temporary" is stripped.
    if let Some(stripped) = name_lower.strip_suffix("temporary") {
        if stripped.len() < name_lower.len() {
            let prefix_len = stripped.len();
            // require at least one whitespace char before "temporary"
            if name_part[..prefix_len].ends_with(char::is_whitespace) {
                name_part = name_part[..prefix_len].trim_end();
            }
        }
    }
    let mut t_name = name_part.trim().to_string();
    if t_name.starts_with('"') && t_name.ends_with('"') && t_name.chars().count() > 2 {
        t_name = t_name[1..t_name.len() - 1].to_string();
    }
    if t_name.is_empty() {
        None
    } else {
        Some(t_name)
    }
}

/// Static type of the i-th call argument when it can be pinned with confidence,
/// else `None`. Faithful port of al-sem `staticArgType`:
///   - variable-backed (parameter/local/global/implicit-rec) via declaredType
///   - call-expression args via the inner callee's declared returnType
///   - member-expression (Rec.Field) args via inferRecordFieldType
///   - qualified_enum_value args → `Enum "<name>"` when the enum is in the index
pub fn static_arg_type(
    caller: &L3Routine,
    call_site: &PCallSite,
    i: usize,
    symbols: &SymbolTable,
) -> Option<String> {
    let binding = call_site.argument_bindings.get(i)?;
    let named = matches!(
        binding.source_kind.as_str(),
        "parameter" | "local" | "global" | "implicit-rec"
    );
    if named {
        if let Some(name) = &binding.source_variable_name {
            if let Some(v) = caller.variables.iter().find(|x| &x.name == name) {
                if !v.declared_type.is_empty() {
                    return Some(v.declared_type.clone());
                }
            }
        }
    }
    let info = call_site.argument_infos.get(i);
    if let Some(info) = info {
        match info.kind.as_str() {
            "call_expression" => {
                if let Some(t) = infer_call_expr_return_type(caller, info, symbols) {
                    return Some(t);
                }
            }
            "member_expression" => {
                if let Some(t) = infer_record_field_type(caller, info, symbols) {
                    return Some(t);
                }
            }
            "qualified_enum_value" => {
                if let Some(qualifier) = &info.qualifier {
                    // al-sem: `qualifier.replace(/^"|"$/g, "")` — strip ONE leading
                    // and ONE trailing double-quote (anchored, not repeated).
                    let enum_name = strip_one_each_quote(qualifier);
                    if !enum_name.is_empty()
                        && symbols.object_by_type_name("Enum", &enum_name).is_some()
                    {
                        return Some(format!("Enum \"{enum_name}\""));
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Strip ONE leading and ONE trailing `"` (anchored), matching JS
/// `s.replace(/^"|"$/g, "")`.
fn strip_one_each_quote(s: &str) -> String {
    let mut out = s;
    if out.starts_with('"') {
        out = &out[1..];
    }
    if out.ends_with('"') {
        out = &out[..out.len() - 1];
    }
    out.to_string()
}
