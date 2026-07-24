//! Compact, SCC-shared db-effect store (spec Part A Steps 2-3 —
//! `docs/superpowers/specs/2026-07-22-l4-dbeffect-store-and-retirement-design.md`,
//! lines 118-211).
//!
//! ## What A3 delivers over A2
//!
//! A2 gave each routine its OWN terminal-set arena entry (no sharing) and
//! materialized a `Vec<DbEffect>` per member mid-solve for feed-forward. A3:
//!
//!   - **SCC-shared [`EffectSetId`]:** all members of one effective SCC record
//!     ONE hash-consed terminal set (`closed_form_union`'s `C`) — the 7.1M
//!     memberships of the motivation's 797-member SCC collapse to ~82k
//!     [`EffectSetId`] refs + tiny per-member `pd_delta`s. The `via` column
//!     stays per-membership (~7.1MB; it does NOT collapse with set-sharing).
//!   - **Feed-forward on ids, not Strings:** the solver reads a settled
//!     callee's terminal bits + PD ids straight from this builder (see
//!     [`SummaryBundleBuilder::terminal_bits`]/[`SummaryBundleBuilder::pd_ids`])
//!     — no `Vec<DbEffect>` (5 heap Strings each) is ever materialized for
//!     feed-forward, killing the ~40GB the motivation measures.
//!   - **`key_rank` output ordering:** EffectIds are NEVER remapped (storage
//!     stays EffectId-keyed, `word id/64`); only the projected OUTPUT order
//!     uses the frozen [`FrozenEffectUniverse`]'s `key_rank`. Each
//!     [`EffectSetId`] caches its members in `key_rank` order once
//!     ([`EffectStore::ordered_ids`]); the base∪delta emit is then an O(result)
//!     two-way merge of two already-ranked runs.
//!
//! ## The freeze boundary (⟨rev4⟩ availability window)
//!
//! The universe GROWS during solve (terminal emissions are discovered as SCCs
//! settle), but dense sets need a FROZEN length and hash-consed [`EffectSetId`]s
//! are built post-solve. So during solve the shared `C` bitsets live in a
//! growable arena keyed by a [`SetRef`] (read by reference — no clone, no String
//! re-intern); [`SummaryBundleBuilder::finish`] hash-conses that arena into the
//! [`EffectStore`] and rewrites every row's `terminal_base` `SetRef` →
//! [`EffectSetId`] AFTER the universe is frozen. `key_rank` is consulted ONLY
//! post-freeze (projection/canonicalization), never mid-solve.

use std::collections::HashMap;
use std::ops::Range;

use crate::engine::l4::db_effect_solver::kind_to_temp_state;
use crate::engine::l4::effect_lattice::TempStateKind;
use crate::engine::l4::effect_universe::{EffectId, FrozenEffectUniverse};
use crate::engine::l4::routine_interner::{RoutineInterner, RoutineIx};
use crate::engine::l4::summary::DbEffect;

// ---------------------------------------------------------------------------
// Bitset primitives — the ONE home for the `EffectId`-keyed presence bitset
// layout (bit `n % 64` of word `n / 64`). `db_effect_solver` imports these so
// the solve-time bitsets and the freeze-time hash-cons agree on layout by
// construction (DRY — one representation of "a set of EffectIds").
// ---------------------------------------------------------------------------

/// OR one interned [`EffectId`] into a presence bitset, growing the backing
/// `Vec<u64>` if the id's word isn't yet allocated.
pub(crate) fn set_bit(bits: &mut Vec<u64>, id: EffectId) {
    let word = (id.0 / 64) as usize;
    if bits.len() <= word {
        bits.resize(word + 1, 0);
    }
    bits[word] |= 1u64 << (id.0 % 64);
}

/// True iff `id`'s bit is set in `bits`. An id past the end of `bits` is
/// absent (never grows `bits` — read-only).
pub(crate) fn has_bit(bits: &[u64], id: EffectId) -> bool {
    let word = (id.0 / 64) as usize;
    word < bits.len() && (bits[word] & (1u64 << (id.0 % 64))) != 0
}

/// Bulk word-at-a-time OR of `src` into `dst`, growing `dst` if `src` is longer.
pub(crate) fn or_bits(dst: &mut Vec<u64>, src: &[u64]) {
    if dst.len() < src.len() {
        dst.resize(src.len(), 0);
    }
    for (d, s) in dst.iter_mut().zip(src.iter()) {
        *d |= s;
    }
}

/// Bit-scan iterator over the SET bits of one presence bitset, yielding
/// `EffectId`s in ascending order (storage order). Cost is proportional to the
/// number of set bits, not the universe length.
pub(crate) fn iter_set_bits(bits: &[u64]) -> impl Iterator<Item = EffectId> + '_ {
    bits.iter().enumerate().flat_map(|(word_idx, &word)| {
        let mut remaining = word;
        std::iter::from_fn(move || {
            if remaining == 0 {
                None
            } else {
                let bit = remaining.trailing_zeros();
                remaining &= remaining - 1;
                Some(EffectId((word_idx as u32) * 64 + bit))
            }
        })
    })
}

/// Total number of set bits (cardinality) of a presence bitset.
fn popcount(bits: &[u64]) -> u32 {
    bits.iter().map(|w| w.count_ones()).sum()
}

// ---------------------------------------------------------------------------
// ViaRank — the 5-rank via provenance, as a `u8` enum (spec Step 2).
// ---------------------------------------------------------------------------

/// The 5 canonical `via` provenance ranks (mirrors
/// `effect_lattice::merge_via`'s `VIA_RANK`: `direct=4 > implicit-trigger=3 >
/// event-subscriber=2 > dynamic=1 > inherited=0`), stored as `u8` instead of
/// an owned `String`. ⟨rev⟩ This does NOT collapse with A3's set-sharing — via
/// is per-membership provenance, not per-effect identity — so it stays a real
/// ~7.1MB column even after A3.
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
    /// `via_for_edge_kind`/`merge_via`/`"direct"` literal outputs.
    pub fn as_str(self) -> &'static str {
        match self {
            ViaRank::Direct => "direct",
            ViaRank::ImplicitTrigger => "implicit-trigger",
            ViaRank::EventSubscriber => "event-subscriber",
            ViaRank::Dynamic => "dynamic",
            ViaRank::Inherited => "inherited",
        }
    }

    /// Parse one of the 5 canonical via strings; any OTHER input defensively
    /// floors to `Inherited` (rank 0) — mirrors the OLD solver's `"inherited"`
    /// floor default — never panics (this repo's "engine never throws in
    /// production" rule).
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
// SetRef / EffectSetId / HybridEffectSet.
// ---------------------------------------------------------------------------

