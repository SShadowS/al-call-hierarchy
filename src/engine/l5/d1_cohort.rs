//! `d1_cohort` — Task C1 of the d1 cohort redesign
//! (`.superpowers/sdd/task-c1-brief.md`,
//! `docs/superpowers/plans/2026-07-21-d1-cohort-redesign.md`): the terminal
//! bitmap-cohort SINK that replaces the per-`(loop, terminal)` witness
//! materialization (`d1_dataflow::emit_lane_aggregates`) causing the ~8-hour
//! Base App 8020 run.
//!
//! ## Why a sink (the output-bound diagnosis)
//!
//! The batched dataflow fixpoint + running-best scan are FAST and CORRECT
//! (ms/batch). The cost is EMISSION: `emit_lane_aggregates` builds one full
//! witness per `(loop, terminal)` aggregate (3.2M builds, each walking ~28k-hop
//! predecessor chains). This module keeps the fixpoint + scan UNCHANGED and
//! replaces the per-loop witness/context emission with **terminal bitmap
//! cohorts**: per terminal, a map [`ContextKey`] → [`GroupBitmap`] recording
//! WHICH loops realize each semantic class, plus a per-`(terminal, verdict)`
//! bitmap for `reachable_verdicts`. No witness is built in the sink path.
//!
//! ## The disjointness invariant (the correctness simplification)
//!
//! The scan's `best[lane]` already selects ONE winner per `(terminal, loop)`.
//! So each loop lands in EXACTLY ONE [`ContextKey`] per terminal (its winner) —
//! the general "bitmap subtraction across candidate cohorts" (gpt r5) is NOT
//! needed here. [`TerminalSink::insert`] just SETS the winner's bit; the
//! disjointness (a loop appears in ≤1 ctx per terminal) is ASSERTED.
//!
//! ## The differential spine
//!
//! `decompress(sink)` — every `(loop, terminal)` with its ctx
//! `verdict`/`depth_bucket`/`unc` + `reachable_verdicts` — MUST equal the
//! tuples the current `emit_lane_aggregates` emits, on components verdict /
//! depth_bucket / unc / coverage / reachable_verdicts. Witness is NOT compared
//! (it becomes a representative, built in a later task). The differential lives
//! in `d1_dataflow`'s `tests` module.
#![allow(dead_code)]

use std::collections::HashMap;

use crate::engine::l5::d1_liveness::Liveness;
use crate::engine::l5::detectors::d1::TempVerdict;

/// A loop-group index — dense over the sorted loop-group universe
/// (`search_loops`'s `groups` vector; up to ~6178 groups on Base App 8020).
pub(crate) type GroupIx = u32;

/// A dense terminal index — assigned first-seen by the sink's terminal-key
/// interning ([`TerminalSink::terminal_ix`]).
pub(crate) type TerminalIx = usize;

/// `reachable_verdicts` is projected over the four verdicts in
/// [`TempVerdict`] DECLARATION order — the SAME order the old
/// `d1_dataflow::reachable_from_masks` / `LoopTerminalAgg::reachable_verdicts`
/// (a sorted+deduped `Vec`) produces. `verdict as usize` is the array index.
const VERDICT_ORDER: [TempVerdict; 4] = [
    TempVerdict::Temporary,
    TempVerdict::Physical,
    TempVerdict::Uncertain,
    TempVerdict::FlowFieldGated,
];

// ===========================================================================
// GroupBitmap — a dense-lazy loop-set bitmap over GroupIx
// ===========================================================================

/// A dense-lazy bitmap over [`GroupIx`]. `Empty` costs nothing until the first
/// [`Self::set`]; `Dense` holds `ceil(highest_bit / 64)` words, grown lazily to
/// fit the highest set bit (no up-front `n_groups` sizing needed).
#[derive(Default)]
pub(crate) enum GroupBitmap {
    #[default]
    Empty,
    Dense(Box<[u64]>),
}

/// Iterates the set bits of one `u64` word, yielding absolute [`GroupIx`]es
/// (`base + bit`). Lowest bit first (`trailing_zeros`).
struct WordBits {
    word: u64,
    base: u32,
}

impl Iterator for WordBits {
    type Item = GroupIx;
    fn next(&mut self) -> Option<GroupIx> {
        if self.word == 0 {
            return None;
        }
        let b = self.word.trailing_zeros();
        self.word &= self.word - 1;
        Some(self.base + b)
    }
}

