//! `EffectUniverse` — interns the structured db-effect identity `(op, table_id,
//! operation_id, TempStateKind)` as a compact `EffectId(u32)`.
//!
//! This is the shared vocabulary the bitvector solver (later tasks) builds its
//! per-member presence sets over: every distinct effect fact seen anywhere in a
//! workspace gets ONE id, so a routine's db-effect set becomes a bitset over
//! `EffectId` rather than a `HashMap<String, DbEffect>`. `sorted_order()`
//! reproduces the EXACT `(effect_key, operation_id)` materialization order that
//! `summary_runner.rs` (around lines 507-510) already produces for the old
//! Jacobi solver's `Vec<DbEffect>` — this determinism is load-bearing: later
//! tasks depend on the new solver's materialized output being byte-identical
//! to the old one.
//!
//! Deliberately EXCLUDES from the interned identity:
//!   - `via` (provenance) — reconstructed by a separate post-pass (Task 6).
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

/// Interner: `EffectIdentity -> EffectId`, plus the reverse `Vec` for
/// `EffectId -> EffectIdentity` lookup. Lazily grows as PD substitution
/// invents new effect variants during solving — never frozen mid-solve.
pub struct EffectUniverse {
    by_identity: HashMap<EffectIdentity, EffectId>,
    by_id: Vec<EffectIdentity>,
}

impl Default for EffectUniverse {
    fn default() -> Self {
        Self::new()
    }
}

impl EffectUniverse {
    pub fn new() -> Self {
        EffectUniverse {
            by_identity: HashMap::new(),
            by_id: Vec::new(),
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
        self.by_id.push(identity.clone());
        self.by_identity.insert(identity.clone(), id);
        id
    }

    /// Look up an already-interned identity without creating one.
    pub fn get(&self, identity: &EffectIdentity) -> Option<EffectId> {
        self.by_identity.get(identity).copied()
    }

    /// The structured identity for an interned id. Panics if `id` was never
    /// produced by this universe's `intern` — every `EffectId` in circulation
    /// is expected to have come from exactly one `EffectUniverse`.
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
    /// `Vec<DbEffect>` sort (`summary_runner.rs:507-510`) exactly, so later
    /// tasks can materialize `DbEffect`s in this order and get a byte-identical
    /// result to the Jacobi solver's output.
    pub fn sorted_order(&self) -> Vec<EffectId> {
        let mut ids: Vec<EffectId> = (0..self.by_id.len() as u32).map(EffectId).collect();
        ids.sort_by(|&a, &b| {
            let ia = self.identity(a);
            let ib = self.identity(b);
            self.effect_key(a)
                .cmp(&self.effect_key(b))
                .then_with(|| ia.operation_id.cmp(&ib.operation_id))
        });
        ids
    }

    /// The full `effect_key` string for an id — lazy, computed only when
    /// needed for materialization/projection/sorting (never stored per-id).
    pub fn effect_key(&self, id: EffectId) -> String {
        let identity = self.identity(id);
        effect_key_of(
            &identity.op,
            &identity.table_id,
            &identity.operation_id,
            &identity.temp,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intern_is_deterministic_and_sorted_order_matches_effect_key() {
        use crate::engine::l4::effect_lattice::TempStateKind;
        let mut u = EffectUniverse::new();
        let a = EffectIdentity {
            op: "Insert".into(),
            table_id: "t2".into(),
            operation_id: "op9".into(),
            temp: TempStateKind::Known(true),
        };
        let b = EffectIdentity {
            op: "Insert".into(),
            table_id: "t1".into(),
            operation_id: "op1".into(),
            temp: TempStateKind::Known(true),
        };
        let ia = u.intern(&a);
        let ib = u.intern(&b);
        assert_eq!(u.intern(&a), ia, "re-intern stable");
        assert_ne!(ia, ib);
        // sorted_order must be by effect_key then operation_id, NOT insertion order.
        let order = u.sorted_order();
        let keys: Vec<String> = order.iter().map(|&e| u.effect_key(e)).collect();
        let mut expected = keys.clone();
        expected.sort();
        assert_eq!(keys, expected, "sorted_order yields effect_key ascending");
    }

    #[test]
    fn get_returns_none_before_intern_and_some_after() {
        let mut u = EffectUniverse::new();
        let a = EffectIdentity {
            op: "Modify".into(),
            table_id: "t1".into(),
            operation_id: "op1".into(),
            temp: TempStateKind::Unknown,
        };
        assert_eq!(u.get(&a), None);
        let id = u.intern(&a);
        assert_eq!(u.get(&a), Some(id));
    }

    #[test]
    fn identity_roundtrips_through_intern() {
        let mut u = EffectUniverse::new();
        let a = EffectIdentity {
            op: "Delete".into(),
            table_id: "t3".into(),
            operation_id: "op7".into(),
            temp: TempStateKind::ParameterDependent(2),
        };
        let id = u.intern(&a);
        assert_eq!(u.identity(id), &a);
        assert_eq!(u.len(), 1);
    }

    #[test]
    fn sorted_order_breaks_effect_key_ties_by_operation_id() {
        // Two distinct identities that happen to share the same effect_key
        // are impossible in practice UNLESS operation_id differs (operation_id
        // is part of effect_key) — but the sort must still order by
        // operation_id as its secondary key, so construct entries whose
        // effect_key differs only in trailing operation_id ordering.
        let mut u = EffectUniverse::new();
        let a = EffectIdentity {
            op: "Insert".into(),
            table_id: "t1".into(),
            operation_id: "op2".into(),
            temp: TempStateKind::Known(true),
        };
        let b = EffectIdentity {
            op: "Insert".into(),
            table_id: "t1".into(),
            operation_id: "op10".into(),
            temp: TempStateKind::Known(true),
        };
        let ia = u.intern(&a);
        let ib = u.intern(&b);
        let order = u.sorted_order();
        // String comparison: "op10" < "op2" lexicographically.
        assert_eq!(order, vec![ib, ia]);
    }
}