/// A handle into the [`SummaryBundleBuilder`]'s SOLVE-TIME terminal-set arena
/// — one entry per effective SCC (and one per fixed leaf), read by reference
/// during feed-forward. Canonicalized to an [`EffectSetId`] at
/// [`SummaryBundleBuilder::finish`]. Distinct from [`EffectSetId`]: many
/// `SetRef`s (one per effective SCC) may hash-cons to the SAME [`EffectSetId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetRef(pub u32);

/// A handle into the frozen, hash-consed [`EffectStore`]. All routines whose
/// terminal set is content-equal share ONE `EffectSetId` (spec Step 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EffectSetId(pub u32);

/// Below this cardinality a set stores its ids sparsely (`Box<[EffectId]>`);
/// at or above it, densely (a `Box<[u64]>` word array). ⟨rev⟩ At ~256 the
/// sparse form (256×4 = 1024 bytes) and a full-width dense form (~1.1KB for a
/// 9,137-effect universe) are comparable — above it the dense form wins.
const SPARSE_THRESHOLD: u32 = 256;

/// One hash-consed set of [`EffectId`]s, stored sparse below
/// [`SPARSE_THRESHOLD`] and dense above. Iteration ALWAYS yields ascending
/// EffectId (storage order) regardless of repr (spec Step 3 invariant); the
/// `key_rank` OUTPUT order is a SEPARATE cached `ordered_ids` array on the
/// store, never this type's concern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HybridEffectSet {
    /// Ascending `EffectId`s, `< SPARSE_THRESHOLD` of them.
    Sparse(Box<[EffectId]>),
    /// A word array sized `ceil(frozen_universe_len / 64)` (NEVER hardcoded),
    /// plus a sampled cumulative-popcount `rank_lut` for O(1) membership
    /// ordinal and the cached `cardinality`.
    Dense {
        words: Box<[u64]>,
        rank_lut: Box<[u32]>,
        cardinality: u32,
    },
}

/// `rank_lut` samples one cumulative popcount per this many words.
const RANK_LUT_BLOCK: usize = 8;

impl HybridEffectSet {
    /// Build from a presence bitset, choosing sparse/dense by cardinality.
    /// `frozen_len` is the universe length at freeze — dense `words` are sized
    /// `ceil(frozen_len / 64)` and tail bits past `frozen_len` are validated
    /// zero (a set never carries a bit for an id that does not exist).
    fn from_bits(bits: &[u64], frozen_len: usize) -> Self {
        let card = popcount(bits);
        if card < SPARSE_THRESHOLD {
            let ids: Vec<EffectId> = iter_set_bits(bits).collect();
            HybridEffectSet::Sparse(ids.into_boxed_slice())
        } else {
            let n_words = frozen_len.div_ceil(64);
            let mut words = vec![0u64; n_words];
            for (i, w) in bits.iter().enumerate() {
                if i < n_words {
                    words[i] = *w;
                } else {
                    debug_assert_eq!(*w, 0, "set carries a bit past the frozen universe length");
                }
            }
            // Validate tail bits in the last word past `frozen_len` are zero.
            let tail = frozen_len % 64;
            if tail != 0 && n_words > 0 {
                let mask = (1u64 << tail) - 1;
                debug_assert_eq!(
                    words[n_words - 1] & !mask,
                    0,
                    "set carries a tail bit past the frozen universe length"
                );
            }
            let rank_lut = Self::build_rank_lut(&words);
            HybridEffectSet::Dense {
                words: words.into_boxed_slice(),
                rank_lut,
                cardinality: card,
            }
        }
    }

    /// Sampled cumulative popcount: `rank_lut[b]` = number of set bits in
    /// `words[0 .. b*RANK_LUT_BLOCK]`. One entry per [`RANK_LUT_BLOCK`] words.
    fn build_rank_lut(words: &[u64]) -> Box<[u32]> {
        let n_blocks = words.len().div_ceil(RANK_LUT_BLOCK);
        let mut lut = Vec::with_capacity(n_blocks + 1);
        let mut acc = 0u32;
        lut.push(0);
        for block in words.chunks(RANK_LUT_BLOCK) {
            acc += block.iter().map(|w| w.count_ones()).sum::<u32>();
            lut.push(acc);
        }
        lut.into_boxed_slice()
    }

    /// The cardinality (number of ids) of this set.
    pub fn cardinality(&self) -> u32 {
        match self {
            HybridEffectSet::Sparse(ids) => ids.len() as u32,
            HybridEffectSet::Dense { cardinality, .. } => *cardinality,
        }
    }

    /// True iff `id` is a member.
    pub fn contains(&self, id: EffectId) -> bool {
        match self {
            HybridEffectSet::Sparse(ids) => ids.binary_search(&id).is_ok(),
            HybridEffectSet::Dense { words, .. } => has_bit(words, id),
        }
    }

    /// Iterate members in ascending `EffectId` (storage) order — the SAME
    /// order for sparse and dense (spec Step 3 invariant).
    pub fn iter(&self) -> Box<dyn Iterator<Item = EffectId> + '_> {
        match self {
            HybridEffectSet::Sparse(ids) => Box::new(ids.iter().copied()),
            HybridEffectSet::Dense { words, .. } => Box::new(iter_set_bits(words)),
        }
    }

    /// The membership ordinal of `id` (its 0-based position in ascending
    /// EffectId order), or `None` if absent. Dense uses `rank_lut` +
    /// `count_ones` for O(1); sparse binary-searches. This is the storage-order
    /// ordinal the future reverse-index (A4) uses for `base_via[ordinal]`
    /// point queries — the A3 projection instead uses the cached `ordered_ids`
    /// merge, so this is exercised by tests today.
    pub fn ordinal_of(&self, id: EffectId) -> Option<u32> {
        match self {
            HybridEffectSet::Sparse(ids) => ids.binary_search(&id).ok().map(|i| i as u32),
            HybridEffectSet::Dense {
                words, rank_lut, ..
            } => {
                if !has_bit(words, id) {
                    return None;
                }
                let word = (id.0 / 64) as usize;
                let block = word / RANK_LUT_BLOCK;
                let mut ord = rank_lut[block];
                for w in &words[block * RANK_LUT_BLOCK..word] {
                    ord += w.count_ones();
                }
                let below = words[word] & ((1u64 << (id.0 % 64)) - 1);
                ord += below.count_ones();
                Some(ord)
            }
        }
    }
}

