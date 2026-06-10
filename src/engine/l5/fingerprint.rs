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
//!
//! ## Routine-id stabilization (edit-stable fingerprint)
//!
//! The `rootCauseKey` embeds INTERNAL routine ids — directly, or as the PREFIX of
//! operation/callsite/loop ids (`${routineId}/suffix`). For SOURCE routines that
//! internal id is `${modelInstanceId}/${keyHash}`; for DEPENDENCY routines it is
//! `dep:<artifactKey>/<keyHash>`. Either form is unstable across edits/cache bumps:
//! `modelInstanceId` is content-derived (shifts when any workspace `.al` file is
//! added/removed/renamed) in the content-MII gate path, and `<artifactKey>` is
//! cache-derived (drifts on every cache bump). Hashing either verbatim makes the
//! fingerprint unstable for an identical logical finding.
//!
//! To match al-sem's NEW stabilized fingerprint (`src/projection/finding-fingerprint.ts`),
//! we build a map from EVERY routine's internal id → its `StableRoutineId`
//! (`${appGuid}:${objectType}:${objectNumber}#${normalizedSignatureHash}`) — for ALL
//! routines, SOURCE and DEPENDENCY alike — and substitute every internal routine-id
//! occurrence in the `rootCauseKey` before hashing. Because operation/callsite/loop
//! ids carry the routine id as a prefix, replacing the routine-id substring preserves
//! the `/suffix`. The result is `modelInstanceId`- and cache-INDEPENDENT (edit-stable).
//!
//! Routines whose `normalized_signature_hash` is empty are skipped (no stable form to
//! swap to — `to_stable_routine_id_from_parts` would emit a degenerate trailing `#`).
//! For the r0-pinned (r4 differential) path the engine pins `modelInstanceId = "r0"`,
//! so source ids are already reproducible; the substitution still applies and both
//! paths stabilize to the SAME `StableRoutineId`. This unifies the prior dep-only
//! special case into one all-routines map.

use crate::engine::ids::sha256_hex;
use crate::engine::l3::l3_workspace::{L3Object, L3Routine};
use crate::engine::l5::finding::Finding;
use std::collections::HashMap;

/// Per-model id indexes for the fingerprint (routine-by-internal-id +
/// object-by-internal-id + all-routines stabilization map). Built once per run.
pub struct FingerprintIndex<'a> {
    routines_by_id: HashMap<&'a str, &'a L3Routine>,
    objects_by_id: HashMap<&'a str, &'a L3Object>,
    /// EVERY routine's internal id → stable routine id (source AND dep). Used to
    /// substitute internal routine-id occurrences in the `rootCauseKey` before
    /// hashing, making the fingerprint edit-/cache-independent. EMPTY only for an
    /// empty model → `fingerprint_of` is then a no-op.
    stable_by_id: HashMap<String, String>,
}

impl<'a> FingerprintIndex<'a> {
    /// Build the fingerprint index. Maps EVERY routine's internal id to its stable
    /// id (source AND dependency) so that all internal routine-id occurrences in a
    /// `rootCauseKey` are stabilized before hashing. Routines whose
    /// `normalized_signature_hash` is empty are skipped (no stable form). This is
    /// the single, unified path — there is no longer a dep-only variant.
    pub fn build(routines: &'a [L3Routine], objects: &'a [L3Object]) -> Self {
        let routines_by_id: HashMap<&str, &L3Routine> =
            routines.iter().map(|r| (r.id.as_str(), r)).collect();
        let objects_by_id: HashMap<&str, &L3Object> =
            objects.iter().map(|o| (o.id.as_str(), o)).collect();

        // Map every routine's INTERNAL id → its modelInstanceId-/cache-independent
        // StableRoutineId. Skip routines with an empty normalized_signature_hash:
        // their stable form would be a degenerate trailing `#` (no stable form to
        // swap to — mirrors al-sem skipping `normalizedSignatureHash === ""`).
        let stable_by_id: HashMap<String, String> = routines
            .iter()
            .filter(|r| !r.normalized_signature_hash.is_empty())
            .map(|r| (r.id.clone(), r.stable_routine_id.clone()))
            .collect();

        FingerprintIndex {
            routines_by_id,
            objects_by_id,
            stable_by_id,
        }
    }