impl GroupBitmap {
    pub(crate) fn new() -> Self {
        GroupBitmap::Empty
    }

    fn words(&self) -> &[u64] {
        match self {
            GroupBitmap::Empty => &[],
            GroupBitmap::Dense(w) => w,
        }
    }

    /// Grow (or allocate) so the backing store holds at least `n_words` words.
    fn ensure_words(&mut self, n_words: usize) {
        match self {
            GroupBitmap::Empty => {
                *self = GroupBitmap::Dense(vec![0u64; n_words].into_boxed_slice());
            }
            GroupBitmap::Dense(words) => {
                if words.len() < n_words {
                    let mut v = words.to_vec();
                    v.resize(n_words, 0);
                    *words = v.into_boxed_slice();
                }
            }
        }
    }

    /// Set the bit for group `g`.
    pub(crate) fn set(&mut self, g: GroupIx) {
        let w = (g as usize) / 64;
        let b = (g as usize) % 64;
        self.ensure_words(w + 1);
        if let GroupBitmap::Dense(words) = self {
            words[w] |= 1u64 << b;
        }
    }

    /// Whether group `g`'s bit is set.
    pub(crate) fn contains(&self, g: GroupIx) -> bool {
        let w = (g as usize) / 64;
        let b = (g as usize) % 64;
        let words = self.words();
        w < words.len() && (words[w] >> b) & 1 == 1
    }

    /// OR `other`'s bits into `self` (grows `self` to fit).
    pub(crate) fn or_with(&mut self, other: &GroupBitmap) {
        if let GroupBitmap::Dense(o) = other {
            self.ensure_words(o.len());
            if let GroupBitmap::Dense(words) = self {
                for (w, ov) in words.iter_mut().zip(o.iter()) {
                    *w |= *ov;
                }
            }
        }
    }

    /// Whether no bit is set.
    pub(crate) fn is_empty(&self) -> bool {
        self.words().iter().all(|&w| w == 0)
    }

    /// The number of set bits (the loop count of the cohort).
    pub(crate) fn count(&self) -> u64 {
        self.words().iter().map(|w| w.count_ones() as u64).sum()
    }

    /// Iterate the set groups in ascending [`GroupIx`] order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = GroupIx> + '_ {
        self.words()
            .iter()
            .enumerate()
            .flat_map(|(wi, &word)| WordBits {
                word,
                base: (wi as u32) * 64,
            })
    }
}

// ===========================================================================
// ContextKey — the per-(terminal, semantic-class) identity (EXCLUDES loop)
// ===========================================================================

/// The per-`(terminal, class)` context identity — everything a
/// `LoopTerminalAgg` carries about the winner EXCEPT the loop and the witness:
/// `severity` / `verdict` / `depth_bucket` / `unc`. Many loops share one
/// `ContextKey` per terminal (the cohort); the [`GroupBitmap`] records which.
///
/// `Hash` is hand-written (not derived) because [`TempVerdict`] does not derive
/// `Hash`; it hashes the same four fields the derived `PartialEq`/`Eq` compare,
/// so the `Eq`/`Hash` contract holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ContextKey {
    pub severity: &'static str,
    pub verdict: TempVerdict,
    pub depth_bucket: i64,
    pub unc: bool,
}

impl std::hash::Hash for ContextKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.severity.hash(state);
        (self.verdict as i32).hash(state);
        self.depth_bucket.hash(state);
        self.unc.hash(state);
    }
}

/// A lightweight, arena-independent handle to a `(terminal, ContextKey)`
/// class's representative winner — stored FIRST-SEEN. Task C1 does NOT build or
/// consume a witness (that is a later task); this records only the winner's
/// first-arrival hop count, the one scalar that outlives the per-batch fact
/// arena and that the eventual bounded representative witness will need.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BestRefLite {
    pub hops: u32,
}

// ===========================================================================
// TerminalSink — run-global: Terminal -> ContextKey -> loop-bitmap
// ===========================================================================

/// One finalized terminal: its `(owner_id, op_id)` key, its context cohorts
/// (each a `ContextKey` + the loops realizing it + the representative), and the
/// per-verdict reachable-loop bitmaps (`verdict_sets[verdict as usize]`).
pub(crate) struct TerminalCohorts<'a> {
    pub key: (&'a str, &'a str),
    pub cohorts: Vec<(ContextKey, GroupBitmap, BestRefLite)>,
    pub verdict_sets: [GroupBitmap; 4],
}

