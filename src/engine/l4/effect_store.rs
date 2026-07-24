//! Compact per-routine db-effect store (spec Part A Step 2 —
//! `docs/superpowers/specs/2026-07-22-l4-dbeffect-store-and-retirement-design.md`,
//! lines 118-153) — `CompactRoutineSummary` rows + a `u8 ViaRank` + a lazy
//! `DbEffect` projection view (`SummaryBundle::db_effects`).
//!
//! ## Staging (⟨rev⟩, spec Step 2)
//!
//! A row's `terminal_base` field is a [`SetRef`] into an ARENA — but Task A2
//! does NOT hash-cons: each routine gets its OWN arena entry (no sharing
//! across an effective SCC's members, even though their terminal sets are
//! logically identical — see `db_effect_solver::closed_form_union`'s own
//! doc). A3 canonicalizes: exact-content sets collapse to ONE shared
//! `EffectSetId` per effective SCC (+ across effective SCCs with identical
//! content), and `terminal_base`/`base_via` are rebuilt against the shared
//! set — `CompactRoutineSummary`'s field TYPES do not change for that,
//! only what `SetRef`/the ranges point into.
//!
//! ## What stays materialized through A2 (by design, not oversight)
//!
//! `db_effect_solver`'s per-Tarjan-SCC feed-forward (`settled`/`v2_map`:
//! `HashMap<String, RoutineSummary>`) is UNCHANGED — every already-solved
//! routine's `db_effects: Vec<DbEffect>` (owned `String`s) still exists,
//! because `solve_pd_reachability`/`closed_form_union`/`reconstruct_via`/
//! `attribute_pd_substituted_via` all read a settled successor's db_effects
//! by String content. Redesigning that feed-forward path to read
//! `(EffectSetId, delta)` instead of materialized strings is explicitly
//! Step 3's job (spec: "Feed-forward reads settled callees' (EffectSetId,
//! delta), not materialized Strings"), not Step 2/A2's — the spec's own
//! Step 2 write-up for `db_solver_ms`/RSS is explicit that "the FULL RSS win
//! lands at A3 (per-SCC `Vec<DbEffect>`/settled-strings survive)" through
//! A2. So `db_effect_solver::materialize_member_db_effects` still returns an
//! owned `Vec<DbEffect>` (for that feed-forward) — but now BUILDS it by
//! first constructing this module's compact row (no `String`/`format!`
//! costs in that step: just `EffectId`(u32)/[`ViaRank`](u8) arrays) and
//! projecting the row through [`merge_and_project`] — the SAME projection
//! [`SummaryBundle::db_effects`] uses (DRY: one implementation of the
//! "merge terminal ∪ delta, sort by (effect_key, operation_id)" rule).
//!
//! `via` collapses from a `HashMap<(RoutineIx,EffectId), String>` to a
//! `HashMap<(RoutineIx,EffectId), ViaRank>` — the arc's own motivation names
//! this ~7.1M-entry map's `String` values as a dominant materialization
//! cost; A1 already fixed the map's KEY (String → `RoutineIx`), A2 fixes the
//! VALUE (String → `ViaRank`, `Copy`, zero allocation per merge).

use std::collections::HashMap;
use std::ops::Range;

use crate::engine::l4::db_effect_solver::kind_to_temp_state;
use crate::engine::l4::effect_lattice::TempStateKind;
use crate::engine::l4::effect_universe::{EffectId, EffectUniverse};
use crate::engine::l4::routine_interner::RoutineIx;
use crate::engine::l4::summary::DbEffect;

// ---------------------------------------------------------------------------
// ViaRank — the 5-rank via provenance, as a `u8` enum (spec Step 2).
// ---------------------------------------------------------------------------

/// The 5 canonical `via` provenance ranks (mirrors
/// `effect_lattice::merge_via`'s `VIA_RANK`: `direct=4 > implicit-trigger=3 >
/// event-subscriber=2 > dynamic=1 > inherited=0`), stored as `u8` instead of
/// an owned `String` — the Step-2 representation win: ~7.1M via entries at
/// one byte each (⟨rev⟩ this does NOT collapse with A3's set-sharing — via
/// is per-membership provenance, not per-effect identity — so it stays a
/// real ~7.1MB column even after A3; stated here per the spec's own note).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ViaRank {
    Inherited = 0,
    Dynamic = 1,
    EventSubscriber = 2,
    ImplicitTrigger = 3,
    Direct = 4,
}

