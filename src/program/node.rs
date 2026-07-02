//! Canonical, app-qualified identity for whole-program graph nodes.

use crate::snapshot::AppId;
pub use al_syntax::ir::ObjectKind;
use std::collections::HashMap;

/// Interned handle for an `AppId` (cheap to copy/compare/sort).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct AppRef(pub u32);

/// Interns `AppId`s by their FULL identity (guid+name+publisher+version) — guid
/// is empty for deps today, so we never key on guid alone.
#[derive(Default)]
pub struct AppRegistry {
    by_key: HashMap<(String, String, String, String), AppRef>,
    apps: Vec<AppId>,
}

impl AppRegistry {
    pub fn intern(&mut self, app: &AppId) -> AppRef {
        let key = (
            app.guid.clone(),
            app.name.clone(),
            app.publisher.clone(),
            app.version.clone(),
        );
        if let Some(&r) = self.by_key.get(&key) {
            return r;
        }
        let r = AppRef(u32::try_from(self.apps.len()).expect("app arena overflow"));
        self.apps.push(app.clone());
        self.by_key.insert(key, r);
        r
    }

    pub fn resolve(&self, r: AppRef) -> &AppId {
        &self.apps[r.0 as usize]
    }

    /// Look up an app ref without panicking if not found (index out of range).
    pub fn try_resolve(&self, r: AppRef) -> Option<&AppId> {
        self.apps.get(r.0 as usize)
    }

    /// Find the first interned app whose `name` matches case-insensitively.
    pub fn find_by_name(&self, name: &str) -> Option<AppRef> {
        self.apps
            .iter()
            .position(|id| id.name.eq_ignore_ascii_case(name))
            .map(|i| AppRef(i as u32))
    }

    /// Look up an interned app by full identity (guid + name + publisher +
    /// version) without mutating the registry.  Returns `None` if the app was
    /// never interned via [`Self::intern`].
    pub fn find(&self, app: &AppId) -> Option<AppRef> {
        let key = (
            app.guid.clone(),
            app.name.clone(),
            app.publisher.clone(),
            app.version.clone(),
        );
        self.by_key.get(&key).copied()
    }
}

/// Object key: prefer the numeric id; fall back to the (lowercased) name for
/// id-less objects (extension objects, or where the IR has no number).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub enum ObjKey {
    Id(i64),
    Name(String),
}

/// Canonical identity of an AL object within one app.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct ObjectNodeId {
    pub app: AppRef,
    pub kind: ObjectKind,
    pub key: ObjKey,
}

impl ObjectNodeId {
    /// True when this object was declared with numeric id `n`.
    pub fn id_equals_number(&self, n: i64) -> bool {
        matches!(&self.key, ObjKey::Id(k) if *k == n)
    }
}

/// Canonical identity of a routine within one object. `name_lc` is lowercased
/// (AL identifiers are case-insensitive). `enclosing_member_lc` is the
/// lowercased name of the field/member that a member-trigger is nested in (e.g.
/// the field name for a table-field `OnValidate`); `None` for regular
/// procedures and object-level triggers. This discriminator prevents same-named
/// member triggers on different fields from colliding in maps/sets.
/// `params_count` is the parameter count of the routine, used to distinguish
/// AL overloads (same name, different arity) so each overload maps to a unique
/// node. For SymbolOnly (dep boundary) routines, `params_count` is the real
/// ABI `Parameters[].len()` (Task 1) — arity checking is NOT bypassed for
/// SymbolOnly in resolution (`resolve_in_object` applies the same arity-exact
/// discipline to every tier). The one exception is a SymbolOnly routine whose
/// `Parameters` field was absent/unparseable in `SymbolReference.json`: its
/// arity is genuinely UNKNOWN, so ingestion (`abi_ingest::UNKNOWN_ARITY`)
/// gives it a sentinel `params_count` that can never equal a real call's
/// arity — it exists (for name-only lookups) but never arity-matches.
/// `sig_fp` is a stable FNV-1a fingerprint of the parameter type-text sequence.
/// `0` for source-bearing routines. Non-zero for SymbolOnly ABI routines when
/// two routines share the same `name_lc` AND `params_count` but differ in param
/// types. Together with `name_lc` and `params_count`, extends `RoutineNodeId` to
/// a total discriminator for ABI overloads.
/// `None < Some(…)` under `Ord`, so object-level triggers sort before field
/// triggers — intentional.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct RoutineNodeId {
    pub object: ObjectNodeId,
    pub name_lc: String,
    pub enclosing_member_lc: Option<String>,
    pub params_count: usize,
    /// Stable fingerprint of the parameter type-text sequence (FNV-1a hash).
    /// `0` for source-bearing routines. Non-zero for SymbolOnly ABI routines when
    /// two routines share the same `name_lc` AND `params_count` but differ in
    /// param types. Extends `RoutineNodeId` to a total discriminator.
    pub sig_fp: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use al_syntax::ir::ObjectKind;

    fn app(name: &str, ver: &str) -> crate::snapshot::AppId {
        crate::snapshot::AppId {
            guid: String::new(),
            name: name.into(),
            publisher: "P".into(),
            version: ver.into(),
        }
    }

    #[test]
    fn app_ref_interns_by_full_identity_even_with_empty_guid() {
        let mut reg = AppRegistry::default();
        let a = reg.intern(&app("Core", "29.0.0.0"));
        let a2 = reg.intern(&app("Core", "29.0.0.0"));
        let b = reg.intern(&app("Core", "28.0.0.0")); // different version
        assert_eq!(a, a2);
        assert_ne!(a, b);
        assert_eq!(reg.resolve(a).name, "Core");
    }

    #[test]
    fn object_node_id_distinguishes_same_name_across_apps() {
        let mut reg = AppRegistry::default();
        let a = reg.intern(&app("AppA", "1.0.0.0"));
        let b = reg.intern(&app("AppB", "1.0.0.0"));
        let na = ObjectNodeId {
            app: a,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Name("Sales-Post".into()),
        };
        let nb = ObjectNodeId {
            app: b,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Name("Sales-Post".into()),
        };
        assert_ne!(
            na, nb,
            "same type+name in different apps must be distinct nodes"
        );
    }
}