/// The run-global terminal sink: per terminal, `ContextKey → GroupBitmap`
/// (which loops realize each class) + one [`GroupBitmap`] per verdict (which
/// loops have that verdict reaching the terminal, for `reachable_verdicts`).
///
/// Terminals are interned first-seen to dense [`TerminalIx`]es (stable across
/// batches — a plan terminal reached in two batches keeps ONE slot). Each
/// `(terminal, loop)` is inserted exactly once (its winner), so the cohorts of
/// a terminal PARTITION its reaching loops — the disjointness invariant, which
/// [`Self::insert`] asserts.
pub(crate) struct TerminalSink<'a> {
    /// Group-index universe size (for the debug-only range check).
    n_groups: usize,
    ix_of: HashMap<(&'a str, &'a str), TerminalIx>,
    keys: Vec<(&'a str, &'a str)>,
    /// Per terminal: `ContextKey -> (loop cohort, first-seen representative)`.
    cohorts: Vec<HashMap<ContextKey, (GroupBitmap, BestRefLite)>>,
    /// Per terminal: per-verdict (indexed `verdict as usize`) reaching loops.
    verdicts: Vec<[GroupBitmap; 4]>,
    /// Per terminal: every loop already inserted (any ctx) — the disjointness
    /// guard (a loop must appear in ≤1 ctx per terminal).
    seen: Vec<GroupBitmap>,
}

impl<'a> TerminalSink<'a> {
    /// `n_terminals` is a capacity hint (the terminal plan's terminal count);
    /// `n_groups` bounds the group-index universe (for the range check).
    pub(crate) fn new(n_terminals: usize, n_groups: usize) -> Self {
        TerminalSink {
            n_groups,
            ix_of: HashMap::with_capacity(n_terminals),
            keys: Vec::with_capacity(n_terminals),
            cohorts: Vec::with_capacity(n_terminals),
            verdicts: Vec::with_capacity(n_terminals),
            seen: Vec::with_capacity(n_terminals),
        }
    }

    /// Intern a terminal key `(owner_id, op_id)` to its dense [`TerminalIx`],
    /// stable across batches (the same key always maps to the same slot).
    pub(crate) fn terminal_ix(&mut self, key: (&'a str, &'a str)) -> TerminalIx {
        if let Some(&ix) = self.ix_of.get(&key) {
            return ix;
        }
        let ix = self.keys.len();
        self.ix_of.insert(key, ix);
        self.keys.push(key);
        self.cohorts.push(HashMap::new());
        self.verdicts
            .push(std::array::from_fn(|_| GroupBitmap::new()));
        self.seen.push(GroupBitmap::new());
        ix
    }

    /// Record loop `group`'s winner at `terminal`: set its bit in the `ctx`
    /// cohort (recording the first-seen representative for the class) and set it
    /// in each reaching verdict's bitmap. Asserts the disjointness invariant.
    pub(crate) fn insert(
        &mut self,
        terminal: TerminalIx,
        group: GroupIx,
        ctx: ContextKey,
        reachable: [bool; 4],
        rep: BestRefLite,
    ) {
        debug_assert!(
            (group as usize) < self.n_groups,
            "group {group} out of range (n_groups = {})",
            self.n_groups
        );
        assert!(
            !self.seen[terminal].contains(group),
            "d1 cohort disjointness violated: loop {group} appears in more than \
             one context at terminal {terminal} (best[lane] must pick ONE winner \
             per (terminal, loop))"
        );
        self.seen[terminal].set(group);
        let entry = self.cohorts[terminal]
            .entry(ctx)
            .or_insert_with(|| (GroupBitmap::new(), rep));
        entry.0.set(group);
        for (v, &r) in reachable.iter().enumerate() {
            if r {
                self.verdicts[terminal][v].set(group);
            }
        }
    }

    /// The number of interned (reached) terminals.
    pub(crate) fn n_terminals(&self) -> usize {
        self.keys.len()
    }

    /// Finalize: yield, per reached terminal, its cohorts + per-verdict
    /// reachable-loop bitmaps. Each loop appears in exactly ONE ctx cohort per
    /// terminal (the disjointness invariant), so no cross-cohort subtraction is
    /// needed.
    pub(crate) fn finalize(self) -> Vec<TerminalCohorts<'a>> {
        let mut out = Vec::with_capacity(self.keys.len());
        for ((key, cmap), vsets) in self.keys.into_iter().zip(self.cohorts).zip(self.verdicts) {
            let cohorts: Vec<(ContextKey, GroupBitmap, BestRefLite)> = cmap
                .into_iter()
                .map(|(ctx, (bm, rep))| (ctx, bm, rep))
                .collect();
            out.push(TerminalCohorts {
                key,
                cohorts,
                verdict_sets: vsets,
            });
        }
        out
    }
}

