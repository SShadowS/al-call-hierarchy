//! Implicit-trigger edges (R2b Task 3) — faithful port of al-sem's
//! `buildImplicitTriggerEdges` from `src/resolve/implicit-edges.ts`.
//!
//! For each trigger-invoking record op whose record variable resolves to a table
//! IN indexed source, emit an edge to that table's trigger routine. Tables
//! al-sem cannot see produce no edge (reflected in coverage, not invented here).
//!
//! `Validate` always runs OnValidate → "resolved". `Insert`/`Modify`/`Delete`
//! run the table trigger only with `RunTrigger = true` (not captured) → "maybe".

use super::call_resolver::CallEdge;
use super::l3_workspace::L3Workspace;
use super::symbol_table::SymbolTable;
use super::taxonomy::{DispatchKind, Resolution};

/// (trigger routine name, edge Resolution).
fn trigger_mapping(op: &str) -> Option<(&'static str, Resolution)> {
    match op {
        "Validate" => Some(("OnValidate", Resolution::Resolved)),
        "Insert" => Some(("OnInsert", Resolution::Maybe)),
        "Modify" => Some(("OnModify", Resolution::Maybe)),
        "Delete" => Some(("OnDelete", Resolution::Maybe)),
        _ => None,
    }
}

/// Build implicit-trigger CallEdges over the workspace. The op's operation id
/// doubles as the callsite ref (`callsiteId == operationId == op.id`).
pub fn build_implicit_trigger_edges(
    workspace: &L3Workspace,
    symbols: &SymbolTable,
) -> Vec<CallEdge> {
    let mut edges: Vec<CallEdge> = Vec::new();
    for routine in &workspace.routines {
        for op in &routine.record_operations {
            let Some((trigger_name, resolution)) = trigger_mapping(&op.op) else {
                continue;
            };
            let Some(table_id) = &op.table_id else {
                continue; // table not resolved → cannot find its trigger
            };
            let Some(table) = symbols.table_by_id(table_id) else {
                continue;
            };
            // Tables are objects too — look up by type + number.
            let Some(table_object) = symbols.object_by_type_number("Table", table.table_number)
            else {
                continue;
            };
            let Some(trigger) = symbols.routine_in_object(&table_object.id, trigger_name) else {
                continue;
            };
            edges.push(CallEdge {
                from: routine.id.clone(),
                to: Some(trigger.id.clone()),
                callsite_id: op.id.clone(),
                operation_id: op.id.clone(),
                dispatch_kind: DispatchKind::ImplicitTrigger,
                resolution,
                candidates: None,
                external_type_ref: None,
                receiver_type: None,
                dispatch_meta: None,
                unknown_method_name: None,
            });
        }
    }
    edges
}