/// FNV-1a hash of a set's CONTENT (its ascending EffectId sequence) — repr
/// independent, so a `Sparse` and a `Dense` holding the same ids hash equal
/// (spec Step 3 hash-cons invariant).
fn content_hash(bits: &[u64]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for id in iter_set_bits(bits) {
        for b in id.0.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h
}

// ---------------------------------------------------------------------------
// EffectStore — the frozen, hash-consed set store + ordered_ids cache.
// ---------------------------------------------------------------------------

/// The immutable, hash-consed set store (spec Step 3). Owns the frozen
/// universe, the deduplicated [`HybridEffectSet`] arena, and one cached
/// `ordered_ids` (key_rank order) per [`EffectSetId`].
pub struct EffectStore {
    universe: FrozenEffectUniverse,
    sets: Vec<HybridEffectSet>,
    /// Per `EffectSetId`: its members in `key_rank` (== `(effect_key,
    /// operation_id)`) order — built ONCE at intern, so a shared 797-member
    /// base is never re-sorted per routine (spec ⟨rev4 P1⟩).
    ordered: Vec<Box<[EffectId]>>,
    /// content-hash → candidate `EffectSetId`s (collision list; full equality
    /// resolves a hash collision).
    dedup: HashMap<u64, Vec<u32>>,
}

impl EffectStore {
    fn new(universe: FrozenEffectUniverse) -> Self {
        EffectStore {
            universe,
            sets: Vec::new(),
            ordered: Vec::new(),
            dedup: HashMap::new(),
        }
    }

    /// The frozen universe this store projects over.
    pub fn universe(&self) -> &FrozenEffectUniverse {
        &self.universe
    }

    /// Intern a presence bitset as a hash-consed [`EffectSetId`]. Content-equal
    /// sets (regardless of sparse/dense repr or trailing-zero word length)
    /// collapse to ONE id; a hash collision is resolved by full set-content
    /// equality. Builds the set's `key_rank`-ordered `ordered_ids` once.
    pub fn intern_set(&mut self, bits: &[u64]) -> EffectSetId {
        let hash = content_hash(bits);
        if let Some(candidates) = self.dedup.get(&hash) {
            for &cand in candidates {
                if self.set_eq_bits(EffectSetId(cand), bits) {
                    return EffectSetId(cand);
                }
            }
        }
        let id = EffectSetId(self.sets.len() as u32);
        let set = HybridEffectSet::from_bits(bits, self.universe.len());
        // ordered_ids: members sorted by key_rank (a total order, no ties).
        let mut ordered: Vec<EffectId> = set.iter().collect();
        ordered.sort_by_key(|&e| self.universe.key_rank(e));
        self.sets.push(set);
        self.ordered.push(ordered.into_boxed_slice());
        self.dedup.entry(hash).or_default().push(id.0);
        id
    }

    /// Full set-content equality between an existing set and a raw bitset —
    /// the hash-collision tie-break for [`Self::intern_set`].
    fn set_eq_bits(&self, id: EffectSetId, bits: &[u64]) -> bool {
        let set = &self.sets[id.0 as usize];
        if set.cardinality() != popcount(bits) {
            return false;
        }
        set.iter().eq(iter_set_bits(bits))
    }

    /// The [`HybridEffectSet`] for an id.
    pub fn set(&self, id: EffectSetId) -> &HybridEffectSet {
        &self.sets[id.0 as usize]
    }

    /// The cached `key_rank`-ordered members of a set — the base run of the
    /// base∪delta emit merge.
    pub fn ordered_ids(&self, id: EffectSetId) -> &[EffectId] {
        &self.ordered[id.0 as usize]
    }

    /// Number of distinct hash-consed sets.
    pub fn set_count(&self) -> usize {
        self.sets.len()
    }
}

// ---------------------------------------------------------------------------
// CompactRoutineSummary — the compact per-routine row (spec Step 3).
// ---------------------------------------------------------------------------

/// One compact per-routine db-effect row. `terminal_base` is a SHARED,
/// hash-consed [`EffectSetId`] (`closed_form_union`'s `C`); `base_via` is
/// per-row, in `key_rank` order PARALLEL to `ordered_ids(terminal_base)`;
/// `pd_delta`/`delta_via` are this routine's OWN PD facts (== `delta \ base`),
/// also `key_rank`-sorted and parallel. All three ranges are CSR slices into
/// the [`SummaryBundle`]'s pooled global arrays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactRoutineSummary {
    pub terminal_base: EffectSetId,
    pub base_via: Range<u32>,
    pub pd_delta: Range<u32>,
    pub delta_via: Range<u32>,
}

// ---------------------------------------------------------------------------
// DbEffectRef — the lazy, borrowing view of one materialized db-effect.
// ---------------------------------------------------------------------------

/// One materialized db-effect, borrowing its identity strings from the
/// bundle's dictionaries (the [`FrozenEffectUniverse`] + the workspace-wide
/// `record_variable_id` map) rather than owning them.
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

impl DbEffectRef<'_> {
    /// Materialize an owned [`DbEffect`] — for legacy callers only.
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

// ---------------------------------------------------------------------------
// SummaryBundleBuilder — solve-time accumulator (arena of shared C + rows).
// ---------------------------------------------------------------------------

