//! C1 — the compact **derived** capability-cone substrate.
//!
//! `capability_cone`'s per-routine `capability_facts_inherited: Vec<CapabilityFact>`
//! is produced by `retag`-cloning one full `CapabilityFact` struct per (routine ×
//! reachable representative). On a 100,941-routine corpus that is ~27M struct
//! clones held live for the whole detector run (~10.9 GB). Every analyze-path
//! consumer of that Vec reads only a derived predicate off it — a presence
//! boolean, a table/event id-set, or (d44) an id-set tagged with which write ops
//! occurred. This module is that derived view, computed by folding the SAME
//! reachable representatives the `retag` sites already visit, with zero clones.
//!
//! ## What one row summarizes
//!
//! A row is a fold over the routine's REACHABLE cone — `own direct ∪ inherited
//! representatives` — exactly the sequence `FullRoutineSummary::reachable_iter`
//! yields:
//!   - `flags` — presence bits OR-merged over the cone (table / commit / http / file).
//!   - `table_writes_all` — insert|modify|delete on `resource_kind == "table"`
//!     with a `resource_id`, INCLUDING known-temp. Backs `writes_tables_of`.
//!   - `physical_table_writes` — the same, EXCLUDING [`fact_is_known_temp`], each
//!     id carrying a `u8` op-mask. Backs `writes_physical_tables_of` and d44's
//!     per-table op-union.
//!   - `physical_table_reads` — `op == "read"` on a non-known-temp table with a
//!     `resource_id`. Backs d44's read set.
//!   - `event_publishes` — `op == "publish"` on `resource_kind == "event"` with a
//!     `resource_id`. Backs `publishes_events_of`.
//!
//! ## Why the fold is byte-identical to the raw path
//!
//! `retag` (`capability_cone.rs`) rewrites ONLY `subject` / `provenance` / `via` /
//! `witness_callsite_id`. No derived predicate above reads any of those four, so
//! folding the PRE-retag representative is equivalent by construction. The fold
//! therefore runs at the exact `retag` call sites, over the exact same
//! representatives (the singleton path's key-winner `best.values()`, the BFS
//! path's `seen`-deduped reps), plus the routine's own RAW direct facts.
//!
//! **The dedup asymmetry is load-bearing.** The cone dedups inherited facts by
//! `inherited_fact_key = op|resource_kind|resource_id|confidence` — `extra` (and
//! so `temp_state`) is NOT in that key, while the `rep_key` tie-break IS
//! extra-aware. Two facts writing table T with identical `(op, kind, rid,
//! confidence)` but different `temp_state` collapse to ONE representative, and
//! `writes_physical_tables_of` today decides temp-vs-physical on the *winning*
//! representative — not on "any physical fact exists". Folding raw reachable
//! facts instead of the key-winners would flip that whenever a temp fact wins a
//! key. The self half is the mirror image: `capability_facts_direct` is stored
//! RAW (un-deduped) and `reachable_iter` scans every one of them, so the self
//! half must fold the raw Vec, not the key-deduped `direct` map.
//!
//! ## Storage (pooled, not per-routine trees)
//!
//! Four `BTreeSet`/`BTreeMap` per routine would be ~404k tree allocations. Instead
//! the fold runs into reusable scratch `Vec`s, and each routine is frozen into
//! sorted slices appended to four shared pools; the per-routine [`ConeDerivedRow`]
//! holds four `Range<u32>` + `flags` (~40 B). Resource ids are interned once
//! workspace-wide ([`ResInterner`]) — the same CSR playbook `l4::effect_store`
//! uses for db-effects. Interning ORDER never reaches output: every query resolves
//! ids back to `String` and sorts by the resolved string, reproducing the
//! `BTreeSet<String>` ordering the raw helpers produce.

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;

use crate::engine::l4::capability_cone::{CapabilityExtra, CapabilityFact};

// ---------------------------------------------------------------------------
// Vocabularies — the closed sets the fold discriminates on.
// ---------------------------------------------------------------------------

/// Any reachable fact with `resource_kind == "table"` (regardless of op) — backs
/// `touches_db_of`.
pub const TOUCHES_TABLE: u8 = 1 << 0;
/// `op == "commit" && resource_kind == "transaction"` — backs `may_commit`.
pub const MAY_COMMIT: u8 = 1 << 1;
/// `resource_kind == "http"` — half of d48's `routine_touches_external_io`.
pub const TOUCHES_HTTP: u8 = 1 << 2;
/// `resource_kind == "file"` — the other half. `{http, file}` is the COMPLETE IO
/// vocabulary (`d48::is_io_resource_kind`).
pub const TOUCHES_FILE: u8 = 1 << 3;

