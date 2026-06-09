//! Port of al-sem `src/digest/unresolved-cone.ts`.
//!
//! BFS over the snapshot's typed-edge graph from a root routine, collecting
//! every callsite-resolution ledger row whose status is NOT "resolved" or
//! "builtin" (open-world honesty: "polymorphic" is always included).
//!
//! The BFS is DETERMINISTIC: the expansion queue is sorted by target routineId
//! at each step (matching the TS sort). Items are deduped by callsiteId string
//! and sorted by (owningRoutine, anchorFile, anchorLine, calleeDisplay).
//!
//! Default caps (must match witness BFS):
//!   maxDepth   = 64
//!   maxVisited = 25_000

use std::collections::{HashMap, HashSet, VecDeque};

use crate::engine::l5::snapshot::{CapabilitySnapshot, SnapshotGraphEdge};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct UnresolvedConeItem {
    /// Workspace-relative forward-slash anchor (may be absent).
    pub callsite_file: Option<String>,
    pub callsite_line: Option<u32>,
    pub callsite_column: Option<u32>,
    pub callee_display: String,
    pub status: String,
    pub candidates: Option<Vec<String>>,
    pub open_world: Option<bool>,
    pub owning_routine: String,
    pub owning_routine_display: String,
}

#[derive(Debug, Clone)]
pub struct UnresolvedTraversal {
    pub truncated: bool,
    pub max_depth: u32,
    pub visited_routines: usize,
}

pub struct UnresolvedConeResult {
    pub items: Vec<UnresolvedConeItem>,
    pub traversal: UnresolvedTraversal,
    /// All visited routine stable IDs (for gapsInCone computation).
    pub visited_ids: HashSet<String>,
}

const DEFAULT_MAX_DEPTH: u32 = 64;
const DEFAULT_MAX_VISITED: usize = 25_000;

// ---------------------------------------------------------------------------
// unresolvedCone
// ---------------------------------------------------------------------------

/// Normalize a source file path for the callsite anchor.
/// Mirrors TS `normalizeAnchorPath(ce.sourceFile, workspaceRoot)` where
/// workspaceRoot is the empty string (no filesystem prefix to strip) — so
/// this is just backslash normalization. The `ws:` / `app:` scheme prefixes
/// are PRESERVED (the caller passed no workspace root to strip against).
fn normalize_anchor_path(p: &str) -> String {
    p.replace('\\', "/")
}

