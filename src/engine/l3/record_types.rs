//! L3 record-type resolution — Rust port of al-sem's
//! `src/resolve/record-types.ts` (`resolveRecordTypes` + `recordTableNameOf`).
//!
//! Three passes over each routine's record vars + ops, plus implicit `Rec`/`xRec`
//! resolution via the object's "effective own table":
//!   1. declared record vars → resolve `tableName` against the symbol table.
//!   2. record ops → derive `tableId` from the matching record var; then the
//!      lexical-scope fallback over `features.variables` (FIRST-wins / innermost
//!      declaration wins on a name collision; params → locals → globals order
//!      means locals always shadow globals) for still-unset ops.
//!   3. implicit `Rec`/`xRec` ops (still unset) → the effective-own-table for
//!      Table / Page / TableExtension / PageExtension, NEVER overriding an
//!      explicit local `Rec` that pass (1)/(2) already resolved.
//!
//! Quoted-identifier parity (MUST match TS EXACTLY, do NOT "fix"):
//!   - `record_table_name_of` strips surrounding quotes from `declaredType`.
//!   - `extendsTargetName` / `sourceTableName` are stored already-stripped at
//!     index time (al-sem `extractExtendsTargetName` / SourceTable unquote), then
//!     passed RAW to `table_by_name` / `object_by_type_name` (which lower-case).

use super::l3_workspace::{L3Object, L3Routine};
use super::symbol_table::SymbolTable;
use crate::engine::l2::node_util::strip_quotes;

/// Extract the TABLE NAME from a record variable's `declaredType` string.
///
///   `Record "Sales Line"`   → `Sales Line`
///   `Record Customer`       → `Customer`
///   `Record "X" temporary`  → `X`
///
/// Returns `None` for non-record types (e.g. `Integer`, `Codeunit "Foo"`).
/// Strips a trailing ` temporary` modifier, then unquotes the remaining name.
pub fn record_table_name_of(declared_type: &str) -> Option<String> {
    let trimmed = declared_type.trim();
    // Match a leading `Record` keyword (case-insensitive), word-bounded.
    let lower = trimmed.to_lowercase();
    if !lower.starts_with("record") {
        return None;
    }
    let rest_bytes = &trimmed[6..]; // "record".len() == 6 (ASCII)
                                    // Word-boundary check: the char after `record` must NOT be alphanumeric/_.
    if let Some(c) = rest_bytes.chars().next() {
        if c.is_alphanumeric() || c == '_' {
            return None;
        }
    }
    let mut rest = rest_bytes.trim().to_string();
    // Strip a trailing ` temporary` modifier (case-insensitive, with leading ws).
    rest = strip_trailing_temporary(&rest);
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }
    let name = strip_quotes(rest).trim().to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Strip a trailing `\s+temporary\s*$` (case-insensitive) — mirrors TS
/// `rest.replace(/\s+temporary\s*$/i, "")`.
fn strip_trailing_temporary(s: &str) -> String {
    let trimmed_end = s.trim_end();
    let lower = trimmed_end.to_lowercase();
    if let Some(prefix_len) = lower.strip_suffix("temporary").map(|p| p.len()) {
        // The char before "temporary" must be whitespace for `\s+temporary`.
        let prefix = &trimmed_end[..prefix_len];
        if prefix.ends_with(char::is_whitespace) {
            return prefix.to_string();
        }
    }
    trimmed_end.to_string()
}