/// `op == "insert"`.
pub const OP_INSERT: u8 = 1 << 0;
/// `op == "modify"`.
pub const OP_MODIFY: u8 = 1 << 1;
/// `op == "delete"`.
pub const OP_DELETE: u8 = 1 << 2;

/// The table-write op bit for `op`, or `None` when `op` is not a write
/// (al-sem `TABLE_WRITE_OPS = {insert, modify, delete}`; identical to d44's
/// `is_write_op`). The ONE definition of "is this a table write" — `capability_query`
/// and the fold both route through it.
pub fn write_op_bit(op: &str) -> Option<u8> {
    match op {
        "insert" => Some(OP_INSERT),
        "modify" => Some(OP_MODIFY),
        "delete" => Some(OP_DELETE),
        _ => None,
    }
}

/// Decode an op-mask into op literals in **LEXICAL** order — `delete, insert,
/// modify`. d44 unions its ops into a `BTreeSet<&str>` and renders
/// `op_union.join(", ")`, so the decoder must emit the `BTreeSet` iteration
/// order, NOT the mask's bit order.
pub fn decode_op_mask(mask: u8) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    if mask & OP_DELETE != 0 {
        out.push("delete");
    }
    if mask & OP_INSERT != 0 {
        out.push("insert");
    }
    if mask & OP_MODIFY != 0 {
        out.push("modify");
    }
    out
}

/// True when a capability fact is a write/read on a PROVABLY temporary record
/// (`extra == Table { temp_state: known/true }`). Such ops are in-memory — they
/// never touch the physical database, so they cannot create cross-routine /
/// cross-extension table conflicts or exposure. Suppression-direction safe: only
/// the exact `known/true` signal qualifies; `Unknown` / parameter-dependent /
/// absent temp_state keep counting.
///
/// This is the ONE implementation — `l5::capability_query::fact_is_known_temp`
/// re-exports it so the fold and the raw helpers cannot drift apart.
pub fn fact_is_known_temp(f: &CapabilityFact) -> bool {
    matches!(
        &f.extra,
        Some(CapabilityExtra::Table { temp_state: Some(ts), .. })
            if ts.kind == "known" && ts.value == Some(true)
    )
}

// ---------------------------------------------------------------------------
// Output mode — the cone seam's gate.
// ---------------------------------------------------------------------------

/// What a cone composition should PRODUCE. The raw inherited `Vec<CapabilityFact>`
/// is allocated *inside* the cone walk, so a post-hoc check could only discard it;
/// the gate has to be threaded into the walk itself.
///
/// [`ConeOutput::DerivedOnly`] is a real skip, not build-then-drop: neither the
/// per-representative `retag` clone nor the `sort_inherited` allocation happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConeOutput {
    /// Only the [`ConeDerivedStore`]. Every routine's raw inherited Vec is empty.
    DerivedOnly,
    /// Only the raw inherited Vecs (today's behaviour). The store stays empty.
    RawOnly,
    /// Both — the dual-run mode the C1 parity oracle asserts over.
    Both,
}

impl ConeOutput {
    /// True when the raw inherited `Vec<CapabilityFact>` must be materialized.
    pub fn wants_raw(self) -> bool {
        matches!(self, ConeOutput::RawOnly | ConeOutput::Both)
    }

    /// True when the derived substrate must be folded.
    pub fn wants_derived(self) -> bool {
        matches!(self, ConeOutput::DerivedOnly | ConeOutput::Both)
    }
}

// ---------------------------------------------------------------------------
// The resource-id interner.
// ---------------------------------------------------------------------------

/// An interned resource-id handle (TableId / EventId). Lossless — `resolve`
/// returns the exact original string.
pub type ResId = u32;

/// Workspace-global, lossless `String → u32` interner for cone resource ids.
/// Serial (the cone walk is serial). Interning ORDER is irrelevant to any output:
/// every query resolves to `String` and sorts by the resolved string.
#[derive(Debug, Default)]
pub struct ResInterner {
    by_str: HashMap<String, ResId>,
    strings: Vec<String>,
}

impl ResInterner {
    /// Intern `s`, returning its stable handle. Allocates only on first sight.
    pub fn intern(&mut self, s: &str) -> ResId {
        if let Some(id) = self.by_str.get(s) {
            return *id;
        }
        let id = self.strings.len();
        debug_assert!(id < u32::MAX as usize, "ResInterner exhausted");
        let id = id as ResId;
        self.strings.push(s.to_string());
        self.by_str.insert(s.to_string(), id);
        id
    }

    /// The exact original string for `id`.
    ///
    /// # Panics
    /// Panics when `id` was not produced by this interner.
    pub fn resolve(&self, id: ResId) -> &str {
        &self.strings[id as usize]
    }

    /// Number of distinct interned ids.
    pub fn len(&self) -> usize {
        self.strings.len()
    }

