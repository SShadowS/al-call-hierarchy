//! `EffectUniverse` — interns the structured db-effect identity `(op, table_id,
//! operation_id, TempStateKind)` as a compact `EffectId(u32)`.
//!
//! ## Growing vs Frozen (spec Part A Step 1, ⟨rev4⟩ — the freeze typestate)
//!
//! The universe grows lazily during solving: new effect identities are
//! discovered as PD substitution invents new variants and terminal emissions
//! settle. So it exists in two type-distinct phases:
//!
//!   - [`GrowingEffectUniverse`] — the solve-time interner. `intern()` mints a
//!     fresh [`EffectId`] on first sight; `get()`/`identity()` read back. This
//!     is what the db-effect solver threads `&mut` through every SCC.
//!   - [`FrozenEffectUniverse`] — the post-solve, immutable form
//!     ([`GrowingEffectUniverse::freeze`] produces it). It has NO `intern`
//!     method at all, so **post-freeze identity creation is a COMPILE error,
//!     not a runtime assert** (⟨rev4⟩). It carries the one-shot
//!     `key_rank: Vec<u32>` (`EffectId → rank under (effect_key, operation_id)`)
//!     computed once at freeze, plus a CHECKED `get()` whose miss a
//!     `debug_assert`/test can catch (guarding against a terminal identity
//!     discovered post-freeze — every identity source must be interned BEFORE
//!     freeze, spec lifecycle step 2).
//!
//! `EffectId`s are NEVER reassigned (⟨rev3⟩): storage/membership stays
//! EffectId-keyed (`word id/64`); only OUTPUT ordering uses `key_rank`. The
//! freeze does not remap ids — it only computes the ordering side-table.
//!
//! `sorted_order()` reproduces the EXACT `(effect_key, operation_id)`
//! materialization order that the old Jacobi solver produced for its
//! `Vec<DbEffect>` — this determinism is load-bearing: the compact store's
//! projection depends on the new solver's materialized output being
//! byte-identical to the old one.
//!
//! Deliberately EXCLUDES from the interned identity:
//!   - `via` (provenance) — reconstructed by a separate post-pass.
//!   - `record_variable_id` — non-key payload, carried alongside the id when a
//!     `DbEffect` is materialized, never part of identity/de-duplication.

use std::collections::HashMap;

use crate::engine::l4::effect_lattice::{TempStateKind, effect_key_of};

/// A compact, interned handle to an `EffectIdentity`. Cheap to copy, put in
/// bitsets, and use as a `HashMap`/`HashSet` key.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct EffectId(pub u32);

/// Structured effect identity — the interned key. Excludes `via` (provenance)
/// and `record_variable_id` (non-key payload), matching `effect_key_of`
/// (`effect_lattice.rs:122`).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct EffectIdentity {
    pub op: String,
    pub table_id: String,
    pub operation_id: String,
    pub temp: TempStateKind,
}

/// Compute the `(effect_key, operation_id)` sort permutation for a slice of
/// interned identities/keys, returned as a `key_rank: Vec<u32>` indexed by
/// `EffectId.0` (`key_rank[id] = rank`). Shared by [`GrowingEffectUniverse::freeze`]
/// (the workspace-wide `key_rank`) and any test that needs the same ranking —
/// ONE implementation of the ordering rule (DRY).
///
/// Ranks form a TOTAL order with no ties: two distinct identities cannot share
/// an `effect_key` (it embeds `op|table|operation|tempfrag`, the whole
/// identity), so the sort is unambiguous and `key_rank` reproduces the old
/// solver's `(effect_key, operation_id)` `Vec<DbEffect>` order exactly.
fn compute_key_rank(keys: &[String], by_id: &[EffectIdentity]) -> Vec<u32> {
    let mut order: Vec<u32> = (0..by_id.len() as u32).collect();
    order.sort_by(|&a, &b| {
        keys[a as usize].cmp(&keys[b as usize]).then_with(|| {
            by_id[a as usize]
                .operation_id
                .cmp(&by_id[b as usize].operation_id)
        })
    });
    let mut key_rank = vec![0u32; by_id.len()];
    for (rank, &id) in order.iter().enumerate() {
        key_rank[id as usize] = rank as u32;
    }
    key_rank
}