impl ViaRank {
    /// The canonical string form — byte-identical to the OLD solver's
    /// `via_for_edge_kind`/`merge_via`/`"direct"` literal outputs, since
    /// `DbEffect.via: String` (unchanged this task) is still built from this.
    pub fn as_str(self) -> &'static str {
        match self {
            ViaRank::Direct => "direct",
            ViaRank::ImplicitTrigger => "implicit-trigger",
            ViaRank::EventSubscriber => "event-subscriber",
            ViaRank::Dynamic => "dynamic",
            ViaRank::Inherited => "inherited",
        }
    }

    /// Parse one of the 5 canonical via strings (the same 5
    /// `effect_lattice::via_for_edge_kind`/the `"direct"` literal ever
    /// produce). Any OTHER input defensively floors to `Inherited` (rank 0)
    /// — mirrors the OLD solver's `"inherited"` floor default
    /// (`materialize_member_db_effects`'s
    /// `unwrap_or_else(|| "inherited".to_string())`) — never panics, so a
    /// stray/garbage `via` string degrades to the LOWEST rank rather than
    /// aborting (this repo's "engine never throws in production" rule).
    /// Named `from_str` (not `FromStr::from_str`/`TryFrom`) per the task
    /// brief's literal interface (`ViaRank::from_str(&str)->Self`) — an
    /// infallible, floor-defaulting parse, not the `Result`-returning
    /// trait contract `FromStr` implies.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "direct" => ViaRank::Direct,
            "implicit-trigger" => ViaRank::ImplicitTrigger,
            "event-subscriber" => ViaRank::EventSubscriber,
            "dynamic" => ViaRank::Dynamic,
            _ => ViaRank::Inherited,
        }
    }
}

// ---------------------------------------------------------------------------
// SetRef / CompactRoutineSummary — the compact per-routine row.
// ---------------------------------------------------------------------------

/// A handle into [`SummaryBundle`]'s (or the in-progress
/// [`SummaryBundleBuilder`]'s) terminal-base arena. Task A2: ONE arena entry
/// PER ROUTINE (no hash-consing — see this module's doc); A3 replaces the
/// arena with a hash-consed one and rebuilds `SetRef`s to point at SHARED
/// entries, without changing this type or `CompactRoutineSummary`'s shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetRef(pub u32);

/// One compact per-routine db-effect row (spec Part A Step 2). `pd_delta`/
/// `base_via`/`delta_via` are CSR ranges into [`SummaryBundle`]'s (or the
/// builder's) pooled global arrays — NOT per-row `SmallVec`s (that would
/// multiply inline capacity across ~100k routines and defeat columnar
/// pooling; see this module's doc / the spec's ⟨rev⟩ note).
///
/// `roles`/`uncertainties`/`has_unresolved_calls` are deliberately NOT
/// fields here (per the brief's interface comment: "+ roles/unc/hu handles
/// or kept on RoutineSummary") — Task A2 only compacts `db_effects`; those
/// three stay on the ordinary `RoutineSummary` the per-SCC assembly already
/// builds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactRoutineSummary {
    pub terminal_base: SetRef,
    pub pd_delta: Range<u32>,
    pub base_via: Range<u32>,
    pub delta_via: Range<u32>,
}

// ---------------------------------------------------------------------------
// DbEffectRef — the lazy, borrowing view of one materialized db-effect.
// ---------------------------------------------------------------------------

/// One materialized db-effect, borrowing its identity strings from the
/// bundle's dictionaries (the [`EffectUniverse`] + the workspace-wide
/// `record_variable_id` map) rather than owning them — the "lazy view" the
/// spec's Public API section describes: "`db_effects(routine) -> impl
/// Iterator<Item = DbEffectRef<'_>>` (borrows dictionary strings). Owned
/// `DbEffect` only when a legacy caller asks" ([`Self::to_owned`]).
#[derive(Debug, Clone, Copy)]
pub struct DbEffectRef<'a> {
    pub effect_key: &'a str,
    pub operation_id: &'a str,
    pub op: &'a str,
    pub table_id: &'a str,
    pub record_variable_id: Option<&'a str>,
    pub temp_state: &'a TempStateKind,
    pub via: ViaRank,
}