    /// Compute the finding's fingerprint. Internal routine ids in `rootCauseKey`
    /// (directly, or as the prefix of operation/callsite/loop ids) are replaced with
    /// their stable ids before hashing (mirrors al-sem's stabilizing substitution —
    /// see module docs). Returns the first 16 hex chars of `sha256(parts.join("|"))`.
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

        // Stabilize every internal routine id embedded in the rootCauseKey (mirrors
        // al-sem's stabilizing reduce). For an empty model stable_by_id is empty, so
        // this branch is skipped and the key is used verbatim.
        let stable_root_cause_key = if self.stable_by_id.is_empty() {
            finding.root_cause_key.clone()
        } else {
            // Single left-to-right pass: try each routine id at each position. Sort
            // by key-length desc so longer keys shadow shorter prefixes (same
            // invariant as make_stable_finding_id_fn). Internal RoutineIds are
            // fixed-structure (no id is a substring of another), so non-overlapping
            // longest-first replacement is deterministic.
            let mut sorted_entries: Vec<(&String, &String)> = self.stable_by_id.iter().collect();
            sorted_entries.sort_by(|a, b| {
                b.0.len()
                    .cmp(&a.0.len())
                    .then_with(|| a.0.as_str().cmp(b.0.as_str()))
            });

            let key = finding.root_cause_key.as_str();
            let len = key.len(); // byte length
            let mut out = String::with_capacity(len);
            let mut pos = 0usize;
            'outer: while pos < len {
                for (k, v) in &sorted_entries {
                    if key[pos..].starts_with(k.as_str()) {
                        out.push_str(v.as_str());
                        pos += k.len();
                        continue 'outer;
                    }
                }
                // Advance by one Unicode scalar value (char-boundary-safe).
                // Routine ids are ASCII so this is always 1 byte in practice; the
                // &str slice approach is safe even for non-ASCII rootCauseKeys.
                let ch = key[pos..]
                    .chars()
                    .next()
                    .expect("valid UTF-8 non-empty slice");
                out.push(ch);
                pos += ch.len_utf8();
            }
            out
        };

        let parts = [
            finding.detector.clone(),
            obj_part,
            routine_name,
            finding.affected_tables.join(","),
            stable_root_cause_key,
        ];
        let joined = parts.join("|");
        sha256_hex(&joined)[..16].to_string()
    }
}