/// Interner: `EffectIdentity -> EffectId`, plus the reverse `Vec` for
/// `EffectId -> EffectIdentity` lookup. Lazily grows as PD substitution
/// invents new effect variants during solving — never frozen mid-solve. Call
/// [`Self::freeze`] once, AFTER solving completes, to obtain the immutable
/// [`FrozenEffectUniverse`] the compact store projects over.
pub struct GrowingEffectUniverse {
    by_identity: HashMap<EffectIdentity, EffectId>,
    by_id: Vec<EffectIdentity>,
    /// Cached `effect_key` per id, parallel to `by_id` — computed ONCE, at
    /// `intern()` time, never recomputed per lookup/comparison (Task A1: the
    /// fix for `materialize_member_db_effects`'s old `format!`-per-comparison
    /// sort key).
    keys: Vec<String>,
}

impl Default for GrowingEffectUniverse {
    fn default() -> Self {
        Self::new()
    }
}

impl GrowingEffectUniverse {
    pub fn new() -> Self {
        GrowingEffectUniverse {
            by_identity: HashMap::new(),
            by_id: Vec::new(),
            keys: Vec::new(),
        }
    }

    /// Intern `identity`, creating a fresh `EffectId` on first sight. Stable
    /// across repeated calls with an equal identity (re-interning an
    /// already-seen identity returns the SAME id, never a duplicate).
    pub fn intern(&mut self, identity: &EffectIdentity) -> EffectId {
        if let Some(&id) = self.by_identity.get(identity) {
            return id;
        }
        let id = EffectId(self.by_id.len() as u32);
        let key = effect_key_of(
            &identity.op,
            &identity.table_id,
            &identity.operation_id,
            &identity.temp,
        );
        self.by_id.push(identity.clone());
        self.by_identity.insert(identity.clone(), id);
        self.keys.push(key);
        id
    }

    /// Look up an already-interned identity without creating one.
    pub fn get(&self, identity: &EffectIdentity) -> Option<EffectId> {
        self.by_identity.get(identity).copied()
    }

    /// The structured identity for an interned id. Panics if `id` was never
    /// produced by this universe's `intern` — every `EffectId` in circulation
    /// is expected to have come from exactly one universe.
    pub fn identity(&self, id: EffectId) -> &EffectIdentity {
        &self.by_id[id.0 as usize]
    }

    /// Number of distinct effect identities interned so far.
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// Deterministic materialization order: every interned `EffectId`, sorted
    /// by `(effect_key, operation_id)` — reproducing the old solver's
    /// `Vec<DbEffect>` sort exactly.
    pub fn sorted_order(&self) -> Vec<EffectId> {
        let mut ids: Vec<EffectId> = (0..self.by_id.len() as u32).map(EffectId).collect();
        ids.sort_by(|&a, &b| {
            let ia = self.identity(a);
            let ib = self.identity(b);
            self.effect_key_cached(a)
                .cmp(self.effect_key_cached(b))
                .then_with(|| ia.operation_id.cmp(&ib.operation_id))
        });
        ids
    }

    /// The full `effect_key` string for an id — allocating, recomputed on
    /// every call. Reference implementation / cross-check oracle against
    /// [`Self::effect_key_cached`]; hot paths use the cached form.
    pub fn effect_key(&self, id: EffectId) -> String {
        let identity = self.identity(id);
        effect_key_of(
            &identity.op,
            &identity.table_id,
            &identity.operation_id,
            &identity.temp,
        )
    }

    /// The cached `effect_key` for an id — computed once, at `intern()` time,
    /// never recomputed per lookup/comparison. `&str` compares, zero allocation.
    pub fn effect_key_cached(&self, id: EffectId) -> &str {
        &self.keys[id.0 as usize]
    }

    /// Freeze this universe into an immutable [`FrozenEffectUniverse`],
    /// computing the one-shot `key_rank` (spec lifecycle step 5). Consumes
    /// `self` — after this, NO new identity can ever be minted (the growable
    /// form is gone), so a post-freeze identity discovery is a COMPILE error
    /// rather than a runtime assert (⟨rev4⟩). Call ONCE, after the whole
    /// solve is complete and every identity source has been interned.
    pub fn freeze(self) -> FrozenEffectUniverse {
        let key_rank = compute_key_rank(&self.keys, &self.by_id);
        FrozenEffectUniverse {
            by_identity: self.by_identity,
            by_id: self.by_id,
            keys: self.keys,
            key_rank,
        }
    }
}