/// One routine's raw, pre-canonicalization row: a [`SetRef`] into the
/// terminal-set arena + per-membership via/PD arrays in STORAGE order
/// (ascending EffectId — `finish` reorders them to `key_rank` order once the
/// universe is frozen).
#[derive(Debug)]
struct RawRow {
    terminal_set: SetRef,
    /// Parallel to `iter_set_bits(arena[terminal_set])` (ascending EffectId).
    base_via: Vec<ViaRank>,
    /// This routine's PD facts (ascending EffectId storage order).
    pd_delta: Vec<EffectId>,
    /// Parallel to `pd_delta`.
    delta_via: Vec<ViaRank>,
}

/// Accumulates the shared terminal-set arena + per-routine raw rows during the
/// per-SCC solve. Doubles as the db-effect FEED-FORWARD source: a settled
/// callee's terminal bits / PD ids are read straight from here by the solver
/// ([`Self::terminal_bits`]/[`Self::pd_ids`]) — no materialized `Vec<DbEffect>`.
/// [`Self::finish`] hash-conses the arena and freezes it into a
/// [`SummaryBundle`].
#[derive(Debug, Default)]
pub struct SummaryBundleBuilder {
    /// Shared `C` bitsets — one push per effective SCC (and per fixed leaf),
    /// referenced by every member's [`RawRow::terminal_set`].
    arena: Vec<Vec<u64>>,
    rows: HashMap<RoutineIx, RawRow>,
}

impl SummaryBundleBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push one shared terminal-set (`closed_form_union`'s `C`) into the arena,
    /// returning a [`SetRef`] every member of that effective SCC records — the
    /// "record ONE EffectSetId per SCC" rule (pre-canonicalization form).
    pub fn push_terminal_set(&mut self, bits: Vec<u64>) -> SetRef {
        let r = SetRef(self.arena.len() as u32);
        self.arena.push(bits);
        r
    }

    /// Record one routine's compact row. `base_via` MUST be parallel to
    /// `iter_set_bits(arena[terminal_set])` (ascending EffectId); `pd_delta`
    /// MUST be ascending-EffectId with `delta_via` parallel — `finish` reorders
    /// both to `key_rank` order.
    pub fn push_row(
        &mut self,
        routine_ix: RoutineIx,
        terminal_set: SetRef,
        base_via: Vec<ViaRank>,
        pd_delta: Vec<EffectId>,
        delta_via: Vec<ViaRank>,
    ) {
        debug_assert_eq!(pd_delta.len(), delta_via.len());
        self.rows.insert(
            routine_ix,
            RawRow {
                terminal_set,
                base_via,
                pd_delta,
                delta_via,
            },
        );
    }

    /// True iff `r` has a row (was solved this run, or is a retained fixed
    /// leaf) — the feed-forward "is this callee settled?" gate, matching the
    /// old `settled.get(&callee).is_some()`.
    pub fn has_row(&self, r: RoutineIx) -> bool {
        self.rows.contains_key(&r)
    }

    /// A settled routine's shared terminal-set bits (`C`) — for the solver's
    /// closed-form union / via folds to OR wholesale. Empty slice if `r` has
    /// no row.
    pub fn terminal_bits(&self, r: RoutineIx) -> &[u64] {
        match self.rows.get(&r) {
            Some(row) => &self.arena[row.terminal_set.0 as usize],
            None => &[],
        }
    }

    /// A settled routine's PD-fact ids — for the solver's PD reachability seed
    /// / substituted-via attribution. Empty slice if `r` has no row / no PD.
    pub fn pd_ids(&self, r: RoutineIx) -> &[EffectId] {
        match self.rows.get(&r) {
            Some(row) => &row.pd_delta,
            None => &[],
        }
    }

    /// Freeze into an immutable [`SummaryBundle`] (spec lifecycle steps 5-7):
    /// hash-cons the terminal-set arena into the [`EffectStore`], then rewrite
    /// every row's `terminal_base` `SetRef` → [`EffectSetId`] and reorder its
    /// per-membership via/PD arrays from STORAGE order to `key_rank` order,
    /// pooling them into the bundle's global CSR arrays.
    pub fn finish(
        self,
        universe: FrozenEffectUniverse,
        interner: RoutineInterner,
        rvid_by_opid: HashMap<String, Option<String>>,
    ) -> SummaryBundle {
        let mut store = EffectStore::new(universe);

        // Hash-cons each arena entry into a shared EffectSetId. Many SetRefs
        // (one per effective SCC) may collapse to one EffectSetId.
        let setref_to_setid: Vec<EffectSetId> = self
            .arena
            .iter()
            .map(|bits| store.intern_set(bits))
            .collect();

        let mut summaries: HashMap<RoutineIx, CompactRoutineSummary> =
            HashMap::with_capacity(self.rows.len());
        let mut via_pool: Vec<ViaRank> = Vec::new();
        let mut delta_pool: Vec<EffectId> = Vec::new();

        for (ix, raw) in self.rows {
            let setid = setref_to_setid[raw.terminal_set.0 as usize];

            // base_via: parallel to arena[terminal_set] storage order
            // (ascending EffectId). Reorder to key_rank order so it stays
            // parallel to ordered_ids(setid) — the ⟨rev3⟩ "rebuild base_via on
            // canonicalization" invariant (a set-id swap must re-align via).
            let storage_ids = &self.arena[raw.terminal_set.0 as usize];
            let base_via_kr = reorder_via_to_key_rank(store.universe(), storage_ids, &raw.base_via);
            let base_via = push_via(&mut via_pool, &base_via_kr);

            // pd_delta: sort by key_rank, permute delta_via in parallel.
            let (pd_sorted, delta_via_sorted) =
                sort_pd_by_key_rank(store.universe(), &raw.pd_delta, &raw.delta_via);
            let pd_delta = push_ids(&mut delta_pool, &pd_sorted);
            let delta_via = push_via(&mut via_pool, &delta_via_sorted);

            summaries.insert(
                ix,
                CompactRoutineSummary {
                    terminal_base: setid,
                    base_via,
                    pd_delta,
                    delta_via,
                },
            );
        }

        SummaryBundle {
            summaries,
            effects: store,
            via_pool,
            delta_pool,
            interner,
            rvid_by_opid,
        }
    }
}

