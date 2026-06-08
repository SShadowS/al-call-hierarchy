//! `fingerprintOf` — port of al-sem `src/projection/finding-fingerprint.ts`.
//!
//! A stable edit-survival key. CRITICAL: it hashes the INTERNAL `rootCauseKey` +
//! INTERNAL `affectedTables` — the finding BEFORE the stable projection. So the
//! detector computes it over internal ids, and the projection copies it verbatim.
//!
//!   sha256( detector | objectType/objectNumber | routineName |
//!           affectedTables.join(",") | rootCauseKey )  → first 16 hex chars
//!
//! `objectType/objectNumber` is the OWNING object of the finding's
//! `primaryLocation.enclosingRoutineId` (empty when unresolved); `routineName` is
//! that routine's name (empty when unresolved). Uses plain UTF-8 sha256 (Node's
//! `createHash("sha256").update(str)`), NOT the UTF-16 length-prefixed framing.

use std::collections::HashMap;

use crate::engine::ids::sha256_hex;
use crate::engine::l3::l3_workspace::{L3Object, L3Routine};
use crate::engine::l5::finding::Finding;

/// Per-model id indexes for the fingerprint (routine-by-internal-id +
/// object-by-internal-id). Built once per run.
pub struct FingerprintIndex<'a> {
    routines_by_id: HashMap<&'a str, &'a L3Routine>,
    objects_by_id: HashMap<&'a str, &'a L3Object>,
}

impl<'a> FingerprintIndex<'a> {
    pub fn build(routines: &'a [L3Routine], objects: &'a [L3Object]) -> Self {
        let routines_by_id = routines.iter().map(|r| (r.id.as_str(), r)).collect();
        let objects_by_id = objects.iter().map(|o| (o.id.as_str(), o)).collect();
        FingerprintIndex {
            routines_by_id,
            objects_by_id,
        }
    }

    /// Compute the finding's fingerprint over its INTERNAL ids. Returns the first
    /// 16 hex chars of `sha256(parts.join("|"))`.
    pub fn fingerprint_of(&self, finding: &Finding) -> String {
        let routine = self
            .routines_by_id
            .get(finding.primary_location.enclosing_routine_id.as_str());
        let obj = routine.and_then(|r| self.objects_by_id.get(r.object_id.as_str()));

        let obj_part = match obj {
            Some(o) => format!("{}/{}", o.object_type, o.object_number),
            None => String::new(),
        };
        let routine_name = routine.map(|r| r.name.clone()).unwrap_or_default();

        let parts = [
            finding.detector.clone(),
            obj_part,
            routine_name,
            finding.affected_tables.join(","),
            finding.root_cause_key.clone(),
        ];
        let joined = parts.join("|");
        sha256_hex(&joined)[..16].to_string()
    }
}