/// The immutable, post-solve universe (spec Part A Step 1, ⟨rev4⟩). Produced
/// by [`GrowingEffectUniverse::freeze`]. Has NO `intern` method — the type
/// system makes post-freeze identity creation unrepresentable — only a CHECKED
/// [`Self::get`], plus the one-shot `key_rank` OUTPUT ordering (`EffectId →
/// rank under (effect_key, operation_id)`). EffectIds are unchanged from the
/// growing form (no remap).
pub struct FrozenEffectUniverse {
    by_identity: HashMap<EffectIdentity, EffectId>,
    by_id: Vec<EffectIdentity>,
    keys: Vec<String>,
    /// `EffectId.0 -> rank under (effect_key, operation_id)`. Drives all
    /// OUTPUT ordering (Step 3's base∪delta ordered merge); storage/membership
    /// stays EffectId-keyed. Computed once at freeze, never mid-solve.
    key_rank: Vec<u32>,
}

impl FrozenEffectUniverse {
    /// Checked lookup of an already-interned identity. A miss returns `None`;
    /// callers that must never miss post-freeze (shared-set/closure
    /// construction) wrap this in a `debug_assert`/test so a
    /// discovered-too-late identity fails loudly rather than silently
    /// vanishing (spec lifecycle step 3).
    pub fn get(&self, identity: &EffectIdentity) -> Option<EffectId> {
        self.by_identity.get(identity).copied()
    }

    /// The structured identity for an interned id. Panics if `id` is out of
    /// range for this universe.
    pub fn identity(&self, id: EffectId) -> &EffectIdentity {
        &self.by_id[id.0 as usize]
    }

    /// Number of distinct effect identities (the frozen universe length `U` —
    /// dense sets are sized `ceil(U/64)`).
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// The cached `effect_key` for an id — `&str`, zero allocation.
    pub fn effect_key_cached(&self, id: EffectId) -> &str {
        &self.keys[id.0 as usize]
    }

    /// The full `effect_key` string for an id — allocating (reference/oracle).
    pub fn effect_key(&self, id: EffectId) -> String {
        let identity = self.identity(id);
        effect_key_of(
            &identity.op,
            &identity.table_id,
            &identity.operation_id,
            &identity.temp,
        )
    }

    /// The OUTPUT rank of an id under `(effect_key, operation_id)` — the key
    /// by which all ordered iteration/merge sorts. A total order (no ties).
    pub fn key_rank(&self, id: EffectId) -> u32 {
        self.key_rank[id.0 as usize]
    }

