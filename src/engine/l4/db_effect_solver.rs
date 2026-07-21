//! L4 summary-fixpoint redesign (Phase 1) — the interned-bitvector db-effect solver.
//!
//! Step 0: effective-SCC re-decomposition. `run_one_scc` (the old Jacobi solver, see
//! `summary_runner.rs`) excludes fixed leaves AND routines missing from
//! `routines_by_id` when it builds a Tarjan SCC's per-member equation graph. Removing
//! those nodes from a strongly-connected component's induced subgraph can SPLIT one
//! cycle into several DAG-shaped pieces — e.g. `a -> b -> c -> a` with `b` excluded
//! degrades to `c -> a`, no cycle at all. The old solver never re-ran Tarjan on that
//! induced subgraph; the new solver does, via [`effective_sccs`].

use std::collections::HashMap;

use crate::engine::l4::combined_graph::CombinedGraph;
use crate::engine::l4::scc::{Scc, SccInputGraph, tarjan_scc};

/// Re-decompose one Tarjan `Scc` into its *effective* SCCs: the strongly-connected
/// components of the subgraph induced by keeping only members for which
/// `is_recomputed` is true (neither a fixed leaf nor a routine missing from
/// `routines_by_id`) and only edges between two such members.
///
/// A member excluded by `is_recomputed` contributes no node and no edge to the
/// induced subgraph — its outgoing/incoming edges are external inputs to whichever
/// effective SCC touches it, not intra-component dependencies, so the caller must
/// account for them separately (fixed-leaf substitution, not re-decomposition).
///
/// Returned in reverse-topological order (callees first), matching `tarjan_scc`'s
/// own contract — `effective_sccs` re-runs `tarjan_scc` and returns its `.sccs`
/// verbatim, so callers can fold over the result exactly like a normal SCC list.
pub fn effective_sccs(
    scc_entry: &Scc,
    graph: &CombinedGraph,
    is_recomputed: &dyn Fn(&str) -> bool,
) -> Vec<Scc> {
    // 1. Filter to recomputed members only. Keep `scc_entry.members`' own order (it
    //    is already sorted — `tarjan_scc` sorts every member list — so the induced
    //    node list is sorted too, matching `SccInputGraph::nodes`'s deterministic-DFS-
    //    roots contract).
    let nodes: Vec<String> = scc_entry
        .members
        .iter()
        .filter(|m| is_recomputed(m))
        .cloned()
        .collect();

    if nodes.is_empty() {
        return Vec::new();
    }

    let recomputed: std::collections::HashSet<&str> = nodes.iter().map(|s| s.as_str()).collect();

    // 2. Project edges: for each recomputed member, keep only `to`s that are ALSO
    //    recomputed members of THIS Scc. Edges to fixed leaves, missing routines, or
    //    nodes outside this Scc entirely are dropped — they are external inputs, not
    //    intra-component dependencies.
    let mut edges_by_from: HashMap<String, Vec<String>> = HashMap::new();
    for m in &nodes {
        let tos: Vec<String> = graph
            .edges_by_from
            .get(m)
            .into_iter()
            .flatten()
            .map(|e| e.to.as_str())
            .filter(|to| recomputed.contains(to))
            .map(|to| to.to_string())
            .collect();
        edges_by_from.insert(m.clone(), tos);
    }

    // 3. Re-run Tarjan on the induced subgraph and hand back its SCCs verbatim —
    //    already reverse-topological, already deterministic-sorted per component.
    let input = SccInputGraph {
        nodes: &nodes,
        edges_by_from: &edges_by_from,
    };
    tarjan_scc(&input).sccs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::l4::combined_graph::CombinedEdge;

    /// Build a minimal `CombinedGraph` for SCC-decomposition tests: every edge is a
    /// plain "direct" call with a synthetic callsite id, no other L4 machinery
    /// (uncertainty/typed edges, event dispatch) attached.
    fn build_cycle_graph(nodes: &[&str], edges: &[(&str, &str)]) -> CombinedGraph {
        let mut sorted_nodes: Vec<String> = nodes.iter().map(|n| n.to_string()).collect();
        sorted_nodes.sort();

        let mut edges_by_from: HashMap<String, Vec<CombinedEdge>> = HashMap::new();
        let mut edges_from_order: Vec<String> = Vec::new();
        for (from, to) in edges {
            let from = from.to_string();
            let to = to.to_string();
            if !edges_by_from.contains_key(&from) {
                edges_from_order.push(from.clone());
            }
            edges_by_from
                .entry(from.clone())
                .or_default()
                .push(CombinedEdge {
                    from,
                    to,
                    kind: "direct".to_string(),
                    callsite_id: Some("cs".to_string()),
                    operation_id: None,
                    event_id: None,
                    subscriber_app_id: None,
                    resolution: "resolved".to_string(),
                });
        }

        CombinedGraph {
            nodes: sorted_nodes,
            edges_by_from,
            edges_from_order,
            uncertainty_edges: Vec::new(),
            typed_edges: Vec::new(),
        }
    }

    #[test]
    fn fixed_leaf_splits_cycle_into_dag_parts() {
        // Tarjan SCC {a,b,c} with edges a->b->c->a. Mark `b` as NOT recomputed (fixed leaf).
        // Induced graph over {a,c}: a-> (b removed), c->a  => edges: c->a only. No cycle.
        let graph = build_cycle_graph(&["a", "b", "c"], &[("a", "b"), ("b", "c"), ("c", "a")]);
        let scc = Scc {
            members: vec!["a".into(), "b".into(), "c".into()],
            recursive: true,
        };
        let eff = effective_sccs(&scc, &graph, &|id| id != "b");
        // b excluded; a and c are now acyclic (c->a). Two non-recursive singletons.
        let members: Vec<Vec<String>> = eff.iter().map(|s| s.members.clone()).collect();
        assert_eq!(eff.len(), 2, "leaf removal splits the cycle");
        assert!(eff.iter().all(|s| !s.recursive));
        // reverse-topo (callees before callers): c calls a, so a (the callee) settles
        // and is emitted BEFORE c (the caller) — this is the opposite pairing from the
        // brief's inline comment, which had caller/callee backwards; verified against
        // `tarjan_scc`'s actual output (see task-3-report.md) and invariant under DFS
        // root order (Tarjan settles a node's SCC only after every SCC it can reach).
        assert_eq!(members, vec![vec!["a".to_string()], vec!["c".to_string()]]);
    }

    #[test]
    fn missing_routine_excluded_same_as_leaf() {
        let graph = build_cycle_graph(&["a", "b"], &[("a", "b"), ("b", "a")]);
        let scc = Scc {
            members: vec!["a".into(), "b".into()],
            recursive: true,
        };
        let eff = effective_sccs(&scc, &graph, &|id| id != "b"); // b missing
        assert_eq!(eff.len(), 1);
        assert_eq!(eff[0].members, vec!["a".to_string()]);
        assert!(!eff[0].recursive);
    }
}