    /// True when nothing has been interned.
    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }
}

// ---------------------------------------------------------------------------
// The per-routine row + the pooled store.
// ---------------------------------------------------------------------------

/// One routine's derived cone: presence flags + four `Range<u32>` windows into
/// the owning [`ConeDerivedStore`]'s pools. Meaningless without its store.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConeDerivedRow {
    /// OR-merged presence bits ([`TOUCHES_TABLE`] / [`MAY_COMMIT`] /
    /// [`TOUCHES_HTTP`] / [`TOUCHES_FILE`]).
    pub flags: u8,
    /// Window into `writes_all_pool` — sorted, deduped [`ResId`]s.
    pub table_writes_all: Range<u32>,
    /// Window into `phys_writes_pool` — sorted, deduped `(ResId, op-mask)`.
    pub physical_table_writes: Range<u32>,
    /// Window into `phys_reads_pool` — sorted, deduped [`ResId`]s.
    pub physical_table_reads: Range<u32>,
    /// Window into `events_pool` — sorted, deduped [`ResId`]s.
    pub event_publishes: Range<u32>,
}

/// The row every routine WITHOUT a folded row reads as: no flags, no ids. A
/// routine absent from the store is indistinguishable from one whose cone is
/// empty, which is exactly the raw path's behaviour (no facts ⇒ every presence
/// predicate falls through to the coverage arm).
static EMPTY_ROW: ConeDerivedRow = ConeDerivedRow {
    flags: 0,
    table_writes_all: 0..0,
    physical_table_writes: 0..0,
    physical_table_reads: 0..0,
    event_publishes: 0..0,
};

/// The workspace-wide derived cone substrate: one row per routine plus the shared
/// id pools and the interner. Parked on `DetectorContext` next to
/// `db_effect_bundle` — never referenced from inside a row.
#[derive(Debug, Default)]
pub struct ConeDerivedStore {
    interner: ResInterner,
    writes_all_pool: Vec<ResId>,
    phys_writes_pool: Vec<(ResId, u8)>,
    phys_reads_pool: Vec<ResId>,
    events_pool: Vec<ResId>,
    rows: HashMap<String, ConeDerivedRow>,
}

fn window<'p, T>(pool: &'p [T], r: &Range<u32>) -> &'p [T] {
    &pool[r.start as usize..r.end as usize]
}

impl ConeDerivedStore {
    /// This routine's row, or the empty row when it has none.
    pub fn row(&self, routine_id: &str) -> &ConeDerivedRow {
        self.rows.get(routine_id).unwrap_or(&EMPTY_ROW)
    }

    /// Number of folded rows (one per routine the cone visited).
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when no row was folded (e.g. a `RawOnly` composition).
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The id interner — for callers that need to resolve raw [`ResId`]s.
    pub fn interner(&self) -> &ResInterner {
        &self.interner
    }

    /// Drop this routine's row, so it reads as the empty row again.
    ///
    /// Needed because a derived row must equal the fold of the summary the
    /// context ACTUALLY ends up holding, and a caller can end up holding a
    /// degenerate summary: `build_detector_context` assembles summaries by
    /// `remove()`-ing each routine's cone entry, and two AL routines can collide
    /// on one internal routine id (two same-name triggers in one object — gap
    /// G-18). The second occurrence's `remove()` then yields `None`, so the
    /// summary that SURVIVES the map insert has no direct facts, no inherited
    /// facts and no coverage. Its derived row must be empty to match.
    ///
    /// Cheap and rare: the pooled ids stay behind as dead space (a handful of
    /// routines per workspace), only the row lookup is removed.
    pub fn forget(&mut self, routine_id: &str) {
        self.rows.remove(routine_id);
    }

    /// Every routine id with a folded row (iteration order unspecified — never
    /// let it reach output).
    pub fn routine_ids(&self) -> impl Iterator<Item = &str> {
        self.rows.keys().map(|s| s.as_str())
    }

    /// This routine's presence flags.
    pub fn flags_of(&self, routine_id: &str) -> u8 {
        self.row(routine_id).flags
    }

    /// Any reachable `resource_kind == "table"` fact (the Yes arm of
    /// `touches_db_of`).
    pub fn touches_table(&self, routine_id: &str) -> bool {
        self.flags_of(routine_id) & TOUCHES_TABLE != 0
    }

    /// Any reachable `commit` on `transaction` (the Yes arm of `may_commit`).
    pub fn may_commit_flag(&self, routine_id: &str) -> bool {
        self.flags_of(routine_id) & MAY_COMMIT != 0
    }

    /// Any reachable http/file fact — d48's `routine_touches_external_io`.
    pub fn touches_io(&self, routine_id: &str) -> bool {
        self.flags_of(routine_id) & (TOUCHES_HTTP | TOUCHES_FILE) != 0
    }