    /// Deterministic materialization order: every id, ascending by `key_rank`
    /// (== `(effect_key, operation_id)` order). O(U log U) — used for cache
    /// builds / tests, not per-set (a set caches its own `ordered_ids`).
    pub fn sorted_order(&self) -> Vec<EffectId> {
        let mut ids: Vec<EffectId> = (0..self.by_id.len() as u32).map(EffectId).collect();
        ids.sort_by_key(|&id| self.key_rank(id));
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ident(op: &str, table: &str, opid: &str, temp: TempStateKind) -> EffectIdentity {
        EffectIdentity {
            op: op.into(),
            table_id: table.into(),
            operation_id: opid.into(),
            temp,
        }
    }

    #[test]
    fn intern_is_deterministic_and_sorted_order_matches_effect_key() {
        let mut u = GrowingEffectUniverse::new();
        let a = ident("Insert", "t2", "op9", TempStateKind::Known(true));
        let b = ident("Insert", "t1", "op1", TempStateKind::Known(true));
        let ia = u.intern(&a);
        let ib = u.intern(&b);
        assert_eq!(u.intern(&a), ia, "re-intern stable");
        assert_ne!(ia, ib);
        let order = u.sorted_order();
        let keys: Vec<String> = order.iter().map(|&e| u.effect_key(e)).collect();
        let mut expected = keys.clone();
        expected.sort();
        assert_eq!(keys, expected, "sorted_order yields effect_key ascending");
    }

    #[test]
    fn effect_key_cached_matches_effect_key() {
        let mut u = GrowingEffectUniverse::new();
        let a = ident("Insert", "t2", "op9", TempStateKind::Known(true));
        let b = ident("Modify", "t1", "op1", TempStateKind::Unknown);
        let ia = u.intern(&a);
        let ib = u.intern(&b);
        assert_eq!(u.effect_key_cached(ia), u.effect_key(ia));
        assert_eq!(u.effect_key_cached(ib), u.effect_key(ib));
    }

    #[test]
    fn get_returns_none_before_intern_and_some_after() {
        let mut u = GrowingEffectUniverse::new();
        let a = ident("Modify", "t1", "op1", TempStateKind::Unknown);
        assert_eq!(u.get(&a), None);
        let id = u.intern(&a);
        assert_eq!(u.get(&a), Some(id));
    }

    #[test]
    fn identity_roundtrips_through_intern() {
        let mut u = GrowingEffectUniverse::new();
        let a = ident("Delete", "t3", "op7", TempStateKind::ParameterDependent(2));
        let id = u.intern(&a);
        assert_eq!(u.identity(id), &a);
        assert_eq!(u.len(), 1);
    }

    #[test]
    fn sorted_order_breaks_effect_key_ties_by_operation_id() {
        let mut u = GrowingEffectUniverse::new();
        let a = ident("Insert", "t1", "op2", TempStateKind::Known(true));
        let b = ident("Insert", "t1", "op10", TempStateKind::Known(true));
        let ia = u.intern(&a);
        let ib = u.intern(&b);
        let order = u.sorted_order();
        // String comparison: "op10" < "op2" lexicographically.
        assert_eq!(order, vec![ib, ia]);
    }

    /// ⟨rev3⟩ `key_rank` is the position of each id in `(effect_key,
    /// operation_id)` order — a TOTAL order (no ties) — and never a remap of
    /// the id itself (the id stays intern-order; `key_rank` is a parallel
    /// side-table).
    #[test]
    fn freeze_key_rank_is_the_sorted_order_inverse() {
        let mut u = GrowingEffectUniverse::new();
        // Intern out of key order: "Zeta" > "Alpha" > "Middle" by effect_key.
        let zeta = u.intern(&ident("Zeta", "t1", "op1", TempStateKind::Known(true)));
        let alpha = u.intern(&ident("Alpha", "t1", "op2", TempStateKind::Unknown));
        let middle = u.intern(&ident("Middle", "t2", "op3", TempStateKind::Known(false)));
        let order_before = u.sorted_order();
        let frozen = u.freeze();
        // key_rank[id] == position of id in sorted_order.
        for (rank, &id) in order_before.iter().enumerate() {
            assert_eq!(frozen.key_rank(id), rank as u32);
        }
        // Ascending key_rank reproduces sorted_order.
        let mut by_rank = vec![alpha, middle, zeta];
        by_rank.sort_by_key(|&id| frozen.key_rank(id));
        assert_eq!(by_rank, order_before);
        // EffectIds are NOT reassigned by freeze.
        assert_eq!(frozen.identity(zeta).op, "Zeta");
        assert_eq!(frozen.identity(alpha).op, "Alpha");
        assert_eq!(frozen.identity(middle).op, "Middle");
    }

    /// Post-freeze `get` of an identity that was NEVER interned returns
    /// `None` (spec lifecycle step 3 — the checked lookup a debug-assert/test
    /// gates on). Structurally, `FrozenEffectUniverse` exposes no `intern`, so
    /// this identity can never be minted post-freeze.
    #[test]
    fn frozen_get_of_uninterned_identity_is_none() {
        let mut u = GrowingEffectUniverse::new();
        let known = ident("Insert", "t1", "op1", TempStateKind::Known(true));
        let id = u.intern(&known);
        let frozen = u.freeze();
        assert_eq!(frozen.get(&known), Some(id));
        let never = ident("Delete", "t9", "op9", TempStateKind::Unknown);
        assert_eq!(
            frozen.get(&never),
            None,
            "an un-interned identity has no id post-freeze"
        );
    }
}
