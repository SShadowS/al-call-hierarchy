//! Synthetic-input constructors shared by the L5 native-oracle tests. NOT a
//! golden fixture — these build minimal `L3Routine` / `CombinedGraph` /
//! `CapabilityFact` / `FullRoutineSummary` values directly so each oracle
//! exercises the query functions on hand-built inputs (mirroring al-sem's
//! probe-style soundness oracles, not a byte-diff).
//!
//! `#[cfg(test)]`-only — never compiled into the shipping engine.

#![cfg(test)]

use std::collections::HashMap;

use crate::engine::l2::features::{PAnchor, PCallSite, PCallee, POperationSite};
use crate::engine::l3::l3_workspace::L3Routine;
use crate::engine::l4::capability_cone::{CapabilityFact, CoverageRecord};
use crate::engine::l4::combined_graph::{CombinedEdge, CombinedGraph};
use crate::engine::l5::full_summary::FullRoutineSummary;

/// A throwaway anchor (positions are irrelevant to the L5 query substrate).
fn dummy_anchor() -> PAnchor {
    PAnchor {
        source_unit_id: "ws:test".to_string(),
        start_line: 0,
        start_column: 0,
        end_line: 0,
        end_column: 0,
        syntax_kind: "test".to_string(),
    }
}

/// A resolved combined edge `from → to` with a callsite id.
pub fn edge(from: &str, to: &str, callsite_id: &str) -> CombinedEdge {
    CombinedEdge {
        from: from.to_string(),
        to: to.to_string(),
        kind: "direct".to_string(),
        callsite_id: Some(callsite_id.to_string()),
        operation_id: None,
        event_id: None,
        subscriber_app_id: None,
        resolution: "resolved".to_string(),
    }
}

/// Build a `CombinedGraph` from a node list + flat edge list. `nodes` is sorted;
/// `edges_by_from` is grouped (each per-from list in input order — the L5
/// reverse-graph builder is robust to per-from ordering).
pub fn graph_from_edges(nodes: &[&str], edges: &[CombinedEdge]) -> CombinedGraph {
    let mut node_vec: Vec<String> = nodes.iter().map(|n| n.to_string()).collect();
    node_vec.sort();
    let mut edges_by_from: HashMap<String, Vec<CombinedEdge>> = HashMap::new();
    for e in edges {
        edges_by_from
            .entry(e.from.clone())
            .or_default()
            .push(e.clone());
    }
    CombinedGraph {
        nodes: node_vec,
        edges_by_from,
        uncertainty_edges: Vec::new(),
        typed_edges: Vec::new(),
    }
}

/// A minimal `L3Routine` with the given internal id + kind. All other fields are
/// empty / defaulted — only the L5-substrate-relevant ones (id, kind,
/// operation_sites, call_sites) are exercised by the oracles.
pub fn routine(id: &str, kind: &str) -> L3Routine {
    L3Routine {
        id: id.to_string(),
        stable_routine_id: format!("stable::{id}"),
        object_id: "app/Codeunit/1".to_string(),
        object_type: "Codeunit".to_string(),
        name: id.to_string(),
        kind: kind.to_string(),
        attributes_parsed: Vec::new(),
        app_guid: "app".to_string(),
        object_number: 1,
        normalized_signature_hash: String::new(),
        body_available: true,
        parse_incomplete: false,
        record_variables: Vec::new(),
        record_operations: Vec::new(),
        field_accesses: Vec::new(),
        variables: Vec::new(),
        parameters: Vec::new(),
        access_modifier: None,
        return_type: None,
        call_sites: Vec::new(),
        operation_sites: Vec::new(),
        statement_tree: None,
        loops: Vec::new(),
        source_anchor: dummy_anchor(),
        identifier_references: Vec::new(),
        unreachable_statements: Vec::new(),
        has_branching: false,
        var_assignments: Vec::new(),
        condition_references: Vec::new(),
    }
}

/// A routine with `kind` carrying `commit` operation sites for each given op id.
pub fn op_commit_routine(id: &str, kind: &str, commit_op_ids: &[&str]) -> L3Routine {
    let mut r = routine(id, kind);
    for op_id in commit_op_ids {
        r.operation_sites.push(POperationSite {
            id: op_id.to_string(),
            kind: "commit".to_string(),
            loop_stack: Vec::new(),
            source_anchor: dummy_anchor(),
            under_asserterror: None,
            control_context: None,
            order: None,
        });
    }
    r
}

/// An object-run call site (e.g. a `Codeunit.Run`) with the given object kind +
/// `objectRunReturnUsed` flag.
pub fn object_run_call_site(id: &str, object_kind: &str, return_used: Option<bool>) -> PCallSite {
    PCallSite {
        id: id.to_string(),
        operation_id: format!("{id}/op"),
        callee_text: format!("{object_kind}.Run"),
        callee: PCallee::ObjectRun {
            object_kind: object_kind.to_string(),
            target_type: object_kind.to_string(),
            target_ref: Some("50100".to_string()),
            target_is_name: false,
        },
        argument_texts: Vec::new(),
        argument_infos: Vec::new(),
        argument_bindings: Vec::new(),
        loop_stack: Vec::new(),
        source_anchor: dummy_anchor(),
        result_consumed: None,
        object_run_return_used: return_used,
        under_asserterror: None,
        control_context: None,
        order: None,
    }
}

/// A capability fact with the given op / resource kind / optional resource id.
/// Other fields are defaulted (they do not affect any L5 query helper).
pub fn fact(op: &str, resource_kind: &str, resource_id: Option<&str>) -> CapabilityFact {
    CapabilityFact {
        subject: "r".to_string(),
        op: op.to_string(),
        resource_kind: resource_kind.to_string(),
        resource_id: resource_id.map(|s| s.to_string()),
        resource_arg_source: None,
        confidence: "static".to_string(),
        provenance: "direct".to_string(),
        via: "self".to_string(),
        witness_operation_id: None,
        witness_callsite_id: None,
        extra: None,
    }
}

/// A coverage record whose `inherited_status` is the given value.
pub fn coverage(inherited_status: &str) -> CoverageRecord {
    CoverageRecord {
        subject: "r".to_string(),
        direct_status: inherited_status.to_string(),
        inherited_status: inherited_status.to_string(),
        reasons: Vec::new(),
        unknown_targets: Vec::new(),
    }
}

/// A `FullRoutineSummary` from direct + inherited facts + optional coverage.
pub fn summary(
    routine_id: &str,
    direct: Vec<CapabilityFact>,
    inherited: Vec<CapabilityFact>,
    cov: Option<CoverageRecord>,
) -> FullRoutineSummary {
    FullRoutineSummary {
        routine_id: routine_id.to_string(),
        capability_facts_direct: direct,
        capability_facts_inherited: inherited,
        coverage: cov,
    }
}