    /// Sorted + deduped written TableIds, temp-INCLUSIVE (`writes_tables_of`).
    pub fn writes_tables_of(&self, routine_id: &str) -> Vec<String> {
        self.resolve_sorted(window(
            &self.writes_all_pool,
            &self.row(routine_id).table_writes_all,
        ))
    }

    /// Sorted + deduped PHYSICAL (non-known-temp) written TableIds
    /// (`writes_physical_tables_of`).
    pub fn writes_physical_tables_of(&self, routine_id: &str) -> Vec<String> {
        let ids = window(
            &self.phys_writes_pool,
            &self.row(routine_id).physical_table_writes,
        );
        let mut out: Vec<String> = ids
            .iter()
            .map(|(id, _)| self.interner.resolve(*id).to_string())
            .collect();
        out.sort();
        out
    }

    /// Sorted + deduped PHYSICAL (non-known-temp) READ TableIds — d44's read set.
    pub fn physical_table_reads_of(&self, routine_id: &str) -> Vec<String> {
        self.resolve_sorted(window(
            &self.phys_reads_pool,
            &self.row(routine_id).physical_table_reads,
        ))
    }

    /// Sorted + deduped published EventIds (`publishes_events_of`).
    pub fn publishes_events_of(&self, routine_id: &str) -> Vec<String> {
        self.resolve_sorted(window(
            &self.events_pool,
            &self.row(routine_id).event_publishes,
        ))
    }

    /// PHYSICAL written TableId → its op set, in d44's `BTreeSet<&str>` order
    /// (`delete, insert, modify`) — d44's per-table op-union.
    pub fn physical_table_write_ops_of(
        &self,
        routine_id: &str,
    ) -> BTreeMap<String, Vec<&'static str>> {
        let ids = window(
            &self.phys_writes_pool,
            &self.row(routine_id).physical_table_writes,
        );
        ids.iter()
            .map(|(id, mask)| {
                (
                    self.interner.resolve(*id).to_string(),
                    decode_op_mask(*mask),
                )
            })
            .collect()
    }

    /// Resolve a pooled id run to sorted `String`s. The run is already deduped by
    /// [`ResId`] and interning is injective, so the resolved strings are distinct
    /// — identical to the raw helpers' `BTreeSet<String>` collect.
    fn resolve_sorted(&self, ids: &[ResId]) -> Vec<String> {
        let mut out: Vec<String> = ids
            .iter()
            .map(|id| self.interner.resolve(*id).to_string())
            .collect();
        out.sort();
        out
    }
}

// ---------------------------------------------------------------------------
// The fold.
// ---------------------------------------------------------------------------

/// Folds reachable facts into per-routine [`ConeDerivedRow`]s. One builder per
/// cone composition; the scratch buffers are reused across routines so a
/// 100k-routine walk allocates the four pools and nothing else.
///
/// Protocol per routine: [`begin_routine`](Self::begin_routine) →
/// [`fold_fact`](Self::fold_fact)* → [`finish_routine`](Self::finish_routine).
#[derive(Debug, Default)]
pub struct ConeDerivedBuilder {
    store: ConeDerivedStore,
    flags: u8,
    s_writes_all: Vec<ResId>,
    s_phys_writes: Vec<(ResId, u8)>,
    s_phys_reads: Vec<ResId>,
    s_events: Vec<ResId>,
}

impl ConeDerivedBuilder {
    /// Start a fresh routine (clears the scratch buffers, keeping their capacity).
    pub fn begin_routine(&mut self) {
        self.flags = 0;
        self.s_writes_all.clear();
        self.s_phys_writes.clear();
        self.s_phys_reads.clear();
        self.s_events.clear();
    }

    /// Fold ONE reachable fact into the routine in progress. Reads only fields
    /// `retag` does not rewrite (`op` / `resource_kind` / `resource_id` /
    /// `extra`), so a pre-retag representative folds identically to its retagged
    /// clone. Idempotent per distinct `(kind, op, resource_id, temp)` — folding a
    /// fact twice is a no-op after the freeze dedup.
    pub fn fold_fact(&mut self, f: &CapabilityFact) {
        match f.resource_kind.as_str() {
            "table" => {
                self.flags |= TOUCHES_TABLE;
                let Some(rid) = f.resource_id.as_deref() else {
                    return;
                };
                let is_temp = fact_is_known_temp(f);
                if let Some(bit) = write_op_bit(&f.op) {
                    let id = self.store.interner.intern(rid);
                    self.s_writes_all.push(id);
                    if !is_temp {
                        self.s_phys_writes.push((id, bit));
                    }
                } else if f.op == "read" && !is_temp {
                    let id = self.store.interner.intern(rid);
                    self.s_phys_reads.push(id);
                }
            }
            "transaction" => {
                if f.op == "commit" {
                    self.flags |= MAY_COMMIT;
                }
            }
            "http" => self.flags |= TOUCHES_HTTP,
            "file" => self.flags |= TOUCHES_FILE,
            "event" => {
                if f.op == "publish"
                    && let Some(rid) = f.resource_id.as_deref()
                {
                    let id = self.store.interner.intern(rid);
                    self.s_events.push(id);
                }
            }
            _ => {}
        }
    }