pub fn unresolved_cone(snap: &CapabilitySnapshot, root: &str) -> UnresolvedConeResult {
    let max_depth = DEFAULT_MAX_DEPTH;
    let max_visited = DEFAULT_MAX_VISITED;

    // --- displayById (identities table) ---
    let mut display_by_id: HashMap<&str, &str> = HashMap::new();
    for i in 0..snap.identities.stable_ids.len() {
        let id = snap
            .identities
            .stable_ids
            .get(i)
            .map(|s| s.as_str())
            .unwrap_or("");
        let name = snap
            .identities
            .display_names
            .get(i)
            .map(|s| s.as_str())
            .unwrap_or("");
        if !id.is_empty() {
            display_by_id.insert(id, name);
        }
    }

    // --- callsiteById (callsiteIndex) ---
    let mut callsite_by_id: HashMap<&str, &crate::engine::l5::snapshot::SnapshotCallsiteEvidence> =
        HashMap::new();
    for cs in &snap.callsite_index {
        callsite_by_id.insert(cs.callsite_id.as_str(), cs);
    }

    // --- outgoing edges (typed-edge → to) for BFS expansion ---
    // Only edges that have a `to` (i.e. all except object-run-unresolved).
    let mut outgoing: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &snap.typed_edges {
        let from = edge_from(edge);
        let to = edge_to(edge);
        if let Some(to) = to {
            outgoing.entry(from).or_default().push(to);
        }
    }
    // Sort each bucket for determinism.
    for bucket in outgoing.values_mut() {
        bucket.sort_unstable();
    }

    // --- ledger by routine ---
    let mut ledger_by_routine: HashMap<
        &str,
        Vec<&crate::engine::l5::snapshot::SnapshotCallsiteResolution>,
    > = HashMap::new();
    for row in &snap.callsite_resolutions {
        ledger_by_routine
            .entry(row.from.as_str())
            .or_default()
            .push(row);
    }

    // --- BFS ---
    let mut collected: HashSet<String> = HashSet::new(); // routine ids whose ledger was harvested
    let mut visited: HashSet<String> = HashSet::new();
    let mut truncated = false;

    // items deduped by callsiteId
    let mut items_by_csid: HashMap<String, UnresolvedConeItem> = HashMap::new();

    // Queue: (routineId, depth)
    let mut queue: VecDeque<(String, u32)> = VecDeque::new();
    visited.insert(root.to_string());
    queue.push_back((root.to_string(), 0));

    while let Some((routine, depth)) = queue.pop_front() {
        // Harvest ledger rows
        if !collected.contains(&routine) {
            collected.insert(routine.clone());
            let rows = ledger_by_routine
                .get(routine.as_str())
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            for row in rows {
                // Skip "resolved" and "builtin" (open-world rule)
                if row.status == "resolved" || row.status == "builtin" {
                    continue;
                }
                let cs_key = row.callsite_id.clone();
                if items_by_csid.contains_key(&cs_key) {
                    continue; // dedupe
                }

                // Join callsiteIndex for anchor
                let cs_ev = callsite_by_id.get(row.callsite_id.as_str()).copied();
                let (callsite_file, callsite_line, callsite_column) = match cs_ev {
                    Some(ev) => (
                        Some(normalize_anchor_path(&ev.source_file)),
                        Some(ev.start_line),
                        Some(ev.start_column),
                    ),
                    None => (None, None, None),
                };

                let candidates = if row
                    .candidates
                    .as_ref()
                    .map(|v| !v.is_empty())
                    .unwrap_or(false)
                {
                    row.candidates.clone()
                } else {
                    None
                };

                let open_world = if row.open_world == Some(true) {
                    Some(true)
                } else {
                    None
                };

                items_by_csid.insert(
                    cs_key,
                    UnresolvedConeItem {
                        callsite_file,
                        callsite_line,
                        callsite_column,
                        callee_display: row.callee_display.clone(),
                        status: row.status.clone(),
                        candidates,
                        open_world,
                        owning_routine: routine.clone(),
                        owning_routine_display: display_by_id
                            .get(routine.as_str())
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| routine.clone()),
                    },
                );
            }
        }

        // Don't expand if at maxDepth
        if depth >= max_depth {
            let out = outgoing
                .get(routine.as_str())
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            if out.iter().any(|to| !visited.contains(*to)) {
                truncated = true;
            }
            continue;
        }

        // Expand: enqueue unvisited neighbors (sorted for determinism)
        let out = outgoing
            .get(routine.as_str())
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        for &to in out {
            if visited.contains(to) {
                continue;
            }
            if visited.len() >= max_visited {
                truncated = true;
                break;
            }
            visited.insert(to.to_string());
            queue.push_back((to.to_string(), depth + 1));
        }
    }

    // Sort items: (owningRoutine, anchorFile, anchorLine, calleeDisplay)
    let mut all_items: Vec<UnresolvedConeItem> = items_by_csid.into_values().collect();
    all_items.sort_by(|a, b| {
        let r = a.owning_routine.cmp(&b.owning_routine);
        if r != std::cmp::Ordering::Equal {
            return r;
        }
        let af = a.callsite_file.as_deref().unwrap_or("");
        let bf = b.callsite_file.as_deref().unwrap_or("");
        let r = af.cmp(bf);
        if r != std::cmp::Ordering::Equal {
            return r;
        }
        let al = a.callsite_line.unwrap_or(0);
        let bl = b.callsite_line.unwrap_or(0);
        if al != bl {
            return al.cmp(&bl);
        }
        a.callee_display.cmp(&b.callee_display)
    });

    UnresolvedConeResult {
        items: all_items,
        traversal: UnresolvedTraversal {
            truncated,
            max_depth,
            visited_routines: visited.len(),
        },
        visited_ids: visited,
    }
}

// ---------------------------------------------------------------------------
// Edge accessors (replicate from digest.rs — no pub access there)
// ---------------------------------------------------------------------------

fn edge_from(e: &SnapshotGraphEdge) -> &str {
    match e {
        SnapshotGraphEdge::DirectCall { from, .. }
        | SnapshotGraphEdge::VariableTypedCall { from, .. }
        | SnapshotGraphEdge::InterfaceDispatch { from, .. }
        | SnapshotGraphEdge::ObjectRunResolved { from, .. }
        | SnapshotGraphEdge::ObjectRunUnresolved { from, .. }
        | SnapshotGraphEdge::EventDispatch { from, .. } => from,
    }
}

fn edge_to(e: &SnapshotGraphEdge) -> Option<&str> {
    match e {
        SnapshotGraphEdge::DirectCall { to, .. }
        | SnapshotGraphEdge::VariableTypedCall { to, .. }
        | SnapshotGraphEdge::InterfaceDispatch { to, .. }
        | SnapshotGraphEdge::ObjectRunResolved { to, .. }
        | SnapshotGraphEdge::EventDispatch { to, .. } => Some(to),
        SnapshotGraphEdge::ObjectRunUnresolved { .. } => None,
    }
}