impl<'a> DbEffectRef<'a> {
    /// Materialize an owned [`DbEffect`] — for legacy callers only (the
    /// compat shim, `aldump`/fingerprint streaming projections that need an
    /// owned value at their own boundary).
    pub fn to_owned(&self) -> DbEffect {
        DbEffect {
            effect_key: self.effect_key.to_string(),
            operation_id: self.operation_id.to_string(),
            op: self.op.to_string(),
            table_id: self.table_id.to_string(),
            record_variable_id: self.record_variable_id.map(str::to_string),
            temp_state: kind_to_temp_state(self.temp_state),
            via: self.via.as_str().to_string(),
        }
    }
}

/// Shared core: merge one routine's (terminal, delta) id/via pairs and
/// project them into [`DbEffectRef`]s, sorted `(effect_key, operation_id)` —
/// byte-identical to the OLD solver's `materialize_member_db_effects` sort
/// (`db_effect_solver.rs`, Task A1: `universe.effect_key_cached` compares,
/// `operation_id` tie-break). BOTH [`SummaryBundle::db_effects`]
/// (post-freeze, reading its own owned arenas) and
/// [`project_db_effects`] (mid-solve, reading the builder's transient
/// pieces before they are pushed into the bundle) delegate here — ONE
/// implementation of the ordering/projection rule (DRY).
fn merge_and_project<'a>(
    terminal_ids: &[EffectId],
    terminal_vias: &[ViaRank],
    delta_ids: &[EffectId],
    delta_vias: &[ViaRank],
    universe: &'a EffectUniverse,
    rvid_by_opid: &'a HashMap<String, Option<String>>,
) -> Vec<DbEffectRef<'a>> {
    debug_assert_eq!(terminal_ids.len(), terminal_vias.len());
    debug_assert_eq!(delta_ids.len(), delta_vias.len());

    let mut items: Vec<(EffectId, ViaRank)> =
        Vec::with_capacity(terminal_ids.len() + delta_ids.len());
    items.extend(
        terminal_ids
            .iter()
            .copied()
            .zip(terminal_vias.iter().copied()),
    );
    items.extend(delta_ids.iter().copied().zip(delta_vias.iter().copied()));

    // Byte-identical to the OLD `materialize_member_db_effects` sort (Task
    // A1: cached `&str` compares, `operation_id` tie-break — vacuous since
    // `operation_id` is already embedded in `effect_key`, kept
    // belt-and-braces per that fn's own doc).
    items.sort_by(|&(a, _), &(b, _)| {
        universe
            .effect_key_cached(a)
            .cmp(universe.effect_key_cached(b))
            .then_with(|| {
                universe
                    .identity(a)
                    .operation_id
                    .cmp(&universe.identity(b).operation_id)
            })
    });

    items
        .into_iter()
        .map(|(id, via)| {
            let identity = universe.identity(id);
            DbEffectRef {
                effect_key: universe.effect_key_cached(id),
                operation_id: &identity.operation_id,
                op: &identity.op,
                table_id: &identity.table_id,
                record_variable_id: rvid_by_opid
                    .get(&identity.operation_id)
                    .and_then(|o| o.as_deref()),
                temp_state: &identity.temp,
                via,
            }
        })
        .collect()
}

/// Project one row's transient pieces directly to an owned `Vec<DbEffect>`,
/// WITHOUT needing a finished [`SummaryBundle`] — used by
/// `db_effect_solver::materialize_member_db_effects` mid-solve (before the
/// bundle exists) to keep feeding `settled`/`v2_map` legacy `Vec<DbEffect>`s
/// (see this module's doc: "settled-strings survive" through A2).
pub fn project_db_effects(
    terminal_ids: &[EffectId],
    terminal_vias: &[ViaRank],
    delta_ids: &[EffectId],
    delta_vias: &[ViaRank],
    universe: &EffectUniverse,
    rvid_by_opid: &HashMap<String, Option<String>>,
) -> Vec<DbEffect> {
    merge_and_project(
        terminal_ids,
        terminal_vias,
        delta_ids,
        delta_vias,
        universe,
        rvid_by_opid,
    )
    .iter()
    .map(DbEffectRef::to_owned)
    .collect()
}