    /// Freeze the routine in progress into the pools and record its row.
    pub fn finish_routine(&mut self, routine_id: &str) {
        let table_writes_all = freeze_ids(&mut self.s_writes_all, &mut self.store.writes_all_pool);
        let physical_table_writes =
            freeze_masked(&mut self.s_phys_writes, &mut self.store.phys_writes_pool);
        let physical_table_reads =
            freeze_ids(&mut self.s_phys_reads, &mut self.store.phys_reads_pool);
        let event_publishes = freeze_ids(&mut self.s_events, &mut self.store.events_pool);
        self.store.rows.insert(
            routine_id.to_string(),
            ConeDerivedRow {
                flags: self.flags,
                table_writes_all,
                physical_table_writes,
                physical_table_reads,
                event_publishes,
            },
        );
    }

    /// Fold ONE routine's complete reachable sequence in a single call
    /// (begin → fold each → finish).
    ///
    /// The production cone does NOT use this: its inherited half must fold
    /// key-deduped representatives, not a flat reachable list (see the module
    /// docs' dedup-asymmetry note). It exists for callers that already hold the
    /// literal reachable sequence — hand-built fixture summaries, whose
    /// `capability_facts_inherited` IS the input rather than a cone output.
    pub fn fold_routine<'f>(
        &mut self,
        routine_id: &str,
        reachable: impl IntoIterator<Item = &'f CapabilityFact>,
    ) {
        self.begin_routine();
        for f in reachable {
            self.fold_fact(f);
        }
        self.finish_routine(routine_id);
    }

    /// Consume the builder, yielding the frozen store.
    pub fn finish(self) -> ConeDerivedStore {
        self.store
    }
}

/// Sort + dedup `scratch` and append it to `pool`, returning its window.
fn freeze_ids(scratch: &mut Vec<ResId>, pool: &mut Vec<ResId>) -> Range<u32> {
    scratch.sort_unstable();
    scratch.dedup();
    let start = pool.len() as u32;
    pool.extend_from_slice(scratch);
    start..pool.len() as u32
}

