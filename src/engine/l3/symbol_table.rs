//! L3 workspace symbol table — Rust port of al-sem's
//! `src/resolve/symbol-table.ts` (`buildSymbolTable`).
//!
//! A read-only lookup index over the assembled L3 workspace model. All name
//! lookups are case-insensitive (AL identifiers are case-insensitive).
//!
//! COLLISION RESOLUTION IS ORDER-DEPENDENT (critical): every name/number index
//! uses `HashMap::insert` (LAST-wins), iterating in the assembled order — so the
//! LAST object/table with a colliding key wins, matching al-sem's `Map.set`.
//! Build this over a workspace assembled in al-sem's deterministic ingestion
//! order (POSIX-path-sorted files → per-file document order) or collisions
//! resolve differently.
//!
//! Routines are keyed `${objectId}::${name.toLowerCase()}` with overload lists
//! pre-sorted by routine id (byte-order). R2b's overload resolution relies on
//! this exact key + sort — locked here in R2a.

use super::l3_workspace::{L3Object, L3Routine, L3Table};
use std::collections::HashMap;

/// Strip surrounding double-quotes from an interface name for case/quote-
/// insensitive matching (mirrors `normalizeInterfaceName`).
fn normalize_interface_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.len() > 1 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        trimmed[1..trimmed.len() - 1].to_lowercase()
    } else {
        trimmed.to_lowercase()
    }
}

/// A read-only lookup index over a workspace L3 model. Holds owned clones so it
/// is independent of the source model's lifetime (the resolver clones cheaply).
pub struct SymbolTable {
    /// `${objectType_lc}/${objectNumber}` → object index.
    by_type_number: HashMap<String, usize>,
    /// `${objectType_lc}/${name_lc}` → object index.
    by_type_name: HashMap<String, usize>,
    objects: Vec<L3Object>,

    /// `${name_lc}` → table index.
    tables_by_name: HashMap<String, usize>,
    /// `${tableId}` → table index.
    tables_by_id: HashMap<String, usize>,
    tables: Vec<L3Table>,

    /// `${objectId}::${name_lc}` → routine index (single, LAST-wins).
    routine_by_key: HashMap<String, usize>,
    /// `${objectId}::${name_lc}` → ALL overloads, sorted by id.
    routines_by_object_and_name: HashMap<String, Vec<usize>>,
    /// `${objectId}` → all routine indices in that object (document order).
    routines_by_object: HashMap<String, Vec<usize>>,
    routines: Vec<L3Routine>,

    /// Interface name (normalized) → codeunit implementer indices, sorted by id.
    codeunit_implementers: HashMap<String, Vec<usize>>,
    /// Interface name (normalized) → enum implementer indices, sorted by id.
    enum_implementers: HashMap<String, Vec<usize>>,

    impls_knowledge: ImplsKnowledge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImplsKnowledge {
    Complete,
    Partial,
}

impl SymbolTable {
    /// Build the symbol table over an assembled workspace. The slices MUST be in
    /// al-sem's deterministic ingestion order (collision resolution depends on it).
    pub fn build(objects: &[L3Object], tables: &[L3Table], routines: &[L3Routine]) -> SymbolTable {
        let objects = objects.to_vec();
        let tables = tables.to_vec();
        let routines = routines.to_vec();

        // --- object indexes (LAST-wins) -------------------------------------
        let mut by_type_number = HashMap::new();
        let mut by_type_name = HashMap::new();
        for (i, o) in objects.iter().enumerate() {
            by_type_number.insert(
                format!("{}/{}", o.object_type.to_lowercase(), o.object_number),
                i,
            );
            by_type_name.insert(
                format!("{}/{}", o.object_type.to_lowercase(), o.name.to_lowercase()),
                i,
            );
        }

        // --- table indexes (LAST-wins, REAL over stub) ----------------------
        // G-5: a `tableextension` stub's id reuses the EXTENSION's own object
        // number (`${appGuid}/table/${extNumber}`), which collides with a real
        // table sharing that number. A real table always wins the collision
        // (by id AND by name); within the same kind LAST-wins is preserved.
        let mut tables_by_name: HashMap<String, usize> = HashMap::new();
        let mut tables_by_id: HashMap<String, usize> = HashMap::new();
        for (i, t) in tables.iter().enumerate() {
            let name_key = t.name.to_lowercase();
            let keep_prev_name = tables_by_name
                .get(&name_key)
                .is_some_and(|&p| !tables[p].is_extension_stub && t.is_extension_stub);
            if !keep_prev_name {
                tables_by_name.insert(name_key, i);
            }
            let keep_prev_id = tables_by_id
                .get(&t.id)
                .is_some_and(|&p| !tables[p].is_extension_stub && t.is_extension_stub);
            if !keep_prev_id {
                tables_by_id.insert(t.id.clone(), i);
            }
        }

        // --- routine indexes ------------------------------------------------
        let mut routine_by_key = HashMap::new();
        let mut routines_by_object: HashMap<String, Vec<usize>> = HashMap::new();
        let mut routines_by_object_and_name: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, r) in routines.iter().enumerate() {
            let key = format!("{}::{}", r.object_id, r.name.to_lowercase());
            routine_by_key.insert(key.clone(), i); // LAST-wins
            routines_by_object
                .entry(r.object_id.clone())
                .or_default()
                .push(i);
            routines_by_object_and_name.entry(key).or_default().push(i);
        }
        // Sort all overload lists by routine id (byte-order).
        for list in routines_by_object_and_name.values_mut() {
            list.sort_by(|&a, &b| routines[a].id.cmp(&routines[b].id));
        }

