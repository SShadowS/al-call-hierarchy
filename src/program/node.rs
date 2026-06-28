//! Canonical, app-qualified identity for whole-program graph nodes.

use crate::snapshot::AppId;
use al_syntax::ir::ObjectKind;
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

/// Canonical identity of a routine within one object. `name_lc` is lowercased
/// (AL identifiers are case-insensitive).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct RoutineNodeId {
    pub object: ObjectNodeId,
    pub name_lc: String,
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