// ---------------------------------------------------------------------------
// SummaryBundleBuilder / SummaryBundle.
// ---------------------------------------------------------------------------

/// Accumulates rows during the per-Tarjan-SCC solve loop
/// (`compute_summaries_v2_bundle_with_leaves`, `summary_runner.rs`) —
/// mirrors how `universe: EffectUniverse` is already threaded `&mut` through
/// that same loop and finalized once, at the end. [`Self::finish`] freezes
/// it into an immutable [`SummaryBundle`] once solving completes.
#[derive(Debug, Default)]
pub struct SummaryBundleBuilder {
    rows: HashMap<RoutineIx, CompactRoutineSummary>,
    base_arena: Vec<Box<[EffectId]>>,
    delta_pool: Vec<EffectId>,
    via_pool: Vec<ViaRank>,
}

impl SummaryBundleBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push one routine's compact row. `terminal_ids`/`terminal_vias` and
    /// `delta_ids`/`delta_vias` must already be PARALLEL (same length, same
    /// order) — the caller (`materialize_member_db_effects`,
    /// `db_effect_solver.rs`) builds them together from the same
    /// presence-bit scan (`iter_set_bits`, ascending `EffectId`/storage
    /// order — matches the spec's Step-3 "storage order" invariant early,
    /// for free).
    pub fn push_row(
        &mut self,
        routine_ix: RoutineIx,
        terminal_ids: Vec<EffectId>,
        terminal_vias: Vec<ViaRank>,
        delta_ids: Vec<EffectId>,
        delta_vias: Vec<ViaRank>,
    ) {
        debug_assert_eq!(terminal_ids.len(), terminal_vias.len());
        debug_assert_eq!(delta_ids.len(), delta_vias.len());

        let terminal_base = SetRef(self.base_arena.len() as u32);
        self.base_arena.push(terminal_ids.into_boxed_slice());

        let base_via_start = self.via_pool.len() as u32;
        self.via_pool.extend(terminal_vias);
        let base_via = base_via_start..self.via_pool.len() as u32;

        let delta_start = self.delta_pool.len() as u32;
        self.delta_pool.extend(delta_ids);
        let pd_delta = delta_start..self.delta_pool.len() as u32;

        let delta_via_start = self.via_pool.len() as u32;
        self.via_pool.extend(delta_vias);
        let delta_via = delta_via_start..self.via_pool.len() as u32;

        self.rows.insert(
            routine_ix,
            CompactRoutineSummary {
                terminal_base,
                pd_delta,
                base_via,
                delta_via,
            },
        );
    }

    /// Freeze into an immutable [`SummaryBundle`], taking ownership of the
    /// (by-then-fully-solved) [`EffectUniverse`] and the workspace-wide
    /// `record_variable_id` dictionary — both built ONCE, outside this
    /// builder, by `compute_summaries_v2_bundle_with_leaves`.
    pub fn finish(
        self,
        universe: EffectUniverse,
        interner: crate::engine::l4::routine_interner::RoutineInterner,
        rvid_by_opid: HashMap<String, Option<String>>,
    ) -> SummaryBundle {
        SummaryBundle {
            rows: self.rows,
            base_arena: self.base_arena,
            delta_pool: self.delta_pool,
            via_pool: self.via_pool,
            universe,
            interner,
            rvid_by_opid,
        }
    }
}

/// The immutable, workspace-complete compact db-effect store (spec Part A,
/// "Public API"). Owns its dictionaries ([`EffectUniverse`], the
/// `record_variable_id` map, and a
/// [`RoutineInterner`](crate::engine::l4::routine_interner::RoutineInterner))
/// so [`Self::db_effects`] needs only `&self`.
pub struct SummaryBundle {
    rows: HashMap<RoutineIx, CompactRoutineSummary>,
    base_arena: Vec<Box<[EffectId]>>,
    delta_pool: Vec<EffectId>,
    via_pool: Vec<ViaRank>,
    universe: EffectUniverse,
    interner: crate::engine::l4::routine_interner::RoutineInterner,
    rvid_by_opid: HashMap<String, Option<String>>,
}

impl SummaryBundle {
    /// Look up the [`RoutineIx`] for a routine id, if interned.
    pub fn routine_ix(&self, routine_id: &str) -> Option<RoutineIx> {
        self.interner.get(routine_id)
    }