/// Reorder a `base_via` array (parallel to `storage_ids` in ascending-EffectId
/// order) into `key_rank` order — so it stays parallel to a set's cached
/// `ordered_ids`. No ties in `key_rank`, so the permutation is total.
fn reorder_via_to_key_rank(
    universe: &FrozenEffectUniverse,
    storage_bits: &[u64],
    base_via: &[ViaRank],
) -> Vec<ViaRank> {
    let storage_ids: Vec<EffectId> = iter_set_bits(storage_bits).collect();
    debug_assert_eq!(storage_ids.len(), base_via.len());
    let mut pairs: Vec<(u32, ViaRank)> = storage_ids
        .iter()
        .zip(base_via.iter())
        .map(|(&id, &v)| (universe.key_rank(id), v))
        .collect();
    pairs.sort_by_key(|p| p.0);
    pairs.into_iter().map(|p| p.1).collect()
}

/// Sort a routine's PD facts by `key_rank`, permuting `delta_via` in parallel.
fn sort_pd_by_key_rank(
    universe: &FrozenEffectUniverse,
    pd_delta: &[EffectId],
    delta_via: &[ViaRank],
) -> (Vec<EffectId>, Vec<ViaRank>) {
    debug_assert_eq!(pd_delta.len(), delta_via.len());
    let mut triples: Vec<(u32, EffectId, ViaRank)> = pd_delta
        .iter()
        .zip(delta_via.iter())
        .map(|(&id, &v)| (universe.key_rank(id), id, v))
        .collect();
    triples.sort_by_key(|t| t.0);
    let ids = triples.iter().map(|t| t.1).collect();
    let vias = triples.iter().map(|t| t.2).collect();
    (ids, vias)
}

fn push_via(pool: &mut Vec<ViaRank>, vias: &[ViaRank]) -> Range<u32> {
    let start = pool.len() as u32;
    pool.extend_from_slice(vias);
    start..pool.len() as u32
}

fn push_ids(pool: &mut Vec<EffectId>, ids: &[EffectId]) -> Range<u32> {
    let start = pool.len() as u32;
    pool.extend_from_slice(ids);
    start..pool.len() as u32
}

// ---------------------------------------------------------------------------
// SummaryBundle — the immutable, workspace-complete compact store.
// ---------------------------------------------------------------------------

/// The immutable, workspace-complete compact db-effect store (spec "Public
/// API"). `db_effects(r)` is an O(result) two-way merge of the shared base's
/// cached `ordered_ids` (+ per-row `base_via`) with the row's `key_rank`-sorted
/// `pd_delta` (+ `delta_via`).
pub struct SummaryBundle {
    summaries: HashMap<RoutineIx, CompactRoutineSummary>,
    effects: EffectStore,
    via_pool: Vec<ViaRank>,
    delta_pool: Vec<EffectId>,
    interner: RoutineInterner,
    rvid_by_opid: HashMap<String, Option<String>>,
}

impl SummaryBundle {
    /// Look up the [`RoutineIx`] for a routine id, if interned.
    pub fn routine_ix(&self, routine_id: &str) -> Option<RoutineIx> {
        self.interner.get(routine_id)
    }

    /// True iff `r` has a compact row (was recomputed this run OR is a retained
    /// fixed leaf — spec ⟨rev3⟩ "fixed leaves get a singleton class"). A
    /// missing routine gets no row.
    pub fn has_row(&self, r: RoutineIx) -> bool {
        self.summaries.contains_key(&r)
    }

    /// The hash-consed set store (for A4's reverse index + queries).
    pub fn effects(&self) -> &EffectStore {
        &self.effects
    }

    /// The shared [`EffectSetId`] of a routine's terminal base (its effect
    /// class's set), if it has a row — the notion A4's reverse transpose
    /// consumes.
    pub fn terminal_base(&self, r: RoutineIx) -> Option<EffectSetId> {
        self.summaries.get(&r).map(|row| row.terminal_base)
    }