        // --- interface implementer indexes ----------------------------------
        let mut codeunit_implementers: HashMap<String, Vec<usize>> = HashMap::new();
        let mut enum_implementers: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, o) in objects.iter().enumerate() {
            let Some(ifaces) = &o.implements_interfaces else {
                continue; // undefined = unknown, skip
            };
            for iface in ifaces {
                let key = normalize_interface_name(iface);
                if o.object_type.to_lowercase() == "enum" {
                    enum_implementers.entry(key).or_default().push(i);
                } else {
                    codeunit_implementers.entry(key).or_default().push(i);
                }
            }
        }
        for list in codeunit_implementers.values_mut() {
            list.sort_by(|&a, &b| objects[a].id.cmp(&objects[b].id));
        }
        for list in enum_implementers.values_mut() {
            list.sort_by(|&a, &b| objects[a].id.cmp(&objects[b].id));
        }

        // --- per-app interface-knowledge detection --------------------------
        // appGuid → hasAnyDefined. "partial" iff at least one app is "unknown".
        let mut app_knowledge: HashMap<String, bool> = HashMap::new();
        for o in &objects {
            let has = o.implements_interfaces.is_some();
            app_knowledge
                .entry(o.app_guid.clone())
                .and_modify(|cur| *cur = *cur || has)
                .or_insert(has);
        }
        let impls_knowledge = if app_knowledge.values().any(|&v| !v) {
            ImplsKnowledge::Partial
        } else {
            ImplsKnowledge::Complete
        };

        SymbolTable {
            by_type_number,
            by_type_name,
            objects,
            tables_by_name,
            tables_by_id,
            tables,
            routine_by_key,
            routines_by_object_and_name,
            routines_by_object,
            routines,
            codeunit_implementers,
            enum_implementers,
            impls_knowledge,
        }
    }

    pub fn object_by_type_number(
        &self,
        object_type: &str,
        object_number: i64,
    ) -> Option<&L3Object> {
        let key = format!("{}/{}", object_type.to_lowercase(), object_number);
        self.by_type_number.get(&key).map(|&i| &self.objects[i])
    }

    pub fn object_by_type_name(&self, object_type: &str, name: &str) -> Option<&L3Object> {
        let key = format!("{}/{}", object_type.to_lowercase(), name.to_lowercase());
        self.by_type_name.get(&key).map(|&i| &self.objects[i])
    }

    pub fn table_by_name(&self, name: &str) -> Option<&L3Table> {
        self.tables_by_name
            .get(&name.to_lowercase())
            .map(|&i| &self.tables[i])
    }

    pub fn table_by_id(&self, id: &str) -> Option<&L3Table> {
        self.tables_by_id.get(id).map(|&i| &self.tables[i])
    }

    pub fn routine_in_object(&self, object_id: &str, routine_name: &str) -> Option<&L3Routine> {
        let key = format!("{}::{}", object_id, routine_name.to_lowercase());
        self.routine_by_key.get(&key).map(|&i| &self.routines[i])
    }

    pub fn routines_in_object(&self, object_id: &str) -> Vec<&L3Routine> {
        self.routines_by_object
            .get(object_id)
            .map(|v| v.iter().map(|&i| &self.routines[i]).collect())
            .unwrap_or_default()
    }

    /// ALL routines in the object with this name (overloads), sorted by id.
    pub fn routines_in_object_by_name(
        &self,
        object_id: &str,
        routine_name: &str,
    ) -> Vec<&L3Routine> {
        let key = format!("{}::{}", object_id, routine_name.to_lowercase());
        self.routines_by_object_and_name
            .get(&key)
            .map(|v| v.iter().map(|&i| &self.routines[i]).collect())
            .unwrap_or_default()
    }

    pub fn objects_implementing(&self, interface_name: &str) -> Vec<&L3Object> {
        self.codeunit_implementers
            .get(&normalize_interface_name(interface_name))
            .map(|v| v.iter().map(|&i| &self.objects[i]).collect())
            .unwrap_or_default()
    }

    pub fn enum_implementers(&self, interface_name: &str) -> Vec<&L3Object> {
        self.enum_implementers
            .get(&normalize_interface_name(interface_name))
            .map(|v| v.iter().map(|&i| &self.objects[i]).collect())
            .unwrap_or_default()
    }

    pub fn interface_impls_knowledge(&self) -> ImplsKnowledge {
        self.impls_knowledge
    }
}