    /// True iff `r` has a compact row (i.e. was RECOMPUTED this run — a
    /// fixed leaf never gets a row; see this module's doc + the spec's Step
    /// 3 "fixed leaves get a singleton class" note, which is NOT this
    /// task's job).
    pub fn has_row(&self, r: RoutineIx) -> bool {
        self.rows.contains_key(&r)
    }

    /// The lazy `DbEffect` view for one routine — an ordered merge of
    /// `terminal_base ∪ pd_delta`, sorted `(effect_key, operation_id)`.
    /// Empty iterator for a routine with no row (never panics — mirrors
    /// `materialize_member_db_effects`'s own `None => Vec::new()` fallback
    /// at its call site).
    pub fn db_effects(&self, r: RoutineIx) -> impl Iterator<Item = DbEffectRef<'_>> {
        let Some(row) = self.rows.get(&r) else {
            return Vec::new().into_iter();
        };
        let terminal_ids: &[EffectId] = &self.base_arena[row.terminal_base.0 as usize];
        let base_via: &[ViaRank] =
            &self.via_pool[row.base_via.start as usize..row.base_via.end as usize];
        let delta_ids: &[EffectId] =
            &self.delta_pool[row.pd_delta.start as usize..row.pd_delta.end as usize];
        let delta_via: &[ViaRank] =
            &self.via_pool[row.delta_via.start as usize..row.delta_via.end as usize];

        merge_and_project(
            terminal_ids,
            base_via,
            delta_ids,
            delta_via,
            &self.universe,
            &self.rvid_by_opid,
        )
        .into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::l4::effect_universe::EffectIdentity;
    use crate::engine::l4::routine_interner::RoutineInterner;
    use crate::engine::l4::summary::TempState;

    const CANONICAL: [&str; 5] = [
        "direct",
        "implicit-trigger",
        "event-subscriber",
        "dynamic",
        "inherited",
    ];

    #[test]
    fn via_rank_round_trips_the_5_canonical_strings() {
        for s in CANONICAL {
            assert_eq!(
                ViaRank::from_str(s).as_str(),
                s,
                "ViaRank round-trip broke for {s:?}"
            );
        }
    }

    #[test]
    fn via_rank_ordering_matches_merge_via_precedence() {
        // direct > implicit-trigger > event-subscriber > dynamic > inherited
        assert!(ViaRank::Direct > ViaRank::ImplicitTrigger);
        assert!(ViaRank::ImplicitTrigger > ViaRank::EventSubscriber);
        assert!(ViaRank::EventSubscriber > ViaRank::Dynamic);
        assert!(ViaRank::Dynamic > ViaRank::Inherited);
    }

    #[test]
    fn via_rank_from_bogus_string_floors_to_inherited() {
        assert_eq!(ViaRank::from_str("totally-bogus-via"), ViaRank::Inherited);
    }