/// The `reachable_verdicts` of loop `g` at a terminal — the verdicts (in
/// [`TempVerdict`] declaration order) whose per-verdict bitmap holds `g`.
/// Mirrors the old `reachable_from_masks`.
pub(crate) fn reachable_verdicts_of(
    verdict_sets: &[GroupBitmap; 4],
    g: GroupIx,
) -> Vec<TempVerdict> {
    let mut out = Vec::new();
    for (i, v) in VERDICT_ORDER.iter().enumerate() {
        if verdict_sets[i].contains(g) {
            out.push(*v);
        }
    }
    out
}

// ===========================================================================
// Census (Hot-tier — zero cost when tracing is off)
// ===========================================================================

/// Emit the static-cost census from [`Liveness`] (Hot-tier only): `ΣNeed`,
/// `max_need_per_node`, `nodes_with_need`, and the derived static fact bounds
/// (`static_reach_facts = 6·nodes`, `static_value_facts = 18·ΣNeed`; 6 = 3
/// depths × 2 unc, 18 = 3 classes × 3 depths × 2 unc). Zero cost when tracing
/// is disabled.
pub(crate) fn emit_liveness_census(liveness: &Liveness, n_nodes: usize) {
    if !crate::engine::perf_trace::enabled(crate::engine::perf_trace::Detail::Hot) {
        return;
    }
    let sum_need: u64 = liveness.need.iter().map(|n| n.len() as u64).sum();
    let max_need: u64 = liveness.need.iter().map(|n| n.len()).max().unwrap_or(0) as u64;
    let nodes_with_need: u64 = liveness.need.iter().filter(|n| !n.is_empty()).count() as u64;
    let nodes = n_nodes as u64;
    crate::engine::perf_trace::instant_lazy("d1.cohort", "liveness_census", || {
        serde_json::json!({
            "sum_need": sum_need,
            "max_need_per_node": max_need,
            "nodes_with_need": nodes_with_need,
            "nodes": nodes,
            "static_reach_facts": 6 * nodes,
            "static_value_facts": 18 * sum_need,
        })
    });
}