/// Sort `scratch` by id, OR-merge the masks of equal ids, and append to `pool`.
fn freeze_masked(scratch: &mut [(ResId, u8)], pool: &mut Vec<(ResId, u8)>) -> Range<u32> {
    scratch.sort_unstable();
    let start = pool.len();
    for &(id, mask) in scratch.iter() {
        // Merge only within THIS routine's window — never into the previous
        // routine's last entry.
        if pool.len() > start
            && let Some(last) = pool.last_mut()
            && last.0 == id
        {
            last.1 |= mask;
            continue;
        }
        pool.push((id, mask));
    }
    start as u32..pool.len() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::l2::features::PTempState;
    use crate::engine::l4::capability_cone::compose_cone_over_graph;
    use crate::engine::l4::combined_graph::{CombinedGraph, TypedEdge};
    use std::collections::HashMap;

    // -- fixture constructors -------------------------------------------------

    fn fact(subject: &str, op: &str, kind: &str, rid: Option<&str>) -> CapabilityFact {
        CapabilityFact {
            subject: subject.to_string(),
            op: op.to_string(),
            resource_kind: kind.to_string(),
            resource_id: rid.map(|s| s.to_string()),
            resource_arg_source: None,
            confidence: "static".to_string(),
            provenance: "direct".to_string(),
            via: "self".to_string(),
            witness_operation_id: None,
            witness_callsite_id: None,
            extra: None,
        }
    }

    /// A table fact carrying an explicit `temp_state` (the physical/temp gate).
    fn temp_fact(
        subject: &str,
        op: &str,
        rid: &str,
        kind: &str,
        value: Option<bool>,
    ) -> CapabilityFact {
        let mut f = fact(subject, op, "table", Some(rid));
        f.extra = Some(CapabilityExtra::Table {
            record_variable_id: None,
            temp_state: Some(PTempState {
                kind: kind.to_string(),
                value,
                parameter_index: None,
            }),
            op_subtype: None,
        });
        f
    }

    fn call_edge(from: &str, to: &str, callsite: &str) -> TypedEdge {
        TypedEdge {
            kind: "direct-call".to_string(),
            from: from.to_string(),
            to: Some(to.to_string()),
            callsite_id: Some(callsite.to_string()),
            operation_id: None,
            event_id: None,
            receiver_type: None,
            interface_name: None,
            candidate_count: None,
            target_object: None,
            object_type: None,
            target_id_source: None,
        }
    }

    fn graph_of(edges: Vec<TypedEdge>) -> CombinedGraph {
        CombinedGraph {
            nodes: Vec::new(),
            edges_by_from: HashMap::new(),
            edges_from_order: Vec::new(),
            uncertainty_edges: Vec::new(),
            typed_edges: edges,
        }
    }

    /// Raw-path `writes_physical_tables_of` over `direct ∪ inherited`, replicating
    /// `capability_query`'s exact filter — the oracle side of these fixtures.
    fn raw_physical_writes(direct: &[CapabilityFact], inherited: &[CapabilityFact]) -> Vec<String> {
        let mut ids: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for f in direct.iter().chain(inherited.iter()) {
            if f.resource_kind != "table" || write_op_bit(&f.op).is_none() || fact_is_known_temp(f)
            {
                continue;
            }
            if let Some(rid) = &f.resource_id {
                ids.insert(rid.clone());
            }
        }
        ids.into_iter().collect()
    }

    // -- R3 source rule 1: the temp/physical dedup trap ------------------------

    /// Two callees write table T with IDENTICAL `(op, resource_kind, resource_id,
    /// confidence)` — one known-temp, one physical. They collapse to ONE cone
    /// representative (`extra` is not in `inherited_fact_key`), and the WINNER
    /// decides whether the ancestor "writes a physical table". The fold must run
    /// over that winner, not over the raw reachable facts: an "any physical"
    /// union would diverge whenever the temp fact wins the key.
    #[test]
    fn temp_physical_dedup_trap_follows_the_winning_representative() {
        let graph = graph_of(vec![
            call_edge("r/root", "r/tempWriter", "cs/1"),
            call_edge("r/root", "r/physWriter", "cs/2"),
        ]);
        let nodes: Vec<String> = ["r/root", "r/tempWriter", "r/physWriter"]
            .iter()
            .map(|s| s.to_string())
            .collect();

        let mut direct_in: HashMap<String, Vec<CapabilityFact>> = HashMap::new();
        direct_in.insert("r/root".to_string(), Vec::new());
        direct_in.insert(
            "r/tempWriter".to_string(),
            vec![temp_fact(
                "r/tempWriter",
                "insert",
                "t/T",
                "known",
                Some(true),
            )],
        );
        direct_in.insert(
            "r/physWriter".to_string(),
            vec![temp_fact(
                "r/physWriter",
                "insert",
                "t/T",
                "known",
                Some(false),
            )],
        );
        let coverage_in: HashMap<String, (String, Vec<String>)> = HashMap::new();

        let out =
            compose_cone_over_graph(&graph, &nodes, &direct_in, &coverage_in, ConeOutput::Both);

        // The two facts share an `inherited_fact_key`, so the root inherits ONE.
        let root_inherited = &out.cones.get("r/root").expect("root cone").inherited;
        assert_eq!(
            root_inherited.len(),
            1,
            "the two same-key facts must collapse to one representative"
        );
        // The singleton path's equal-distance tie-break is `edge_sort_key`, and
        // `cs/1` (the TEMP callee) sorts first — so the TEMP fact is the winner.
        assert!(
            fact_is_known_temp(&root_inherited[0]),
            "fixture precondition: the temp fact must win the key"
        );

        // The discriminating assertion: the root writes NO physical table, because
        // the surviving representative is the temp one. A fold over the raw
        // reachable facts ("any physical fact exists") would answer `["t/T"]`.
        let raw = raw_physical_writes(&[], root_inherited);
        assert!(raw.is_empty(), "the raw path drops the temp winner");
        assert_eq!(
            out.derived.writes_physical_tables_of("r/root"),
            raw,
            "the fold must decide temp-vs-physical on the winning representative"
        );
        // ...while temp-INCLUSIVE writes still contain the table.
        assert_eq!(out.derived.writes_tables_of("r/root"), vec!["t/T"]);
    }

    // -- R3 source rule 2: BFS sibling facts come from the key-deduped map -----

    /// In a recursive SCC the BFS path emits a sibling MEMBER's facts from the
    /// KEY-DEDUPED `direct` map, while the subject's own half comes from its RAW,
    /// un-deduped `direct_full` Vec. Both halves are pinned here, and both pairs
    /// are built so the KNOWN-TEMP fact wins the `rep_key` tie-break (its
    /// `extra_json` sorts before the `unknown`-temp-state one, which is NOT
    /// known-temp and therefore counts as physical):
    ///
    ///   - `t/Sib` (sibling, key-deduped) ⇒ the temp winner survives ⇒ NOT a
    ///     physical write. Folding the sibling's RAW Vec instead would wrongly
    ///     add it.
    ///   - `t/Own` (subject, RAW) ⇒ both facts are scanned ⇒ the non-temp one
    ///     keeps it a physical write. Folding the subject's KEY-DEDUPED map
    ///     instead would wrongly drop it.
    #[test]
    fn bfs_sibling_facts_use_the_key_deduped_direct_map() {
        let graph = graph_of(vec![
            call_edge("r/a", "r/b", "cs/ab"),
            call_edge("r/b", "r/a", "cs/ba"),
        ]);
        let nodes: Vec<String> = ["r/a", "r/b"].iter().map(|s| s.to_string()).collect();

        let mut direct_in: HashMap<String, Vec<CapabilityFact>> = HashMap::new();
        direct_in.insert(
            "r/a".to_string(),
            vec![
                temp_fact("r/a", "modify", "t/Own", "known", Some(true)),
                temp_fact("r/a", "modify", "t/Own", "unknown", None),
            ],
        );
        direct_in.insert(
            "r/b".to_string(),
            vec![
                temp_fact("r/b", "insert", "t/Sib", "known", Some(true)),
                temp_fact("r/b", "insert", "t/Sib", "unknown", None),
            ],
        );
        let coverage_in: HashMap<String, (String, Vec<String>)> = HashMap::new();

        let out =
            compose_cone_over_graph(&graph, &nodes, &direct_in, &coverage_in, ConeOutput::Both);

        let a_inherited = &out.cones.get("r/a").expect("a cone").inherited;
        // The sibling's two same-key facts arrive as ONE deduped representative —
        // the known-temp one.
        assert_eq!(
            a_inherited.len(),
            1,
            "sibling facts must arrive key-deduped, not raw"
        );
        assert!(
            fact_is_known_temp(&a_inherited[0]),
            "fixture precondition: the temp fact must win the sibling's key"
        );

        let raw = raw_physical_writes(direct_in.get("r/a").unwrap(), a_inherited);
        assert_eq!(
            raw,
            vec!["t/Own"],
            "raw path: the sibling's temp winner is dropped, the subject's own \
             un-deduped physical fact is kept"
        );
        assert_eq!(out.derived.writes_physical_tables_of("r/a"), raw);
        // Temp-inclusive writes span both halves.
        assert_eq!(out.derived.writes_tables_of("r/a"), vec!["t/Own", "t/Sib"]);
    }

    // -- R7: d44's op order is lexical ----------------------------------------

    /// One routine writing ONE table with all three ops: the mask decoder must
    /// emit `delete, insert, modify` — the order d44's `BTreeSet<&str>` yields.
    #[test]
    fn d44_op_mask_decodes_in_lexical_order() {
        assert_eq!(
            decode_op_mask(OP_INSERT | OP_MODIFY | OP_DELETE),
            vec!["delete", "insert", "modify"]
        );

        let graph = graph_of(Vec::new());
        let nodes: Vec<String> = vec!["r/w".to_string()];
        let mut direct_in: HashMap<String, Vec<CapabilityFact>> = HashMap::new();
        direct_in.insert(
            "r/w".to_string(),
            vec![
                fact("r/w", "insert", "table", Some("t/A")),
                fact("r/w", "modify", "table", Some("t/A")),
                fact("r/w", "delete", "table", Some("t/A")),
            ],
        );
        let coverage_in: HashMap<String, (String, Vec<String>)> = HashMap::new();

        let out =
            compose_cone_over_graph(&graph, &nodes, &direct_in, &coverage_in, ConeOutput::Both);
        let ops = out.derived.physical_table_write_ops_of("r/w");
        assert_eq!(ops.len(), 1);
        assert_eq!(ops["t/A"], vec!["delete", "insert", "modify"]);
    }

    // -- flags + the coverage tri-state arms -----------------------------------

    /// Flag presence is op/kind-exact, and the absent-fact tri-state is decided
    /// by `coverage.inherited_status`: complete ⇒ No, partial ⇒ Unknown.
    #[test]
    fn flags_and_coverage_tristate_arms() {
        let graph = graph_of(Vec::new());
        let nodes: Vec<String> = ["r/complete", "r/partial", "r/io"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut direct_in: HashMap<String, Vec<CapabilityFact>> = HashMap::new();
        direct_in.insert("r/complete".to_string(), Vec::new());
        direct_in.insert("r/partial".to_string(), Vec::new());
        direct_in.insert(
            "r/io".to_string(),
            vec![
                fact("r/io", "send", "http", None),
                fact("r/io", "write", "file", None),
                fact("r/io", "commit", "transaction", None),
                // A non-commit transaction op must NOT set MAY_COMMIT.
                fact("r/io", "rollback", "transaction", None),
            ],
        );
        let mut coverage_in: HashMap<String, (String, Vec<String>)> = HashMap::new();
        coverage_in.insert("r/complete".to_string(), ("complete".to_string(), vec![]));
        coverage_in.insert("r/partial".to_string(), ("partial".to_string(), vec![]));

        let out =
            compose_cone_over_graph(&graph, &nodes, &direct_in, &coverage_in, ConeOutput::Both);

        // No facts ⇒ no flags; the tri-state then reads coverage.
        assert!(!out.derived.touches_table("r/complete"));
        assert!(!out.derived.may_commit_flag("r/complete"));
        assert_eq!(
            out.cones["r/complete"].coverage.inherited_status, "complete",
            "absent fact + complete ⇒ the No arm"
        );
        assert_eq!(
            out.cones["r/partial"].coverage.inherited_status, "partial",
            "absent fact + partial ⇒ the Unknown arm"
        );

        assert!(out.derived.touches_io("r/io"));
        assert_eq!(
            out.derived.flags_of("r/io"),
            TOUCHES_HTTP | TOUCHES_FILE | MAY_COMMIT
        );
        assert!(!out.derived.touches_table("r/io"));
    }

    // -- the mode is a real skip ----------------------------------------------

    /// `DerivedOnly` must produce a fully-populated derived row AND an empty raw
    /// Vec; `RawOnly` the mirror image; and the derived rows must be identical
    /// under `DerivedOnly` and `Both` (one traversal, two guarded emissions).
    #[test]
    fn cone_output_mode_skips_the_raw_vec_without_perturbing_the_fold() {
        let graph = graph_of(vec![call_edge("r/root", "r/callee", "cs/1")]);
        let nodes: Vec<String> = ["r/root", "r/callee"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut direct_in: HashMap<String, Vec<CapabilityFact>> = HashMap::new();
        direct_in.insert(
            "r/root".to_string(),
            vec![fact("r/root", "publish", "event", Some("e/E"))],
        );
        direct_in.insert(
            "r/callee".to_string(),
            vec![
                fact("r/callee", "insert", "table", Some("t/A")),
                fact("r/callee", "read", "table", Some("t/B")),
            ],
        );
        let coverage_in: HashMap<String, (String, Vec<String>)> = HashMap::new();

        let both =
            compose_cone_over_graph(&graph, &nodes, &direct_in, &coverage_in, ConeOutput::Both);
        let derived_only = compose_cone_over_graph(
            &graph,
            &nodes,
            &direct_in,
            &coverage_in,
            ConeOutput::DerivedOnly,
        );
        let raw_only = compose_cone_over_graph(
            &graph,
            &nodes,
            &direct_in,
            &coverage_in,
            ConeOutput::RawOnly,
        );

        // DerivedOnly: the raw Vec is never built...
        assert!(
            derived_only.cones.values().all(|c| c.inherited.is_empty()),
            "DerivedOnly must not materialize the raw inherited Vec"
        );
        // ...but the row is fully populated, and identical to `Both`'s.
        assert_eq!(derived_only.derived.writes_tables_of("r/root"), vec!["t/A"]);
        assert_eq!(
            derived_only.derived.physical_table_reads_of("r/root"),
            vec!["t/B"]
        );
        assert_eq!(
            derived_only.derived.publishes_events_of("r/root"),
            vec!["e/E"]
        );
        for id in &nodes {
            assert_eq!(
                derived_only.derived.writes_tables_of(id),
                both.derived.writes_tables_of(id),
                "{id}: DerivedOnly and Both must fold identically"
            );
            assert_eq!(
                derived_only.derived.flags_of(id),
                both.derived.flags_of(id),
                "{id}: flags diverged between modes"
            );
            assert_eq!(
                derived_only.derived.physical_table_write_ops_of(id),
                both.derived.physical_table_write_ops_of(id),
                "{id}: op masks diverged between modes"
            );
        }
        // `Both` still returns the raw Vec unchanged.
        assert_eq!(both.cones["r/root"].inherited.len(), 2);

        // RawOnly: the raw Vec is byte-identical to `Both`'s; no row is folded.
        assert_eq!(
            raw_only.cones["r/root"].inherited,
            both.cones["r/root"].inherited
        );
        assert!(raw_only.derived.is_empty());
    }

    // -- interner invariants ---------------------------------------------------

    #[test]
    fn interner_is_lossless_and_stable() {
        let mut i = ResInterner::default();
        assert!(i.is_empty());
        let a = i.intern("t/A");
        let b = i.intern("t/B");
        assert_eq!(i.intern("t/A"), a, "re-interning must be stable");
        assert_ne!(a, b);
        assert_eq!(i.resolve(a), "t/A");
        assert_eq!(i.resolve(b), "t/B");
        assert_eq!(i.len(), 2);
    }
}
