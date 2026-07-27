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

use serde::{Deserialize, Serialize};

use crate::engine::l3::l3_workspace::{L3RecordOperation, L3Routine};
use crate::engine::l4::summary::{Uncertainty, uncertainty_key};
use crate::engine::l5::d1_liveness::Liveness;
use crate::engine::l5::d1_witness::WitnessSummary;
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
#[derive(Default, Clone)]
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

    /// AND `other`'s bits into `self` IN PLACE — keep only bits set in BOTH
    /// (Task C9: the bitmap-partition replacement for the finest-cohort
    /// per-loop `by_rv` scan — see `d1.rs`'s finest-cohort assembly). A bit
    /// implicitly absent past a bitmap's word length is 0, so AND never needs
    /// to GROW `self`: any word of `self` past `other`'s length is zeroed
    /// (nothing there can be set in `other`), and `self` never gains bits it
    /// didn't already have.
    pub(crate) fn and_with(&mut self, other: &GroupBitmap) {
        if let GroupBitmap::Dense(words) = self {
            let ow = other.words();
            for (i, w) in words.iter_mut().enumerate() {
                let ov = if i < ow.len() { ow[i] } else { 0 };
                *w &= ov;
            }
        }
        // `GroupBitmap::Empty AND anything` stays `Empty` — a no-op.
    }

    /// AND-NOT `other`'s bits OUT of `self` IN PLACE — clear every bit
    /// `other` has set, keeping everything else. Never grows `self` (clearing
    /// bits can only shrink the set).
    pub(crate) fn and_not(&mut self, other: &GroupBitmap) {
        if let GroupBitmap::Dense(words) = self {
            let ow = other.words();
            for (i, w) in words.iter_mut().enumerate() {
                if i < ow.len() {
                    *w &= !ow[i];
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
// LoopSetId / LoopSetRegistry — hash-consed GroupBitmap interning (Task C4,
// `.superpowers/sdd/task-c4-brief.md`)
// ===========================================================================

/// An interned [`GroupBitmap`] handle — hash-consed by [`LoopSetRegistry::intern`].
/// Two structurally-identical bitmaps (the same set bits, built independently —
/// e.g. by two different terminals whose cohorts happen to be realized by the
/// exact same loops) always intern to the SAME id, so the compressed report's
/// [`crate::engine::l5::finding::D1CohortContext::loop_set`] handles reference ONE
/// shared bitmap instead of one copy per cohort. `#[serde(transparent)]` so the
/// STABLE form (`StableD1CohortContext::loop_set`) serializes as a bare integer —
/// an opaque, run-scoped index a consumer resolves via the accompanying
/// [`StableLoopSetRegistry`] + loop catalog, not a semantically stable id of its
/// own (unlike routine/table/object ids elsewhere in this file's projection).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LoopSetId(pub u32);

/// Hash-consed [`GroupBitmap`] registry — the run-level de-duplicated store a
/// compressed d1 report's `D1CohortContext::loop_set` handles index into (Task C4).
/// `intern` hash-conses by bitmap CONTENT (its trimmed word sequence): many
/// `(terminal, ContextKey)` cohorts across a run realize the exact same
/// reaching-loop set (e.g. every terminal directly inside one loop with no nested
/// calls shares that loop's singleton set), so collapsing identical bitmaps to one
/// shared id is the compressed report's second big memory win — alongside the
/// terminal-cohort collapse (Task C1) and the bounded representative witness
/// (Task C3). `get`/`iter` decompress an id back to its loop-group indices; `len`
/// interned sets total in the (small) hundreds to low thousands on real corpora,
/// nowhere near the 3.2M per-`(loop, terminal)` population this redesign replaces.
#[derive(Debug, Clone, Default)]
pub struct LoopSetRegistry {
    /// Canonical (trimmed, no trailing all-zero word) word sequence per interned
    /// id — `sets[id.0]` is `id`'s bitmap. Positional: `to_stable`/
    /// `StableLoopSetRegistry::to_registry` rely on `Vec` index == `LoopSetId.0`.
    sets: Vec<Box<[u64]>>,
    /// Content -> id, for hash-consing. Duplicates `sets`' bytes (the standard
    /// interner tradeoff — see `string-interner`, already a crate dependency for
    /// the same reason) in exchange for O(1) intern lookups.
    index: HashMap<Box<[u64]>, LoopSetId>,
}

impl LoopSetRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// `bitmap`'s word slice, trimmed of any trailing (highest-index) all-zero
    /// words, so two bitmaps that differ only in over-allocated trailing zero
    /// capacity intern to the SAME id. (`GroupBitmap` built via `set`/`or_with`
    /// alone never actually produces a trailing zero word — `set`'s `ensure_words`
    /// only ever grows to fit the bit it is about to set — so this is a defensive
    /// normalization, not a case reachable through the public `GroupBitmap` API
    /// today; kept because a hash-consing key must never trust an invariant it
    /// cannot itself verify.)
    fn canonical_words(bitmap: &GroupBitmap) -> &[u64] {
        let words = bitmap.words();
        match words.iter().rposition(|&w| w != 0) {
            Some(ix) => &words[..=ix],
            None => &[],
        }
    }

    /// Intern `bitmap`, hash-consing by content: an identical bitmap (same set
    /// bits) built independently returns the SAME [`LoopSetId`]; a different
    /// bitmap gets a new one. `pub(crate)` (not `pub`) because it takes a
    /// [`GroupBitmap`] by reference, and `GroupBitmap` itself is `pub(crate)` —
    /// matches its own visibility rather than the registry's (which is `pub`
    /// so a genuinely-external-reachable consumer like
    /// `finding::decompress_cohort_context` can still hold `&LoopSetRegistry`).
    pub(crate) fn intern(&mut self, bitmap: &GroupBitmap) -> LoopSetId {
        let key = Self::canonical_words(bitmap);
        if let Some(&id) = self.index.get(key) {
            return id;
        }
        let boxed: Box<[u64]> = key.to_vec().into_boxed_slice();
        let id = LoopSetId(self.sets.len() as u32);
        self.sets.push(boxed.clone());
        self.index.insert(boxed, id);
        id
    }

    /// Decompress `id` back to its interned [`GroupBitmap`] (a fresh owned copy —
    /// `GroupBitmap` has no borrowed variant to hand back a reference as one).
    /// `pub(crate)` — see [`Self::intern`]'s doc for why.
    pub(crate) fn get(&self, id: LoopSetId) -> GroupBitmap {
        let words = &self.sets[id.0 as usize];
        if words.is_empty() {
            GroupBitmap::Empty
        } else {
            GroupBitmap::Dense(words.clone())
        }
    }

    /// Iterate `id`'s loop-group indices directly, ascending — avoids
    /// materializing a [`GroupBitmap`] when the caller only wants the indices
    /// (the common case: rendering a cohort's loop list).
    pub fn iter(&self, id: LoopSetId) -> impl Iterator<Item = GroupIx> + '_ {
        let words: &[u64] = &self.sets[id.0 as usize];
        words.iter().enumerate().flat_map(|(wi, &word)| WordBits {
            word,
            base: (wi as u32) * 64,
        })
    }

    /// The number of loops `id`'s bitmap names (mirrors `GroupBitmap::count`).
    pub fn count(&self, id: LoopSetId) -> u64 {
        self.sets[id.0 as usize]
            .iter()
            .map(|w| w.count_ones() as u64)
            .sum()
    }

    /// The number of DISTINCT interned sets.
    pub fn len(&self) -> usize {
        self.sets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }

    /// Project to the STABLE (serialized) form: `sets[id.0]` is `id`'s
    /// decompressed loop-group indices, ascending — positional by `LoopSetId`.
    pub fn to_stable(&self) -> StableLoopSetRegistry {
        StableLoopSetRegistry {
            sets: (0..self.sets.len() as u32)
                .map(|ix| self.iter(LoopSetId(ix)).collect())
                .collect(),
        }
    }
}

/// [`LoopSetRegistry`] — STABLE (serialized) form: `sets[id.0]` is `id`'s
/// decompressed loop-group indices (ascending), a plain `Vec<Vec<GroupIx>>`
/// positional by [`LoopSetId`]. `to_registry` rebuilds an interning-equivalent
/// registry — SAME id for SAME position — the round-trip this compressed
/// report's JSON serialization relies on.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StableLoopSetRegistry {
    pub sets: Vec<Vec<GroupIx>>,
}

impl StableLoopSetRegistry {
    /// Rebuild a [`LoopSetRegistry`] whose `LoopSetId`s match this stable form's
    /// positions exactly (`sets[i]` decompresses under id `LoopSetId(i)`), built
    /// directly from the positional data rather than by re-`intern`-ing (which
    /// would silently COLLAPSE two equal-content entries at different positions —
    /// impossible for a registry honestly produced by `to_stable`, but this stays
    /// a faithful 1:1 rebuild of whatever JSON it is actually given).
    pub fn to_registry(&self) -> LoopSetRegistry {
        let mut reg = LoopSetRegistry::default();
        for indices in &self.sets {
            let mut bm = GroupBitmap::new();
            for &g in indices {
                bm.set(g);
            }
            let words: Box<[u64]> = LoopSetRegistry::canonical_words(&bm)
                .to_vec()
                .into_boxed_slice();
            let id = LoopSetId(reg.sets.len() as u32);
            reg.sets.push(words.clone());
            reg.index.insert(words, id);
        }
        reg
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

// ===========================================================================
// UncertaintyId / UncertaintyTable — run-level Uncertainty interning
// ===========================================================================

/// An interned [`Uncertainty`] handle — assigned first-seen by
/// [`UncertaintyTable::intern`]. Run-scoped and opaque: it is an index into the
/// table that produced it and has no meaning outside that table (and never
/// escapes into any output — the d1 report projects `kind`/`at` back to text at
/// `detectors::d1`'s confidence build).
///
/// The field is PRIVATE, unlike [`LoopSetId`]'s. That is a deliberate divergence
/// from the sibling convention rather than an oversight: a `LoopSetId` is always
/// serialized alongside its [`StableLoopSetRegistry`], so the data model itself
/// pairs an id with its table and `LoopSetId(0)` is a legitimate thing for
/// deserialization to construct. Nothing pairs an `UncertaintyId` with its
/// `UncertaintyTable` — they travel as two independent fields of `D1CohortRun`
/// — so a mis-paired table would resolve IN-RANGE ids to DIFFERENT uncertainties:
/// wrong evidence-note text with the right count, right level and right
/// `cappedBy`. [`UncertaintyTable::intern`] is therefore the ONLY thing anywhere
/// that can mint one, which makes "this id came from some other table" the sole
/// remaining way to get it wrong, and [`UncertaintyTable::resolve`] names that
/// hypothesis when the index is out of range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct UncertaintyId(u32);

/// The run-level de-duplicated [`Uncertainty`] store every [`CohortRep`]'s
/// uncertainty union indexes into.
///
/// **Why.** A cohort's union is the uncertainties of every node on its
/// representative path, and those are drawn from a tiny closed vocabulary:
/// measured on Base App 8020 (2026-07-27, heap census over the `pub`
/// `DetectorOutput`), the ~3.7M records in `ctx.uncertainties_by_node` collapse
/// to 3,073 distinct `"{kind} at {at}"` notes across 7,418,849 retained evidence
/// records. Storing the values per cohort meant ~8-10M owned `Uncertainty`
/// records — each 120 B of struct plus 2-3 `String`s — alive for the whole run
/// inside [`TerminalSink`], which was the single largest contributor to `alsem
/// analyze`'s peak RSS. Storing 4-byte ids into one shared table keeps EXACTLY
/// the same values in EXACTLY the same per-cohort order; only the ownership
/// changes.
///
/// Interning is by the FULL value (all five fields), not by the `(kind, at)`
/// pair the confidence mapper happens to read, so `id ↔ Uncertainty` is a
/// bijection and no field is silently dropped by the representation.
///
/// Mirrors the [`LoopSetRegistry`] convention already in this module: a newtype
/// id, a positional `Vec` of values, and a content→id map that duplicates the
/// values' bytes in exchange for O(1) interning (the standard interner tradeoff
/// — negligible here, at ~3k entries).
#[derive(Debug, Clone, Default)]
pub(crate) struct UncertaintyTable {
    /// Value per interned id — `entries[id.0]` is `id`'s uncertainty.
    entries: Vec<Uncertainty>,
    /// `uncertainty_key(entries[i])`, precomputed at intern time: [`Self::dedupe`]
    /// needs it once per id per cohort, and recomputing the `format!` there would
    /// reintroduce exactly the per-record allocation this table exists to remove.
    keys: Vec<Box<str>>,
    /// Value -> id, for interning.
    index: HashMap<Uncertainty, UncertaintyId>,
}

impl UncertaintyTable {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Intern `u`, returning its id — equal values always return the SAME id, a
    /// new value gets the next one.
    pub(crate) fn intern(&mut self, u: &Uncertainty) -> UncertaintyId {
        if let Some(&id) = self.index.get(u) {
            return id;
        }
        // `try_from`, not `as`: past `u32::MAX` distinct values an `as` cast
        // WRAPS, so a new value would silently alias an existing id — a wrong
        // answer rather than a crash. Unreachable in practice (3,150 distinct on
        // Base App 8020, bounded by the distinct values in
        // `ctx.uncertainties_by_node`), and this runs only on the miss path, so
        // making the impossible loud costs nothing.
        let id = UncertaintyId(
            u32::try_from(self.entries.len())
                .expect("UncertaintyTable exceeded u32::MAX distinct uncertainties"),
        );
        self.entries.push(u.clone());
        self.keys.push(uncertainty_key(u).into_boxed_str());
        self.index.insert(u.clone(), id);
        id
    }

    /// `id`'s index into this table's positional `Vec`s, checked.
    ///
    /// Every [`UncertaintyId`] in existence was minted by THIS type's
    /// [`Self::intern`] (the newtype's field is private — see its doc), so an
    /// out-of-range index has exactly one cause worth naming: the id came from a
    /// DIFFERENT `UncertaintyTable` than the one being resolved against. The
    /// bare `self.entries[..]` this replaces panicked too, but with a bare
    /// index-out-of-bounds that told the next reader nothing about which
    /// invariant broke. Note this can only catch the OUT-OF-RANGE case; a
    /// mis-paired table whose id happens to be in range still resolves silently
    /// to the wrong value, which is why `finalize` hands the table and the ids
    /// out together.
    fn resolve(&self, id: UncertaintyId) -> usize {
        let ix = id.0 as usize;
        assert!(
            ix < self.entries.len(),
            "UncertaintyId({ix}) is out of range for an UncertaintyTable of {} entries \
             — the id was almost certainly minted by a DIFFERENT table",
            self.entries.len()
        );
        ix
    }

    /// The [`Uncertainty`] `id` names.
    pub(crate) fn get(&self, id: UncertaintyId) -> &Uncertainty {
        &self.entries[self.resolve(id)]
    }

    /// `uncertainty_key` of the [`Uncertainty`] `id` names.
    pub(crate) fn key(&self, id: UncertaintyId) -> &str {
        &self.keys[self.resolve(id)]
    }

    /// The number of DISTINCT interned uncertainties.
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// De-duplicate a concatenated id sequence EXACTLY as
    /// [`crate::engine::l4::summary::dedupe_uncertainties`] de-duplicates the
    /// values themselves: keyed by `uncertainty_key`, **last-write-wins** on a key
    /// collision, emitted in byte-sorted key order.
    ///
    /// This is the one place the id substitution has to be argued rather than
    /// merely asserted, so, explicitly: two ids collide here iff their VALUES
    /// have the same `uncertainty_key`, which is the same condition under which
    /// the value-keyed `BTreeMap<String, Uncertainty>` collided; the surviving id
    /// is the last-inserted, as `Map.set`/`BTreeMap::insert` are; and `&str`'s
    /// `Ord` is byte order, as `String`'s is. So the emitted sequence is
    /// positionally identical to `dedupe_uncertainties`' on the same input.
    pub(crate) fn dedupe(&self, ids: &[UncertaintyId]) -> Vec<UncertaintyId> {
        let mut seen: std::collections::BTreeMap<&str, UncertaintyId> =
            std::collections::BTreeMap::new();
        for &id in ids {
            seen.insert(self.key(id), id); // last-write-wins, matching dedupe_uncertainties
        }
        seen.into_values().collect()
    }
}

/// The representative evidence for a `(terminal, ContextKey)` cohort — built
/// ONCE, FIRST-SEEN (Task C6 cutover), while the per-batch fact arena is still
/// alive (the arena-lifetime constraint: `score_batch_to_sink` drops its
/// `BatchSolver` per batch, so the witness must be materialized at insert time).
/// Carries the bounded representative [`WitnessSummary`] (Task C3) AND the
/// uncertainty union along that representative path — the latter drives the
/// finding-level confidence, which the OLD per-loop path derived from the
/// winner's own path uncertainties (so computing it along the first-seen
/// representative, which for a cohort IS the lowest-group-index — hence the OLD
/// winner-selection's `loop_routine_id`/`loop_id`-min — reaching loop, preserves
/// the finding's confidence exactly).
///
/// The union is stored as [`UncertaintyId`]s into the run-level
/// [`UncertaintyTable`] the owning [`TerminalSink`] holds — same uncertainties,
/// same order, 4 bytes per entry instead of an owned record. Resolve them with
/// [`UncertaintyTable::get`].
#[derive(Debug, Clone)]
pub(crate) struct CohortRep {
    pub witness: WitnessSummary,
    pub uncertainties: Vec<UncertaintyId>,
}

// ===========================================================================
// TerminalSink — run-global: Terminal -> ContextKey -> loop-bitmap
// ===========================================================================

/// One finalized terminal: the OWNING routine + db op it fires (the actual graph
/// references, NOT re-derived from ids — a colliding-id sibling routine may lack
/// the op, which is exactly the G-18 hazard that would drop the finding), its
/// context cohorts (each a `ContextKey` + the loops realizing it + the
/// representative), and the per-verdict reachable-loop bitmaps
/// (`verdict_sets[verdict as usize]`).
pub(crate) struct TerminalCohorts<'a> {
    pub key: (&'a L3Routine, &'a L3RecordOperation),
    pub cohorts: Vec<(ContextKey, GroupBitmap, CohortRep)>,
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
    /// Intern by `(owner_id, op_id)` (dedups colliding terminals to one slot,
    /// mirroring the old `assemble_findings` group-by-id) — but the STORED
    /// identity (`terminals`) is the first-seen routine+op REFERENCE, so the
    /// finding builder never re-derives the op from a colliding-id sibling.
    ix_of: HashMap<(&'a str, &'a str), TerminalIx>,
    terminals: Vec<(&'a L3Routine, &'a L3RecordOperation)>,
    /// Per terminal: `ContextKey -> (loop cohort, first-seen representative)`.
    cohorts: Vec<HashMap<ContextKey, (GroupBitmap, CohortRep)>>,
    /// Per terminal: per-verdict (indexed `verdict as usize`) reaching loops.
    verdicts: Vec<[GroupBitmap; 4]>,
    /// Per terminal: every loop already inserted (any ctx) — the disjointness
    /// guard (a loop must appear in ≤1 ctx per terminal).
    seen: Vec<GroupBitmap>,
    /// The run-level interned uncertainty store every [`CohortRep`] in this sink
    /// indexes into. Owned HERE rather than by the caller so [`Self::insert`] can
    /// hand `&mut` to it straight to the `build_rep` closure (disjoint field
    /// borrows), which keeps `score_batch_to_sink`/`sink_emit` free of a second
    /// threaded parameter. Moved out by [`Self::finalize`] to live on the run
    /// alongside the cohorts that reference it.
    uncertainties: UncertaintyTable,
}

impl<'a> TerminalSink<'a> {
    /// `n_terminals` is a capacity hint (the terminal plan's terminal count);
    /// `n_groups` bounds the group-index universe (for the range check).
    pub(crate) fn new(n_terminals: usize, n_groups: usize) -> Self {
        TerminalSink {
            n_groups,
            ix_of: HashMap::with_capacity(n_terminals),
            terminals: Vec::with_capacity(n_terminals),
            cohorts: Vec::with_capacity(n_terminals),
            verdicts: Vec::with_capacity(n_terminals),
            seen: Vec::with_capacity(n_terminals),
            uncertainties: UncertaintyTable::new(),
        }
    }

    /// Intern a terminal (owning routine + db op) to its dense [`TerminalIx`],
    /// stable across batches (the same `(owner_id, op_id)` always maps to the same
    /// slot). The first-seen `(owner, op)` REFERENCE is stored — see the struct's
    /// `ix_of`/`terminals` doc for why the reference, not the id, is kept.
    pub(crate) fn terminal_ix(
        &mut self,
        owner: &'a L3Routine,
        op: &'a L3RecordOperation,
    ) -> TerminalIx {
        let key = (owner.id.as_str(), op.id.as_str());
        if let Some(&ix) = self.ix_of.get(&key) {
            return ix;
        }
        let ix = self.terminals.len();
        self.ix_of.insert(key, ix);
        self.terminals.push((owner, op));
        self.cohorts.push(HashMap::new());
        self.verdicts
            .push(std::array::from_fn(|_| GroupBitmap::new()));
        self.seen.push(GroupBitmap::new());
        ix
    }

    /// Record loop `group`'s winner at `terminal`: set its bit in the `ctx`
    /// cohort and set it in each reaching verdict's bitmap. Asserts the
    /// disjointness invariant.
    ///
    /// `build_rep` is called AT MOST ONCE per `(terminal, ContextKey)` cohort —
    /// only on the FIRST loop that lands in it (`or_insert_with`), never on a
    /// later loop joining an existing cohort. That is the whole cohort-redesign
    /// win: exactly one representative witness is materialized per cohort
    /// (~34,861 total), not one per `(loop, terminal)` (3.2M). The closure is
    /// where the caller ([`crate::engine::l5::d1_dataflow::sink_emit`]) builds
    /// the witness from the STILL-ALIVE per-batch `BatchSolver` — the arena is
    /// dropped after the batch, so the witness cannot be deferred to finalize.
    ///
    /// `build_rep` receives this sink's [`UncertaintyTable`] to intern the
    /// representative path's uncertainty union into; it is the sink's own field,
    /// borrowed disjointly from `cohorts`.
    pub(crate) fn insert(
        &mut self,
        terminal: TerminalIx,
        group: GroupIx,
        ctx: ContextKey,
        reachable: [bool; 4],
        build_rep: impl FnOnce(&mut UncertaintyTable) -> CohortRep,
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
        let table = &mut self.uncertainties;
        let entry = self.cohorts[terminal]
            .entry(ctx)
            .or_insert_with(|| (GroupBitmap::new(), build_rep(table)));
        entry.0.set(group);
        for (v, &r) in reachable.iter().enumerate() {
            if r {
                self.verdicts[terminal][v].set(group);
            }
        }
    }

    /// The number of interned (reached) terminals.
    pub(crate) fn n_terminals(&self) -> usize {
        self.terminals.len()
    }

    /// Finalize: yield, per reached terminal, its cohorts + per-verdict
    /// reachable-loop bitmaps, PLUS the run-level [`UncertaintyTable`] every
    /// `CohortRep.uncertainties` id sequence indexes into (moved out, not cloned —
    /// the cohorts are meaningless without it, so the two travel together). Each
    /// loop appears in exactly ONE ctx cohort per terminal (the disjointness
    /// invariant), so no cross-cohort subtraction is needed.
    pub(crate) fn finalize(self) -> (Vec<TerminalCohorts<'a>>, UncertaintyTable) {
        let mut out = Vec::with_capacity(self.terminals.len());
        for ((key, cmap), vsets) in self
            .terminals
            .into_iter()
            .zip(self.cohorts)
            .zip(self.verdicts)
        {
            let cohorts: Vec<(ContextKey, GroupBitmap, CohortRep)> = cmap
                .into_iter()
                .map(|(ctx, (bm, rep))| (ctx, bm, rep))
                .collect();
            out.push(TerminalCohorts {
                key,
                cohorts,
                verdict_sets: vsets,
            });
        }
        (out, self.uncertainties)
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
/// over all terminals), `unique_reached_terminals`, and the interned-uncertainty
/// population — `uncertainty_ids` (Σ `CohortRep.uncertainties.len()`, the record
/// count that USED to be one owned `Uncertainty` each) against
/// `distinct_uncertainties` (the table's size). That pair is the DIRECT
/// measurement of a population that could previously only be derived (the
/// pre-interning scoping estimate was a ~8.3-10.7M-record band, inferred from
/// cohort counts × mean path length because nothing counted the records
/// themselves). First reading, Base App 8020, 2026-07-27: 10,266,162
/// `uncertainty_ids` over 3,150 `distinct_uncertainties` — 3,259x duplication,
/// inside the derived band. Zero cost when tracing is disabled.
pub(crate) fn emit_finalize_census(cohorts: &[TerminalCohorts], uncertainties: &UncertaintyTable) {
    if !crate::engine::perf_trace::enabled(crate::engine::perf_trace::Detail::Hot) {
        return;
    }
    let total_cohorts: u64 = cohorts.iter().map(|t| t.cohorts.len() as u64).sum();
    let unique_reached_terminals = cohorts.len() as u64;
    let uncertainty_ids: u64 = cohorts
        .iter()
        .flat_map(|t| t.cohorts.iter())
        .map(|(_ck, _bm, rep)| rep.uncertainties.len() as u64)
        .sum();
    let distinct_uncertainties = uncertainties.len() as u64;
    crate::engine::perf_trace::instant_lazy("d1.cohort", "finalize_census", || {
        serde_json::json!({
            "total_cohorts": total_cohorts,
            "unique_reached_terminals": unique_reached_terminals,
            "uncertainty_ids": uncertainty_ids,
            "distinct_uncertainties": distinct_uncertainties,
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::l4::summary::dedupe_uncertainties;
    use crate::engine::l5::finding::{EvidenceStep, SourceAnchor};

    fn u(
        kind: &str,
        callsite: Option<&str>,
        operation: Option<&str>,
        routine: Option<&str>,
        interface: Option<&str>,
    ) -> Uncertainty {
        Uncertainty {
            kind: kind.to_string(),
            callsite_id: callsite.map(str::to_string),
            operation_id: operation.map(str::to_string),
            routine_id: routine.map(str::to_string),
            interface_name: interface.map(str::to_string),
        }
    }

    /// **The substitution contract, as a differential against the oracle it
    /// replaces.** Interning + [`UncertaintyTable::dedupe`] + resolution must
    /// yield the byte-identical `Vec<Uncertainty>` that
    /// [`dedupe_uncertainties`] — still live, still the `#[cfg(test)]` walker
    /// path's own de-duplicator — produces from the same input.
    ///
    /// The input is chosen so each clause of the contract is load-bearing:
    /// - keys arrive OUT of sorted order (so a missing sort diverges);
    /// - one value is an EXACT duplicate (so a missing dedupe diverges);
    /// - two values share a `uncertainty_key` but differ in `interface_name`
    ///   (so keep-FIRST instead of keep-LAST diverges — the only observable
    ///   difference between the two, and the case
    ///   `dedupe_uncertainties`' own doc calls out).
    #[test]
    fn interned_dedupe_matches_dedupe_uncertainties() {
        let list = vec![
            u("unresolved-call", Some("r/cs2"), None, None, None),
            u("external-target", Some("r/cs1"), None, None, None),
            u(
                "interface-open-world",
                Some("r/cs0"),
                None,
                None,
                Some("IFirst"),
            ),
            u("unresolved-call", Some("r/cs2"), None, None, None), // exact duplicate
            u(
                "interface-open-world",
                Some("r/cs0"),
                None,
                None,
                Some("ISecond"),
            ), // same key, different value
            u("dynamic-dispatch", None, None, Some("R"), None),
            u("member-not-found", None, Some("r/op0"), None, None),
        ];

        let mut table = UncertaintyTable::new();
        let ids: Vec<UncertaintyId> = list.iter().map(|x| table.intern(x)).collect();
        let via_ids: Vec<Uncertainty> = table
            .dedupe(&ids)
            .into_iter()
            .map(|id| table.get(id).clone())
            .collect();

        assert_eq!(
            via_ids,
            dedupe_uncertainties(list.clone()),
            "the id path must reproduce dedupe_uncertainties exactly"
        );
        // Non-vacuity + the specific clauses, so a both-sides-empty result cannot
        // pass and the keep-LAST tie-break is asserted directly.
        assert_eq!(via_ids.len(), 5, "7 in, 5 distinct keys out");
        assert_eq!(via_ids[0].kind, "dynamic-dispatch", "sorted by key");
        assert_eq!(
            via_ids[2].interface_name.as_deref(),
            Some("ISecond"),
            "same-key collision keeps the LAST value"
        );
        // Interning is by FULL VALUE, not by key: the two `interface-open-world`
        // entries share a key but are distinct uncertainties, so the table holds
        // 6 of the 7 inputs (only the exact duplicate collapses).
        assert_eq!(table.len(), 6, "interned by value, not by uncertainty_key");
    }

    /// Resolving an id against the WRONG [`UncertaintyTable`] must name that
    /// hypothesis, not just index out of bounds.
    ///
    /// This pins the USE (`get`, the one production resolution site's own
    /// entry point) rather than `resolve` itself, and it pins the MESSAGE: the
    /// whole value of the check is that the next reader is told which invariant
    /// broke, so a panic with the old bare `index out of bounds` text would be
    /// no better than what it replaced. `#[should_panic(expected = ...)]`
    /// substring-matches, so this fails if the "DIFFERENT table" wording is
    /// dropped.
    #[test]
    #[should_panic(expected = "minted by a DIFFERENT table")]
    fn resolving_an_id_from_another_table_names_the_invariant() {
        let mut minted_from = UncertaintyTable::new();
        minted_from.intern(&u("unresolved-call", Some("r/cs0"), None, None, None));
        let id = minted_from.intern(&u("opaque-callee", Some("r/cs1"), None, None, None));

        // A DIFFERENT table that never saw the second value — `id` is in range
        // for `minted_from` but not for this one.
        let mut other = UncertaintyTable::new();
        other.intern(&u("unresolved-call", Some("r/cs0"), None, None, None));
        assert_eq!(other.len(), 1, "the wrong table is genuinely shorter");

        other.get(id);
    }

    /// A throwaway [`CohortRep`] for the sink unit tests — its contents are never
    /// inspected here (the differential in `d1_dataflow` proves the real
    /// witness/uncertainty build); these tests only exercise the bitmap-cohort
    /// bookkeeping (interning, disjointness, verdict decompression).
    fn dummy_rep(_table: &mut UncertaintyTable) -> CohortRep {
        let step = EvidenceStep {
            routine_id: "R".to_string(),
            operation_id: None,
            callsite_id: None,
            loop_id: None,
            source_anchor: SourceAnchor {
                source_unit_id: String::new(),
                start_line: 0,
                start_column: 0,
                end_line: 0,
                end_column: 0,
                enclosing_routine_id: "R".to_string(),
                syntax_kind: String::new(),
                normalized_text_hash: None,
                leading_context_hash: None,
                trailing_context_hash: None,
            },
            note: String::new(),
        };
        CohortRep {
            witness: WitnessSummary {
                total_hops: 0,
                first_steps: vec![step.clone()],
                omitted_hops: 0,
                last_steps: vec![],
                terminal_step: step,
            },
            uncertainties: vec![],
        }
    }

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
    fn group_bitmap_and_with_intersects_without_growing() {
        let mut a = GroupBitmap::new();
        a.set(1);
        a.set(70);
        a.set(130);
        let mut b = GroupBitmap::new();
        b.set(70); // overlap
        b.set(200); // beyond a's current length — must NOT grow a
        a.and_with(&b);
        assert_eq!(a.iter().collect::<Vec<_>>(), vec![70]);
        assert_eq!(a.count(), 1);
        assert!(!a.contains(200), "and_with must never grow self");

        // AND with an empty bitmap clears everything (nothing is in `Empty`).
        let mut c = GroupBitmap::new();
        c.set(5);
        c.and_with(&GroupBitmap::new());
        assert!(c.is_empty());

        // Empty AND anything stays Empty (no-op, no panic).
        let mut e = GroupBitmap::new();
        e.and_with(&b);
        assert!(e.is_empty());
    }

    #[test]
    fn group_bitmap_and_not_clears_shared_bits_only() {
        let mut a = GroupBitmap::new();
        a.set(1);
        a.set(70);
        a.set(130);
        let mut b = GroupBitmap::new();
        b.set(70); // shared — must be cleared
        b.set(200); // not in a — irrelevant
        a.and_not(&b);
        assert_eq!(a.iter().collect::<Vec<_>>(), vec![1, 130]);
        assert_eq!(a.count(), 2);

        // and_not an empty bitmap is a no-op.
        let mut c = GroupBitmap::new();
        c.set(9);
        c.and_not(&GroupBitmap::new());
        assert_eq!(c.iter().collect::<Vec<_>>(), vec![9]);

        // Empty and_not anything stays Empty.
        let mut e = GroupBitmap::new();
        e.and_not(&b);
        assert!(e.is_empty());
    }

    #[test]
    fn group_bitmap_and_with_and_not_partition_like_old_per_loop_scan() {
        // Mirrors the Task C9 partition shape: a cohort bitmap `bm` split by
        // membership in a `verdict` bitmap using AND / AND-NOT must equal
        // filtering `bm.iter()` by `verdict.contains(g)` directly.
        let mut bm = GroupBitmap::new();
        for g in [0u32, 1, 64, 65, 200] {
            bm.set(g);
        }
        let mut verdict = GroupBitmap::new();
        for g in [1u32, 64, 999] {
            verdict.set(g);
        }

        let mut reaches = bm.clone();
        reaches.and_with(&verdict);
        let mut not_reaches = bm.clone();
        not_reaches.and_not(&verdict);

        let expected_reaches: Vec<GroupIx> = bm.iter().filter(|g| verdict.contains(*g)).collect();
        let expected_not_reaches: Vec<GroupIx> =
            bm.iter().filter(|g| !verdict.contains(*g)).collect();

        assert_eq!(reaches.iter().collect::<Vec<_>>(), expected_reaches);
        assert_eq!(not_reaches.iter().collect::<Vec<_>>(), expected_not_reaches);
        // Partition covers `bm` exactly, disjointly.
        assert_eq!(reaches.count() + not_reaches.count(), bm.count());
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
        use crate::engine::l5::test_support::{record_op, routine};
        let r = routine("R", "procedure");
        let op_r = record_op("R/op0", "Modify", "Rec", None, vec![], false);
        let s = routine("S", "procedure");
        let op_s = record_op("S/op0", "Modify", "Rec", None, vec![], false);

        let mut sink = TerminalSink::new(2, 8);
        let t0 = sink.terminal_ix(&r, &op_r);
        let t0_again = sink.terminal_ix(&r, &op_r);
        let t1 = sink.terminal_ix(&s, &op_s);
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
        // t0: loop 0 -> ctx_a (physical reaches; temporary also reaches),
        //     loop 1 -> ctx_b (temporary only).
        sink.insert(t0, 0, ctx_a, [true, true, false, false], dummy_rep);
        sink.insert(t0, 1, ctx_b, [true, false, false, false], dummy_rep);
        // t1: loop 2 -> ctx_a.
        sink.insert(t1, 2, ctx_a, [false, true, false, false], dummy_rep);

        let (finalized, _unc) = sink.finalize();
        // Decompress into (group, key) -> (verdict, depth, unc, reachable).
        type Row = (TempVerdict, i64, bool, Vec<TempVerdict>);
        let mut got: HashMap<(GroupIx, &str, &str), Row> = HashMap::new();
        for tc in &finalized {
            for (ctx, bm, _rep) in &tc.cohorts {
                for g in bm.iter() {
                    let reachable = reachable_verdicts_of(&tc.verdict_sets, g);
                    let prev = got.insert(
                        (g, tc.key.0.id.as_str(), tc.key.1.id.as_str()),
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
        use crate::engine::l5::test_support::{record_op, routine};
        let r = routine("R", "procedure");
        let op_r = record_op("R/op0", "Modify", "Rec", None, vec![], false);
        let mut sink = TerminalSink::new(1, 4);
        let t = sink.terminal_ix(&r, &op_r);
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
        sink.insert(t, 0, ctx_a, [false, true, false, false], dummy_rep);
        // Same (terminal, loop) in a SECOND context — must panic.
        sink.insert(t, 0, ctx_b, [true, false, false, false], dummy_rep);
    }

    // === LoopSetRegistry — hash-cons interning (Task C4) =====================

    #[test]
    fn loop_set_registry_interns_identical_bitmaps_to_same_id() {
        let mut reg = LoopSetRegistry::new();
        let mut a = GroupBitmap::new();
        a.set(0);
        a.set(5);
        a.set(64);
        let mut b = GroupBitmap::new();
        // Same content, built in a DIFFERENT insertion order and by a different
        // (independent) bitmap — hash-consing must key on content, not identity.
        b.set(64);
        b.set(0);
        b.set(5);

        let id_a = reg.intern(&a);
        let id_b = reg.intern(&b);
        assert_eq!(id_a, id_b, "identical bitmaps intern to the SAME id");

        let mut c = GroupBitmap::new();
        c.set(0);
        c.set(5);
        let id_c = reg.intern(&c);
        assert_ne!(id_a, id_c, "a different bitmap gets a DIFFERENT id");

        // Re-interning `a` again returns the already-assigned id (no growth).
        assert_eq!(reg.intern(&a), id_a);
        assert_eq!(reg.len(), 2);
    }

    #[test]
    fn loop_set_registry_decompresses_to_exact_group_set() {
        let mut reg = LoopSetRegistry::new();
        let mut a = GroupBitmap::new();
        a.set(0);
        a.set(5);
        a.set(64);
        a.set(200);
        let id_a = reg.intern(&a);

        assert_eq!(reg.iter(id_a).collect::<Vec<_>>(), vec![0, 5, 64, 200]);
        assert_eq!(reg.count(id_a), 4);
        assert_eq!(
            reg.get(id_a).iter().collect::<Vec<_>>(),
            vec![0, 5, 64, 200],
            "get(id) decompresses back to the exact original bitmap"
        );
    }

    #[test]
    fn loop_set_registry_empty_bitmaps_share_one_id() {
        let mut reg = LoopSetRegistry::new();
        let id1 = reg.intern(&GroupBitmap::new());
        let id2 = reg.intern(&GroupBitmap::new());
        assert_eq!(
            id1, id2,
            "two independently-built empty bitmaps intern equal"
        );
        assert_eq!(reg.iter(id1).collect::<Vec<_>>(), Vec::<GroupIx>::new());
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn loop_set_registry_stable_round_trip() {
        let mut reg = LoopSetRegistry::new();
        let mut a = GroupBitmap::new();
        a.set(1);
        a.set(2);
        a.set(130);
        let mut b = GroupBitmap::new();
        b.set(9);
        let id_a = reg.intern(&a);
        let id_b = reg.intern(&b);

        let stable = reg.to_stable();
        assert_eq!(stable.sets[id_a.0 as usize], vec![1, 2, 130]);
        assert_eq!(stable.sets[id_b.0 as usize], vec![9]);

        // JSON round trip of the stable form itself.
        let json = serde_json::to_string(&stable).unwrap();
        let back: StableLoopSetRegistry = serde_json::from_str(&json).unwrap();
        assert_eq!(stable, back);

        // Rebuilding via `to_registry` preserves the id<->content mapping: the
        // ORIGINAL bitmaps re-intern to the SAME ids against the rebuilt registry.
        let mut rebuilt = back.to_registry();
        assert_eq!(rebuilt.intern(&a), id_a);
        assert_eq!(rebuilt.intern(&b), id_b);
        assert_eq!(rebuilt.iter(id_a).collect::<Vec<_>>(), vec![1, 2, 130]);
    }

    #[test]
    fn loop_set_id_serializes_transparently() {
        let id = LoopSetId(42);
        assert_eq!(serde_json::to_string(&id).unwrap(), "42");
        let back: LoopSetId = serde_json::from_str("42").unwrap();
        assert_eq!(back, id);
    }
}