    /// Build a tiny universe with 2 terminal effects (one `Known`, one
    /// `Unknown`, non-trivially ordered by `effect_key`) + 1 `PD` effect,
    /// push a row via the builder, freeze, and assert `db_effects` yields
    /// the SAME `Vec<DbEffect>` shape (`effect_key`/op/table_id/
    /// operation_id/temp_state/via/record_variable_id, in `(effect_key,
    /// operation_id)` order) that the OLD `materialize_member_db_effects`
    /// would have produced from the same (bits, via_map) inputs.
    #[test]
    fn lazy_db_effects_view_reproduces_byte_identical_effects_for_a_fixture() {
        let mut universe = EffectUniverse::new();
        // "Zeta" sorts AFTER "Alpha" by effect_key — intern out of key order
        // so a correct implementation must actually reorder.
        let zeta_known = universe.intern(&EffectIdentity {
            op: "Zeta".to_string(),
            table_id: "t1".to_string(),
            operation_id: "op1".to_string(),
            temp: TempStateKind::Known(true),
        });
        let alpha_unknown = universe.intern(&EffectIdentity {
            op: "Alpha".to_string(),
            table_id: "t1".to_string(),
            operation_id: "op2".to_string(),
            temp: TempStateKind::Unknown,
        });
        let pd_effect = universe.intern(&EffectIdentity {
            op: "Middle".to_string(),
            table_id: "t2".to_string(),
            operation_id: "op3".to_string(),
            temp: TempStateKind::ParameterDependent(0),
        });

        let mut interner = RoutineInterner::new();
        let r = interner.intern("r");

        let mut rvid_by_opid: HashMap<String, Option<String>> = HashMap::new();
        rvid_by_opid.insert("op1".to_string(), Some("Rec".to_string()));
        rvid_by_opid.insert("op2".to_string(), None);
        rvid_by_opid.insert("op3".to_string(), Some("Rec2".to_string()));

        let mut builder = SummaryBundleBuilder::new();
        builder.push_row(
            r,
            vec![zeta_known, alpha_unknown],
            vec![ViaRank::Direct, ViaRank::EventSubscriber],
            vec![pd_effect],
            vec![ViaRank::Dynamic],
        );
        let bundle = builder.finish(universe, interner, rvid_by_opid);

        let got: Vec<DbEffect> = bundle.db_effects(r).map(|e| e.to_owned()).collect();

        assert_eq!(got.len(), 3);
        // Sorted by effect_key: "Alpha|..." < "Middle|..." < "Zeta|...".
        assert_eq!(got[0].op, "Alpha");
        assert_eq!(got[0].operation_id, "op2");
        assert_eq!(got[0].table_id, "t1");
        assert_eq!(got[0].temp_state, TempState::Unknown);
        assert_eq!(got[0].via, "event-subscriber");
        assert_eq!(got[0].record_variable_id, None);

        assert_eq!(got[1].op, "Middle");
        assert_eq!(got[1].operation_id, "op3");
        assert_eq!(got[1].temp_state, TempState::ParameterDependent(0));
        assert_eq!(got[1].via, "dynamic");
        assert_eq!(got[1].record_variable_id, Some("Rec2".to_string()));

        assert_eq!(got[2].op, "Zeta");
        assert_eq!(got[2].operation_id, "op1");
        assert_eq!(got[2].temp_state, TempState::Known(true));
        assert_eq!(got[2].via, "direct");
        assert_eq!(got[2].record_variable_id, Some("Rec".to_string()));
    }

    #[test]
    fn db_effects_is_empty_for_a_routine_with_no_row() {
        let universe = EffectUniverse::new();
        let mut interner = RoutineInterner::new();
        let r = interner.intern("leaf");
        let rvid_by_opid: HashMap<String, Option<String>> = HashMap::new();
        let builder = SummaryBundleBuilder::new();
        let bundle = builder.finish(universe, interner, rvid_by_opid);
        assert_eq!(bundle.db_effects(r).count(), 0);
        assert!(!bundle.has_row(r));
    }

    /// `project_db_effects` (the mid-solve, pre-bundle projection) must
    /// agree with `SummaryBundle::db_effects` (the post-freeze lazy view) —
    /// both delegate to the SAME `merge_and_project` core (DRY), so feeding
    /// them the identical pieces must yield identical output.
    #[test]
    fn project_db_effects_agrees_with_bundle_db_effects() {
        let mut universe = EffectUniverse::new();
        let e1 = universe.intern(&EffectIdentity {
            op: "Insert".to_string(),
            table_id: "t1".to_string(),
            operation_id: "op1".to_string(),
            temp: TempStateKind::Known(true),
        });
        let e2 = universe.intern(&EffectIdentity {
            op: "Modify".to_string(),
            table_id: "t2".to_string(),
            operation_id: "op2".to_string(),
            temp: TempStateKind::ParameterDependent(1),
        });
        let rvid_by_opid: HashMap<String, Option<String>> = HashMap::new();

        let eager = project_db_effects(
            &[e1],
            &[ViaRank::Direct],
            &[e2],
            &[ViaRank::Inherited],
            &universe,
            &rvid_by_opid,
        );

        let mut interner = RoutineInterner::new();
        let r = interner.intern("r");
        let mut builder = SummaryBundleBuilder::new();
        builder.push_row(
            r,
            vec![e1],
            vec![ViaRank::Direct],
            vec![e2],
            vec![ViaRank::Inherited],
        );
        let bundle = builder.finish(universe, interner, rvid_by_opid);
        let lazy: Vec<DbEffect> = bundle.db_effects(r).map(|e| e.to_owned()).collect();

        assert_eq!(eager, lazy);
    }
}