    /// The lazy `DbEffect` view for one routine — an O(result) two-way merge
    /// of `ordered_ids(terminal_base)` (with `base_via`) and the row's
    /// `key_rank`-sorted `pd_delta` (with `delta_via`), yielding effects in
    /// `(effect_key, operation_id)` order (byte-identical to the old solver's
    /// `Vec<DbEffect>` sort). Empty for a routine with no row.
    pub fn db_effects(&self, r: RoutineIx) -> impl Iterator<Item = DbEffectRef<'_>> {
        let out = match self.summaries.get(&r) {
            Some(row) => self.project_row(row),
            None => Vec::new(),
        };
        out.into_iter()
    }

    fn project_row(&self, row: &CompactRoutineSummary) -> Vec<DbEffectRef<'_>> {
        let base_ids = self.effects.ordered_ids(row.terminal_base);
        let base_via = &self.via_pool[row.base_via.start as usize..row.base_via.end as usize];
        let pd_ids = &self.delta_pool[row.pd_delta.start as usize..row.pd_delta.end as usize];
        let pd_via = &self.via_pool[row.delta_via.start as usize..row.delta_via.end as usize];
        debug_assert_eq!(base_ids.len(), base_via.len());
        debug_assert_eq!(pd_ids.len(), pd_via.len());

        let universe = self.effects.universe();
        let mut out: Vec<DbEffectRef<'_>> = Vec::with_capacity(base_ids.len() + pd_ids.len());
        let (mut i, mut j) = (0usize, 0usize);
        // Two-way merge by key_rank. base (terminal) and pd (PD) are disjoint
        // EffectId sets with a total key_rank order, so the merge is total and
        // interleaves PD/terminal variants of the same base effect in
        // (effect_key, operation_id) order.
        while i < base_ids.len() && j < pd_ids.len() {
            if universe.key_rank(base_ids[i]) <= universe.key_rank(pd_ids[j]) {
                out.push(self.make_ref(base_ids[i], base_via[i]));
                i += 1;
            } else {
                out.push(self.make_ref(pd_ids[j], pd_via[j]));
                j += 1;
            }
        }
        while i < base_ids.len() {
            out.push(self.make_ref(base_ids[i], base_via[i]));
            i += 1;
        }
        while j < pd_ids.len() {
            out.push(self.make_ref(pd_ids[j], pd_via[j]));
            j += 1;
        }
        out
    }

    fn make_ref(&self, id: EffectId, via: ViaRank) -> DbEffectRef<'_> {
        let universe = self.effects.universe();
        let identity = universe.identity(id);
        DbEffectRef {
            effect_key: universe.effect_key_cached(id),
            operation_id: &identity.operation_id,
            op: &identity.op,
            table_id: &identity.table_id,
            record_variable_id: self
                .rvid_by_opid
                .get(&identity.operation_id)
                .and_then(|o| o.as_deref()),
            temp_state: &identity.temp,
            via,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::l4::effect_universe::{EffectIdentity, GrowingEffectUniverse};
    use crate::engine::l4::summary::TempState;

    const CANONICAL: [&str; 5] = [
        "direct",
        "implicit-trigger",
        "event-subscriber",
        "dynamic",
        "inherited",
    ];

    fn ident(op: &str, table: &str, opid: &str, temp: TempStateKind) -> EffectIdentity {
        EffectIdentity {
            op: op.into(),
            table_id: table.into(),
            operation_id: opid.into(),
            temp,
        }
    }

    fn bits_of(ids: &[EffectId]) -> Vec<u64> {
        let mut b = Vec::new();
        for &id in ids {
            set_bit(&mut b, id);
        }
        b
    }

    // ---- ViaRank ---------------------------------------------------------

    #[test]
    fn via_rank_round_trips_the_5_canonical_strings() {
        for s in CANONICAL {
            assert_eq!(ViaRank::from_str(s).as_str(), s);
        }
    }

    #[test]
    fn via_rank_ordering_matches_merge_via_precedence() {
        assert!(ViaRank::Direct > ViaRank::ImplicitTrigger);
        assert!(ViaRank::ImplicitTrigger > ViaRank::EventSubscriber);
        assert!(ViaRank::EventSubscriber > ViaRank::Dynamic);
        assert!(ViaRank::Dynamic > ViaRank::Inherited);
    }

    #[test]
    fn via_rank_from_bogus_string_floors_to_inherited() {
        assert_eq!(ViaRank::from_str("totally-bogus-via"), ViaRank::Inherited);
    }

    // ---- HybridEffectSet: sparse/dense parity ----------------------------

    /// A `Sparse` and a `Dense` set holding the SAME ids hash equal
    /// (content-hash is repr independent) and iterate the SAME ascending
    /// storage order (spec Step 3 invariants).
    #[test]
    fn sparse_and_dense_same_ids_hash_and_iterate_identically() {
        // Ids spanning several words, chosen so one repr is forced sparse and
        // the other dense by threshold in `from_bits`.
        let ids: Vec<EffectId> = (0..400u32).map(|n| EffectId(n * 3)).collect();
        let bits = bits_of(&ids);

        // Force sparse and dense explicitly (bypassing the threshold) to prove
        // repr-independence of both content_hash and iteration order.
        let sparse = HybridEffectSet::Sparse(ids.clone().into_boxed_slice());
        let frozen_len = (ids.last().unwrap().0 + 1) as usize;
        let dense = {
            // Build a dense repr directly from the bits.
            match HybridEffectSet::from_bits(&bits, frozen_len) {
                d @ HybridEffectSet::Dense { .. } => d,
                HybridEffectSet::Sparse(_) => unreachable!("400 ids exceed the sparse threshold"),
            }
        };
        // Same ascending storage order.
        let s_iter: Vec<EffectId> = sparse.iter().collect();
        let d_iter: Vec<EffectId> = dense.iter().collect();
        assert_eq!(s_iter, ids);
        assert_eq!(d_iter, ids);
        // Same cardinality.
        assert_eq!(sparse.cardinality(), dense.cardinality());
        // content_hash is over the bits (repr independent): the SAME hash for
        // the same logical set regardless of which repr produced the bits.
        assert_eq!(content_hash(&bits), content_hash(&bits_of(&d_iter)));
    }

    /// Threshold conversion preserves cardinality and ascending order: a set
    /// just below the threshold is Sparse, one just above is Dense, both
    /// round-trip their ids in order.
    #[test]
    fn threshold_conversion_preserves_cardinality_and_order() {
        let below: Vec<EffectId> = (0..(SPARSE_THRESHOLD - 1)).map(EffectId).collect();
        let above: Vec<EffectId> = (0..(SPARSE_THRESHOLD + 5)).map(EffectId).collect();
        let flen = (SPARSE_THRESHOLD + 5) as usize;

        let s = HybridEffectSet::from_bits(&bits_of(&below), flen);
        let d = HybridEffectSet::from_bits(&bits_of(&above), flen);
        assert!(matches!(s, HybridEffectSet::Sparse(_)), "below is sparse");
        assert!(matches!(d, HybridEffectSet::Dense { .. }), "above is dense");
        assert_eq!(s.cardinality(), SPARSE_THRESHOLD - 1);
        assert_eq!(d.cardinality(), SPARSE_THRESHOLD + 5);
        assert_eq!(s.iter().collect::<Vec<_>>(), below);
        assert_eq!(d.iter().collect::<Vec<_>>(), above);
    }

    /// `ordinal_of` (storage-order membership rank) agrees with a linear scan,
    /// for both reprs — the future A4 point-query primitive.
    #[test]
    fn ordinal_of_matches_linear_scan_sparse_and_dense() {
        for card in [10u32, SPARSE_THRESHOLD + 10] {
            let ids: Vec<EffectId> = (0..card).map(|n| EffectId(n * 2 + 1)).collect();
            let flen = (ids.last().unwrap().0 + 1) as usize;
            let set = HybridEffectSet::from_bits(&bits_of(&ids), flen);
            for (expected_ord, &id) in ids.iter().enumerate() {
                assert_eq!(set.ordinal_of(id), Some(expected_ord as u32));
            }
            assert_eq!(
                set.ordinal_of(EffectId(0)),
                None,
                "absent id has no ordinal"
            );
        }
    }

    // ---- EffectStore: hash-cons ------------------------------------------

    /// Content-equal sets collapse to ONE `EffectSetId` (across sparse & dense
    /// reprs); distinct sets get distinct ids.
    #[test]
    fn intern_set_dedups_content_equal_sets() {
        let mut u = GrowingEffectUniverse::new();
        for n in 0..300u32 {
            u.intern(&ident(
                "Op",
                "t",
                &format!("op{n}"),
                TempStateKind::Known(true),
            ));
        }
        let store_u = u.freeze();
        let mut store = EffectStore::new(store_u);

        let small_a = bits_of(&[EffectId(1), EffectId(5), EffectId(9)]);
        let small_b = bits_of(&[EffectId(9), EffectId(1), EffectId(5)]); // same set
        let big: Vec<EffectId> = (0..300).map(EffectId).collect();

        let a = store.intern_set(&small_a);
        let b = store.intern_set(&small_b);
        assert_eq!(a, b, "content-equal small sets share one id");

        let big1 = store.intern_set(&bits_of(&big));
        let big2 = store.intern_set(&bits_of(&big));
        assert_eq!(big1, big2, "content-equal dense sets share one id");
        assert_ne!(a, big1, "distinct sets get distinct ids");
        assert_eq!(store.set_count(), 2);
    }

    /// `ordered_ids` is `key_rank` (== `(effect_key, operation_id)`) order,
    /// NOT raw EffectId order — even when ids are interned out of key order.
    #[test]
    fn ordered_ids_is_key_rank_order() {
        let mut u = GrowingEffectUniverse::new();
        // Intern so EffectId order != effect_key order: Zeta(0) Alpha(1) Mu(2).
        let zeta = u.intern(&ident("Zeta", "t", "op1", TempStateKind::Known(true)));
        let alpha = u.intern(&ident("Alpha", "t", "op2", TempStateKind::Known(true)));
        let mu = u.intern(&ident("Mu", "t", "op3", TempStateKind::Known(true)));
        let frozen = u.freeze();
        let mut store = EffectStore::new(frozen);
        let sid = store.intern_set(&bits_of(&[zeta, alpha, mu]));
        // key order: Alpha < Mu < Zeta.
        assert_eq!(store.ordered_ids(sid), &[alpha, mu, zeta]);
    }

    // ---- Bundle projection: delta invariants + round-trip ----------------

    /// The full row round-trip: build a universe with 2 terminal + 1 PD
    /// effect (interned OUT of key order), push a shared set + a row, freeze,
    /// and assert `db_effects` yields the effects in `(effect_key,
    /// operation_id)` order with the right via/temp/rvid.
    #[test]
    fn db_effects_two_way_merge_is_key_rank_ordered() {
        let mut u = GrowingEffectUniverse::new();
        let zeta = u.intern(&ident("Zeta", "t1", "op1", TempStateKind::Known(true)));
        let alpha = u.intern(&ident("Alpha", "t1", "op2", TempStateKind::Unknown));
        let mid = u.intern(&ident(
            "Middle",
            "t2",
            "op3",
            TempStateKind::ParameterDependent(0),
        ));

        let mut rvid: HashMap<String, Option<String>> = HashMap::new();
        rvid.insert("op1".into(), Some("Rec".into()));
        rvid.insert("op2".into(), None);
        rvid.insert("op3".into(), Some("Rec2".into()));

        let mut interner = RoutineInterner::new();
        let r = interner.intern("r");

        let mut b = SummaryBundleBuilder::new();
        // Terminal C = {zeta, alpha}; base_via parallel to ascending EffectId
        // storage order: zeta(id0) then alpha(id1) => [Direct, EventSubscriber].
        let set = b.push_terminal_set(bits_of(&[zeta, alpha]));
        b.push_row(
            r,
            set,
            vec![ViaRank::Direct, ViaRank::EventSubscriber],
            vec![mid],
            vec![ViaRank::Dynamic],
        );
        let bundle = b.finish(u.freeze(), interner, rvid);

        let got: Vec<DbEffect> = bundle.db_effects(r).map(|e| e.to_owned()).collect();
        assert_eq!(got.len(), 3);
        // key order: Alpha|..|op2|u < Middle|..|op3|p0 < Zeta|..|op1|t
        assert_eq!(
            (got[0].op.as_str(), got[0].via.as_str()),
            ("Alpha", "event-subscriber")
        );
        assert_eq!(got[0].temp_state, TempState::Unknown);
        assert_eq!(got[0].record_variable_id, None);
        assert_eq!(
            (got[1].op.as_str(), got[1].via.as_str()),
            ("Middle", "dynamic")
        );
        assert_eq!(got[1].temp_state, TempState::ParameterDependent(0));
        assert_eq!(got[1].record_variable_id, Some("Rec2".to_string()));
        assert_eq!(
            (got[2].op.as_str(), got[2].via.as_str()),
            ("Zeta", "direct")
        );
        assert_eq!(got[2].temp_state, TempState::Known(true));
        assert_eq!(got[2].record_variable_id, Some("Rec".to_string()));
    }

    /// A row's stored `pd_delta` is `key_rank`-sorted, unique by EffectId, and
    /// exactly `delta \ base` (disjoint from the terminal base — PD-typed vs
    /// terminal-typed identities are distinct EffectIds by construction).
    #[test]
    fn pd_delta_is_key_rank_sorted_unique_and_disjoint_from_base() {
        let mut u = GrowingEffectUniverse::new();
        // Two terminals + two PD facts, interned so EffectId order != key order.
        let t_z = u.intern(&ident("Zeta", "t", "opz", TempStateKind::Known(true)));
        let t_a = u.intern(&ident("Alpha", "t", "opa", TempStateKind::Known(true)));
        let pd_y = u.intern(&ident(
            "Yak",
            "t",
            "opy",
            TempStateKind::ParameterDependent(0),
        ));
        let pd_b = u.intern(&ident(
            "Beta",
            "t",
            "opb",
            TempStateKind::ParameterDependent(1),
        ));

        let mut interner = RoutineInterner::new();
        let r = interner.intern("r");
        let mut b = SummaryBundleBuilder::new();
        let set = b.push_terminal_set(bits_of(&[t_z, t_a]));
        // pd_delta pushed in EffectId order (pd_y before pd_b); finish must
        // reorder to key_rank (Beta < Yak).
        b.push_row(
            r,
            set,
            vec![ViaRank::Direct, ViaRank::Direct],
            vec![pd_y, pd_b],
            vec![ViaRank::Direct, ViaRank::Dynamic],
        );
        let rvid: HashMap<String, Option<String>> = HashMap::new();
        let frozen = u.freeze();
        // Compute expected key_rank BEFORE moving `frozen` into the bundle.
        let (kr_b, kr_y) = (frozen.key_rank(pd_b), frozen.key_rank(pd_y));
        assert!(kr_b < kr_y, "Beta sorts before Yak by effect_key");
        let bundle = b.finish(frozen, interner, rvid);

        // Inspect the projected PD effects (temp is PD) — they appear in
        // key_rank order, and every id is unique.
        let got: Vec<DbEffect> = bundle.db_effects(r).map(|e| e.to_owned()).collect();
        let pd_ops: Vec<&str> = got
            .iter()
            .filter(|e| matches!(e.temp_state, TempState::ParameterDependent(_)))
            .map(|e| e.op.as_str())
            .collect();
        assert_eq!(
            pd_ops,
            vec!["Beta", "Yak"],
            "pd_delta emitted key_rank-sorted"
        );
        // Delta is disjoint from base: no op appears with both a terminal and
        // a PD temp here (distinct identities), and counts add up.
        assert_eq!(got.len(), 4);
    }

    /// A fixed-leaf singleton class round-trips its OWN via (spec ⟨rev3⟩): a
    /// leaf pushed as its own set + row projects back to the same via it came
    /// in with.
    #[test]
    fn fixed_leaf_singleton_class_round_trips_its_own_via() {
        let mut u = GrowingEffectUniverse::new();
        let e = u.intern(&ident("Insert", "t3", "c_op1", TempStateKind::Known(true)));
        let mut interner = RoutineInterner::new();
        let leaf = interner.intern("c");
        let mut b = SummaryBundleBuilder::new();
        let set = b.push_terminal_set(bits_of(&[e]));
        // The leaf's OWN via is "direct".
        b.push_row(leaf, set, vec![ViaRank::Direct], vec![], vec![]);
        let rvid: HashMap<String, Option<String>> = HashMap::new();
        let bundle = b.finish(u.freeze(), interner, rvid);

        assert!(bundle.has_row(leaf));
        let got: Vec<DbEffect> = bundle.db_effects(leaf).map(|e| e.to_owned()).collect();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].via, "direct");
        assert_eq!(got[0].op, "Insert");
        assert_eq!(got[0].temp_state, TempState::Known(true));
    }

    #[test]
    fn db_effects_is_empty_for_a_routine_with_no_row() {
        let u = GrowingEffectUniverse::new();
        let mut interner = RoutineInterner::new();
        let r = interner.intern("leaf");
        let rvid: HashMap<String, Option<String>> = HashMap::new();
        let bundle = SummaryBundleBuilder::new().finish(u.freeze(), interner, rvid);
        assert_eq!(bundle.db_effects(r).count(), 0);
        assert!(!bundle.has_row(r));
    }

    /// SCC-sharing: two routines with the SAME terminal set (pushed as two
    /// SetRefs) canonicalize to ONE `EffectSetId`, but keep their OWN per-row
    /// via.
    #[test]
    fn scc_shared_set_id_with_per_member_via() {
        let mut u = GrowingEffectUniverse::new();
        let e0 = u.intern(&ident("Insert", "t", "op0", TempStateKind::Known(true)));
        let e1 = u.intern(&ident("Modify", "t", "op1", TempStateKind::Known(true)));
        let mut interner = RoutineInterner::new();
        let (m0, m1) = (interner.intern("m0"), interner.intern("m1"));
        let mut b = SummaryBundleBuilder::new();
        // Two separate pushes of the SAME content — must hash-cons to one id.
        let s0 = b.push_terminal_set(bits_of(&[e0, e1]));
        let s1 = b.push_terminal_set(bits_of(&[e0, e1]));
        b.push_row(
            m0,
            s0,
            vec![ViaRank::Direct, ViaRank::Direct],
            vec![],
            vec![],
        );
        b.push_row(
            m1,
            s1,
            vec![ViaRank::EventSubscriber, ViaRank::Dynamic],
            vec![],
            vec![],
        );
        let rvid: HashMap<String, Option<String>> = HashMap::new();
        let bundle = b.finish(u.freeze(), interner, rvid);
        assert_eq!(
            bundle.terminal_base(m0),
            bundle.terminal_base(m1),
            "SCC members share one EffectSetId"
        );
        assert_eq!(bundle.effects().set_count(), 1, "hash-consed to one set");
        // Per-member via preserved.
        let v0: Vec<String> = bundle
            .db_effects(m0)
            .map(|e| e.via.as_str().to_string())
            .collect();
        let v1: Vec<String> = bundle
            .db_effects(m1)
            .map(|e| e.via.as_str().to_string())
            .collect();
        assert_eq!(v0, vec!["direct", "direct"]);
        assert_eq!(v1, vec!["event-subscriber", "dynamic"]);
    }

    /// Feed-forward reads (terminal bits + PD ids) come straight off the
    /// builder — no `Vec<DbEffect>` materialization.
    #[test]
    fn builder_feed_forward_reads_ids_not_strings() {
        let mut u = GrowingEffectUniverse::new();
        let t = u.intern(&ident("Insert", "t", "op0", TempStateKind::Known(true)));
        let pd = u.intern(&ident(
            "Modify",
            "t",
            "op1",
            TempStateKind::ParameterDependent(0),
        ));
        let mut interner = RoutineInterner::new();
        let m = interner.intern("m");
        let ghost = interner.intern("ghost");
        let mut b = SummaryBundleBuilder::new();
        let set = b.push_terminal_set(bits_of(&[t]));
        b.push_row(
            m,
            set,
            vec![ViaRank::Direct],
            vec![pd],
            vec![ViaRank::Direct],
        );
        assert!(b.has_row(m));
        assert!(!b.has_row(ghost));
        assert_eq!(
            iter_set_bits(b.terminal_bits(m)).collect::<Vec<_>>(),
            vec![t]
        );
        assert_eq!(b.pd_ids(m), &[pd]);
        assert!(b.terminal_bits(ghost).is_empty());
        assert!(b.pd_ids(ghost).is_empty());
    }
}