/// Emit the post-finalize census (Hot-tier only): `total_cohorts` (Σ contexts
/// over all terminals) and `unique_reached_terminals`. Zero cost when tracing
/// is disabled.
pub(crate) fn emit_finalize_census(cohorts: &[TerminalCohorts]) {
    if !crate::engine::perf_trace::enabled(crate::engine::perf_trace::Detail::Hot) {
        return;
    }
    let total_cohorts: u64 = cohorts.iter().map(|t| t.cohorts.len() as u64).sum();
    let unique_reached_terminals = cohorts.len() as u64;
    crate::engine::perf_trace::instant_lazy("d1.cohort", "finalize_census", || {
        serde_json::json!({
            "total_cohorts": total_cohorts,
            "unique_reached_terminals": unique_reached_terminals,
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_bitmap_set_contains_iter_count() {
        let mut b = GroupBitmap::new();
        assert!(b.is_empty());
        assert_eq!(b.count(), 0);
        assert_eq!(b.iter().collect::<Vec<_>>(), Vec::<GroupIx>::new());

        b.set(0);
        b.set(63);
        b.set(64);
        b.set(200);
        assert!(!b.is_empty());
        assert_eq!(b.count(), 4);
        assert!(b.contains(0) && b.contains(63) && b.contains(64) && b.contains(200));
        assert!(!b.contains(1) && !b.contains(65) && !b.contains(199));
        // Ascending order across word boundaries.
        assert_eq!(b.iter().collect::<Vec<_>>(), vec![0, 63, 64, 200]);
    }

    #[test]
    fn group_bitmap_or_with_grows_and_unions() {
        let mut a = GroupBitmap::new();
        a.set(1);
        a.set(70);
        let mut c = GroupBitmap::new();
        c.set(70); // overlap
        c.set(130); // beyond a's length
        a.or_with(&c);
        assert_eq!(a.iter().collect::<Vec<_>>(), vec![1, 70, 130]);
        assert_eq!(a.count(), 3);
        // or_with an empty bitmap is a no-op.
        let empty = GroupBitmap::new();
        a.or_with(&empty);
        assert_eq!(a.count(), 3);
    }

    #[test]
    fn context_key_eq_hash_consistent() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let k1 = ContextKey {
            severity: "high",
            verdict: TempVerdict::Physical,
            depth_bucket: 2,
            unc: true,
        };
        let k2 = k1;
        let k3 = ContextKey {
            verdict: TempVerdict::Temporary,
            ..k1
        };
        assert_eq!(k1, k2);
        assert_ne!(k1, k3);
        let h = |k: &ContextKey| {
            let mut s = DefaultHasher::new();
            k.hash(&mut s);
            s.finish()
        };
        assert_eq!(h(&k1), h(&k2), "equal keys hash equal");

        // Usable as a HashMap key.
        let mut m: HashMap<ContextKey, u32> = HashMap::new();
        m.insert(k1, 10);
        *m.entry(k1).or_insert(0) += 5;
        m.insert(k3, 1);
        assert_eq!(m[&k1], 15);
        assert_eq!(m[&k3], 1);
    }

    #[test]
    fn sink_interns_terminals_and_decompresses() {
        let mut sink = TerminalSink::new(2, 8);
        let t0 = sink.terminal_ix(("R", "R/op0"));
        let t0_again = sink.terminal_ix(("R", "R/op0"));
        let t1 = sink.terminal_ix(("S", "S/op0"));
        assert_eq!(t0, t0_again, "same key interns to the same slot");
        assert_ne!(t0, t1);

        let ctx_a = ContextKey {
            severity: "high",
            verdict: TempVerdict::Physical,
            depth_bucket: 1,
            unc: false,
        };
        let ctx_b = ContextKey {
            severity: "medium",
            verdict: TempVerdict::Temporary,
            depth_bucket: 0,
            unc: true,
        };
        let rep = BestRefLite { hops: 3 };
        // t0: loop 0 -> ctx_a (physical reaches; temporary also reaches),
        //     loop 1 -> ctx_b (temporary only).
        sink.insert(t0, 0, ctx_a, [true, true, false, false], rep);
        sink.insert(t0, 1, ctx_b, [true, false, false, false], rep);
        // t1: loop 2 -> ctx_a.
        sink.insert(t1, 2, ctx_a, [false, true, false, false], rep);

        let finalized = sink.finalize();
        // Decompress into (group, key) -> (verdict, depth, unc, reachable).
        type Row = (TempVerdict, i64, bool, Vec<TempVerdict>);
        let mut got: HashMap<(GroupIx, &str, &str), Row> = HashMap::new();
        for tc in &finalized {
            for (ctx, bm, _rep) in &tc.cohorts {
                for g in bm.iter() {
                    let reachable = reachable_verdicts_of(&tc.verdict_sets, g);
                    let prev = got.insert(
                        (g, tc.key.0, tc.key.1),
                        (ctx.verdict, ctx.depth_bucket, ctx.unc, reachable),
                    );
                    assert!(prev.is_none(), "each (loop, terminal) decompresses once");
                }
            }
        }
        assert_eq!(
            got[&(0, "R", "R/op0")],
            (
                TempVerdict::Physical,
                1,
                false,
                vec![TempVerdict::Temporary, TempVerdict::Physical]
            )
        );
        assert_eq!(
            got[&(1, "R", "R/op0")],
            (
                TempVerdict::Temporary,
                0,
                true,
                vec![TempVerdict::Temporary]
            )
        );
        assert_eq!(
            got[&(2, "S", "S/op0")],
            (TempVerdict::Physical, 1, false, vec![TempVerdict::Physical])
        );
        assert_eq!(got.len(), 3);
    }

    #[test]
    #[should_panic(expected = "disjointness")]
    fn sink_asserts_disjointness() {
        let mut sink = TerminalSink::new(1, 4);
        let t = sink.terminal_ix(("R", "R/op0"));
        let ctx_a = ContextKey {
            severity: "high",
            verdict: TempVerdict::Physical,
            depth_bucket: 1,
            unc: false,
        };
        let ctx_b = ContextKey {
            severity: "low",
            verdict: TempVerdict::Temporary,
            depth_bucket: 0,
            unc: false,
        };
        let rep = BestRefLite { hops: 0 };
        sink.insert(t, 0, ctx_a, [false, true, false, false], rep);
        // Same (terminal, loop) in a SECOND context — must panic.
        sink.insert(t, 0, ctx_b, [true, false, false, false], rep);
    }
}