// ---------------------------------------------------------------------------
// Native oracles — #[cfg(test)]
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ids::sha256_hex;
    use crate::engine::l3::l3_workspace::L3Routine;
    use crate::engine::l5::finding::{Evidence, FindingConfidence, SourceAnchor};

    fn dummy_anchor(enclosing: &str) -> SourceAnchor {
        SourceAnchor {
            source_unit_id: "ws:test.al".to_string(),
            start_line: 1,
            start_column: 0,
            end_line: 1,
            end_column: 10,
            enclosing_routine_id: enclosing.to_string(),
            syntax_kind: "procedure".to_string(),
            normalized_text_hash: None,
            leading_context_hash: None,
            trailing_context_hash: None,
        }
    }

    fn dummy_finding(detector: &str, root_cause_key: &str, enclosing: &str) -> Finding {
        Finding {
            id: root_cause_key.to_string(),
            root_cause_key: root_cause_key.to_string(),
            detector: detector.to_string(),
            title: "Test finding".to_string(),
            root_cause: "root cause".to_string(),
            severity: "info".to_string(),
            confidence: FindingConfidence {
                level: "possible".to_string(),
                capped_by: None,
                evidence: Vec::new(),
            },
            primary_location: dummy_anchor(enclosing),
            evidence_path: Vec::new(),
            additional_paths: None,
            affected_objects: Vec::new(),
            affected_tables: Vec::new(),
            fix_options: Vec::new(),
            provenance: vec![Evidence {
                source: "tree-sitter".to_string(),
                note: None,
            }],
            actionable_anchor: None,
            fingerprint: None,
            event_kind: None,
            cross_extension_subscribers: None,
        }
    }

    fn minimal_routine(id: &str, object_id: &str, name: &str) -> L3Routine {
        L3Routine {
            id: id.to_string(),
            stable_routine_id: format!("stable::{id}"),
            object_id: object_id.to_string(),
            object_type: "Codeunit".to_string(),
            name: name.to_string(),
            kind: "procedure".to_string(),
            attributes_parsed: Vec::new(),
            app_guid: "app".to_string(),
            object_number: 1,
            // Non-empty so the routine is included in the all-routines stable map
            // (routines with an empty hash are skipped — no stable form).
            normalized_signature_hash: "sig".to_string(),
            body_available: true,
            parse_incomplete: false,
            record_variables: Vec::new(),
            record_operations: Vec::new(),
            field_accesses: Vec::new(),
            variables: Vec::new(),
            parameters: Vec::new(),
            access_modifier: None,
            return_type: None,
            call_sites: Vec::new(),
            operation_sites: Vec::new(),
            statement_tree: None,
            loops: Vec::new(),
            source_anchor: crate::engine::l2::features::PAnchor {
                source_unit_id: "ws:test.al".to_string(),
                start_line: 0,
                start_column: 0,
                end_line: 0,
                end_column: 0,
                syntax_kind: "procedure".to_string(),
            },
            identifier_references: Vec::new(),
            unreachable_statements: Vec::new(),
            has_branching: false,
            var_assignments: Vec::new(),
            condition_references: Vec::new(),
            enclosing_member: None,
            originating_object: None,
            enclosing_member_range: None,
        }
    }

    fn minimal_object(id: &str, object_type: &str, object_number: i64) -> L3Object {
        L3Object {
            id: id.to_string(),
            app_guid: "app".to_string(),
            object_type: object_type.to_string(),
            object_number,
            name: format!("Obj{object_number}"),
            source_table_name: None,
            extends_target_name: None,
            implements_interfaces: Some(Vec::new()),
            object_subtype: None,
            page_type: None,
            inherent_commit_behavior: None,
        }
    }

    // -----------------------------------------------------------------------
    // Oracle 1 — a rootCauseKey that embeds NO routine id is hashed verbatim
    // (the only routine's id is not a substring of the key → no substitution).
    // -----------------------------------------------------------------------
    #[test]
    fn fingerprint_with_no_embedded_routine_id_is_sha256_of_parts() {
        let r = minimal_routine("r1", "obj/Codeunit/50100", "MyProc");
        let o = minimal_object("obj/Codeunit/50100", "Codeunit", 50100);
        let routines = vec![r];
        let objects = vec![o];
        let idx = FingerprintIndex::build(&routines, &objects);

        let finding = dummy_finding("d17-min-version-drift", "d17/some-guid", "r1");
        let fp = idx.fingerprint_of(&finding);

        // Verify it is 16 hex chars.
        assert_eq!(fp.len(), 16, "fingerprint must be 16 hex chars: {fp}");
        assert!(
            fp.chars().all(|c| c.is_ascii_hexdigit()),
            "must be hex: {fp}"
        );

        // Reproducibility: calling again yields the same value.
        let fp2 = idx.fingerprint_of(&finding);
        assert_eq!(fp, fp2, "fingerprint must be deterministic");

        // "r1" is not a substring of "d17/some-guid", so the key is verbatim:
        // sha256("d17-min-version-drift|Codeunit/50100|MyProc||d17/some-guid")
        let expected_parts = "d17-min-version-drift|Codeunit/50100|MyProc||d17/some-guid";
        let expected = &sha256_hex(expected_parts)[..16];
        assert_eq!(
            fp, expected,
            "fingerprint must match manual sha256 of parts"
        );
    }

    // -----------------------------------------------------------------------
    // Oracle 2 — a DEP routine id embedded in rootCauseKey is replaced with its
    // stable id before hashing (the cross-app case, now via the unified build).
    // -----------------------------------------------------------------------
    #[test]
    fn dep_id_in_root_cause_key_is_substituted_with_stable_id() {
        let internal_dep_id = "r0/dep:abc123/sig456hash";
        let stable_dep_id =
            "app:Codeunit:99#aabbccdd0011223344556677889900aabbccdd0011223344556677889900aabb";

        let mut r_dep = minimal_routine(internal_dep_id, "dep/Codeunit/99", "DepProc");
        r_dep.stable_routine_id = stable_dep_id.to_string();
        let o = minimal_object("dep/Codeunit/99", "Codeunit", 99);

        let mut r_ws = minimal_routine("ws_routine", "ws/Codeunit/50100", "WSProc");
        r_ws.stable_routine_id = "ws_stable".to_string();
        let o_ws = minimal_object("ws/Codeunit/50100", "Codeunit", 50100);

        let routines2 = vec![r_dep.clone(), r_ws.clone()];
        let objects2 = vec![o.clone(), o_ws.clone()];
        let idx = FingerprintIndex::build(&routines2, &objects2);

        // rootCauseKey embeds the internal dep id (as d16 does)
        let root_cause_key = format!("d16/{internal_dep_id}");
        let finding = dummy_finding("d16-obsolete-routine-call", &root_cause_key, "ws_routine");
        let fp = idx.fingerprint_of(&finding);

        // Build the expected fingerprint with the dep id substituted.
        let stable_key = format!("d16/{stable_dep_id}");
        let expected_parts =
            format!("d16-obsolete-routine-call|Codeunit/50100|WSProc||{stable_key}");
        let expected = &sha256_hex(&expected_parts)[..16];
        assert_eq!(
            fp, expected,
            "dep id must be substituted with stable id in fingerprint"
        );
    }

    // -----------------------------------------------------------------------
    // Oracle 3 — a SOURCE routine id embedded in rootCauseKey is ALSO replaced
    // with its stable id (the edit-stable fix: ALL routine ids stabilize).
    // -----------------------------------------------------------------------
    #[test]
    fn source_id_in_root_cause_key_is_substituted_with_stable_id() {
        let source_id = "r0/ws:src/caller.al/hash123";
        let source_stable = "app:Codeunit:1#ws_stable_hash";
        let dep_id = "r0/dep:abc/hash456";

        let mut r_dep = minimal_routine(dep_id, "dep/Codeunit/99", "DepProc");
        r_dep.stable_routine_id = "dep_stable_id".to_string();

        let mut r_ws = minimal_routine(source_id, "ws/Codeunit/1", "WsProc");
        r_ws.stable_routine_id = source_stable.to_string();
        let o_ws = minimal_object("ws/Codeunit/1", "Codeunit", 1);
        let o_dep = minimal_object("dep/Codeunit/99", "Codeunit", 99);

        let routines3 = vec![r_dep.clone(), r_ws.clone()];
        let objects3 = vec![o_dep.clone(), o_ws.clone()];
        let idx = FingerprintIndex::build(&routines3, &objects3);

        // rootCauseKey embeds a SOURCE id → now stabilized (edit-survival fix).
        let root_cause_key = format!("d16/{source_id}");
        let finding = dummy_finding("d16-obsolete-routine-call", &root_cause_key, source_id);
        let fp = idx.fingerprint_of(&finding);

        // The source id is replaced with its stable form before hashing.
        let stable_key = format!("d16/{source_stable}");
        let expected_parts = format!("d16-obsolete-routine-call|Codeunit/1|WsProc||{stable_key}");
        let expected = &sha256_hex(&expected_parts)[..16];
        assert_eq!(
            fp, expected,
            "source id must be substituted with stable id (edit-stable fix)"
        );
    }

    // -----------------------------------------------------------------------
    // Oracle 4 — routines with an empty normalized_signature_hash are skipped
    // (no stable form): their internal id stays verbatim in the rootCauseKey.
    // -----------------------------------------------------------------------
    #[test]
    fn routine_with_empty_sig_hash_is_not_substituted() {
        let id = "r0/ws:src/x.al/h";
        let mut r = minimal_routine(id, "ws/Codeunit/1", "WsProc");
        r.normalized_signature_hash = String::new(); // skipped
        r.stable_routine_id = "app:Codeunit:1#".to_string();
        let o = minimal_object("ws/Codeunit/1", "Codeunit", 1);

        let routines = vec![r];
        let objects = vec![o];
        let idx = FingerprintIndex::build(&routines, &objects);

        let root_cause_key = format!("d16/{id}");
        let finding = dummy_finding("d16-obsolete-routine-call", &root_cause_key, id);
        let fp = idx.fingerprint_of(&finding);

        // No stable form → key used verbatim.
        let expected_parts = format!("d16-obsolete-routine-call|Codeunit/1|WsProc||d16/{id}");
        let expected = &sha256_hex(&expected_parts)[..16];
        assert_eq!(
            fp, expected,
            "routine with empty sig hash must not be substituted"
        );
    }
}