/// Run the 3-pass record-type resolution over one routine, mutating each record
/// var / op's resolved `table_id` (internal TableId; the L3 dump projects it as a
/// StableTableId). Mirrors `resolveRecordTypes`'s per-routine body EXACTLY.
pub fn resolve_routine_record_types(
    routine: &mut L3Routine,
    object: Option<&L3Object>,
    symbols: &SymbolTable,
) {
    // --- pass 1: resolve declared record variables ---------------------------
    // name (lowercased) -> index into record_variables, for matching ops below.
    use std::collections::HashMap;
    let mut var_index_by_name: HashMap<String, usize> = HashMap::new();
    for (i, variable) in routine.record_variables.iter_mut().enumerate() {
        // intentionally last-wins — record_variables has no name duplicates at this layer (Task 3 global-promotion must preserve this invariant)
        var_index_by_name.insert(variable.name.to_lowercase(), i);
        if let Some(table_name) = &variable.table_name {
            if let Some(table) = symbols.table_by_name(table_name) {
                variable.table_id = Some(table.id.clone());
            }
        }
    }

    // --- pass 2a: resolve record ops against their declaring record var ------
    for op in routine.record_operations.iter_mut() {
        if let Some(&vi) = var_index_by_name.get(&op.record_variable_name.to_lowercase()) {
            let variable = &routine.record_variables[vi];
            op.record_variable_id = Some(variable.id.clone());
            if let Some(tid) = &variable.table_id {
                op.table_id = Some(tid.clone());
            }
        }
    }

    // --- pass 2b: lexical-scope fallback over features.variables --------------
    // FIRST-wins on a name collision (innermost declaration wins). `routine.variables`
    // is ordered params → locals → globals, so the first entry for a name is the
    // innermost (param/local) scope. Using `.entry().or_insert()` means a later
    // global with the same name is silently ignored rather than clobbering the
    // local — the correct AL lexical-scope rule, and a prerequisite for the
    // tempState backfill (Task 3) that promotes globals into the list.
    // The unset guard below is essential — never override a tableId pass (1)/(2a)
    // already set.
    let mut variable_decl_by_name: HashMap<String, String> = HashMap::new();
    for v in &routine.variables {
        variable_decl_by_name
            .entry(v.name.to_lowercase())
            .or_insert(v.declared_type.clone());
    }
    for op in routine.record_operations.iter_mut() {
        if op.table_id.is_some() {
            continue;
        }
        let Some(decl) = variable_decl_by_name.get(&op.record_variable_name.to_lowercase()) else {
            continue;
        };
        let Some(table_name) = record_table_name_of(decl) else {
            continue;
        };
        if let Some(table) = symbols.table_by_name(&table_name) {
            op.table_id = Some(table.id.clone());
        }
    }

    // --- pass 3: implicit Rec/xRec via the object's effective own table -------
    // NEVER overrides an explicit local `Rec` (the unset guard). The effective
    // own table:
    //   Table          → the table itself (object.name).
    //   Page           → the declared SourceTable (object.source_table_name).
    //   TableExtension → the EXTENDED table (object.extends_target_name).
    //   PageExtension  → the base page's SourceTable (resolve the base page via the
    //                    extends target, then read ITS source_table_name).
    let own_table_id: Option<String> = match object {
        Some(obj) => match obj.object_type.as_str() {
            "Table" => symbols.table_by_name(&obj.name).map(|t| t.id.clone()),
            "Page" => obj
                .source_table_name
                .as_ref()
                .and_then(|st| symbols.table_by_name(st))
                .map(|t| t.id.clone()),
            "TableExtension" => obj
                .extends_target_name
                .as_ref()
                .and_then(|et| symbols.table_by_name(et))
                .map(|t| t.id.clone()),
            "PageExtension" => obj.extends_target_name.as_ref().and_then(|et| {
                let base_page = symbols.object_by_type_name("Page", et)?;
                let st = base_page.source_table_name.as_ref()?;
                symbols.table_by_name(st).map(|t| t.id.clone())
            }),
            _ => None,
        },
        None => None,
    };

    if let Some(own_id) = own_table_id {
        for op in routine.record_operations.iter_mut() {
            if op.table_id.is_some() {
                continue;
            }
            let name = op.record_variable_name.to_lowercase();
            if name != "rec" && name != "xrec" {
                continue;
            }
            op.table_id = Some(own_id.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_table_name_of_basics() {
        assert_eq!(
            record_table_name_of("Record Customer").as_deref(),
            Some("Customer")
        );
        assert_eq!(
            record_table_name_of("Record \"Sales Line\"").as_deref(),
            Some("Sales Line")
        );
        assert_eq!(
            record_table_name_of("Record \"X\" temporary").as_deref(),
            Some("X")
        );
        assert_eq!(
            record_table_name_of("Record Customer temporary").as_deref(),
            Some("Customer")
        );
        assert_eq!(record_table_name_of("Integer"), None);
        assert_eq!(record_table_name_of("Codeunit \"Foo\""), None);
        assert_eq!(record_table_name_of("RecordRef"), None);
        assert_eq!(record_table_name_of("Record"), None);
        // Parity landmine: `Record temporary` has NO whitespace BEFORE
        // "temporary" once `Record` is consumed, so TS's `/\s+temporary\s*$/i`
        // does NOT match — rest stays "temporary" and is returned verbatim (the
        // table name "temporary"). Reproduce, don't "fix".
        assert_eq!(
            record_table_name_of("Record temporary").as_deref(),
            Some("temporary")
        );
        // With a real name + trailing modifier the strip DOES fire.
        assert_eq!(
            record_table_name_of("Record  Item   temporary  ").as_deref(),
            Some("Item")
        );
    }
}
