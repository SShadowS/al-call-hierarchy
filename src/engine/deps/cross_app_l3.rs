//! R2.5b CROSS-APP L3 wiring — feed the R2.5a merged index (workspace native
//! entities + projected `.app`-dep objects/tables/routines) into the SAME already
//! ported L3 pipeline (`l3_workspace::resolve` → call/event/coverage projections),
//! so cross-app callsites / record-vars / subscribers RESOLVE.
//!
//! ## What this is (and is NOT)
//!
//! - It is WIRING + the merged input. There is NO new L3 ALGORITHM here: the dep
//!   entities are converted to the EXACT `L3Object`/`L3Table`/`L3Routine` shape the
//!   native source path produces, APPENDED to the workspace AFTER the native
//!   entities (mirroring al-sem's `withDependencyArtifacts`, which `push`es dep
//!   entities last — `src/deps/dependency-artifact.ts:213-215`), and the standard
//!   `l3_workspace::resolve` runs over the merged whole.
//! - The append order is LOAD-BEARING: the symbol table is LAST-wins and the
//!   extension-field merge is FIRST-wins, both keyed off assembled order — so
//!   workspace-first / dep-last reproduces al-sem's collision/shadowing semantics.
//!
//! ## L4 / cone / summary LEAKAGE BOUNDARY (Rev 2 #5)
//!
//! The input to L3 is an **L3-only** merged index. The dep side comes from
//! `project_abi_to_index` (`ProjectedObject`/`ProjectedTable`/`ProjectedRoutine`),
//! whose Rust types STRUCTURALLY DO NOT CARRY any L4 field — there is no `summary`,
//! `intraAppCallEdges`, `citedOperationEvidence`, `depOrderIndex`, capability-cone,
//! or typed-edge field anywhere on these structs. So a poisoned merged model
//! "carrying" such fields cannot influence L3: there is nowhere for them to live.
//! The poison NEGATIVE test (`cross_app_l3_poison.rs`) proves this by constructing
//! the merged L3 with bogus extra-field-bearing inputs and asserting the projection
//! is byte-identical. DO NOT add an L4 field to the L3 entity structs — the boundary
//! is enforced by the type, not by a runtime strip.
//!
//! ## Capture-point mutation audit (Rev 2 #3) — dep-entity in-place mutations
//!
//! `resolveModel` mutates these on DEP-origin entities; `l3_workspace::resolve`
//! reproduces each, and the projections read POST-resolve values:
//!   1. **Extension-field merge** (`merge_extension_fields`): a dep `TableExtension`'s
//!      fields are merged INTO its base table (here: dep `Dep Vendor` gains the dep
//!      ext's `Rating`; a WORKSPACE `TableExtension` on a dep table merges its field
//!      onto the dep base table — both directions cross the app boundary). This is
//!      the ONLY mutation that touches a dep-ORIGIN identity field.
//!   2. **record-var `tableId` backfill** (`resolve_record_types`) + **`argumentBindings`
//!      upgrade** (`upgrade_bindings`): under `noDepSummaries:true` the dep routines
//!      carry EMPTY features (no record vars / call sites), so these mutate only the
//!      WORKSPACE caller's record vars / callsite bindings — never the dep routine.
//!      Confirmed against al-sem (dep routines: recordVariables=[], callSites=[]).
//!
//! So feeding the dep entities + running the unchanged `resolve` is sufficient — the
//! same three resolve sub-steps mutate the same fields al-sem mutates.

use std::path::Path;

use crate::engine::deps::merged_index::collect_app_paths;
use crate::engine::deps::projection::{ProjectedObject, ProjectedRoutine, ProjectedTable};
use crate::engine::l3::l3_workspace::{
    assemble_l3_workspace_from_disk, resolve, L3Field, L3Object, L3Parameter, L3Resolved,
    L3Routine, L3Table, L3Workspace,
};

/// The merged-input context: the assembled+resolved cross-app workspace plus the
/// dep-app ledger the call-graph / coverage projections need (declared deps,
/// fetched app guids, and per-app sourceKind for `opaqueApps`).
pub struct CrossAppL3 {
    pub resolved: L3Resolved,
    /// Declared dependency app guids (from the workspace app.json) — drives the
    /// member-call opaque-vs-external-target split (`has_unfetched_declared_dependency`).
    pub declared_dep_app_guids: Vec<String>,
    /// Dep app guids actually FETCHED (a readable `.app` produced entities).
    pub fetched_app_guids: Vec<String>,
    /// `(appGuid, sourceKind)` for every app — workspace ("source") + each dep
    /// ("symbol-only" | "app-source"). Drives coverage `opaqueApps`.
    pub apps: Vec<(String, String)>,
}

/// Convert one projected dep object into the L3 object shape (identical to the
/// native source path's `L3Object`). The dep object carries the same identity
/// (StableObjectId-independent internal id) the native path mints.
fn dep_object_to_l3(o: &ProjectedObject) -> L3Object {
    L3Object {
        id: o.id.clone(),
        app_guid: o.app_guid.clone(),
        object_type: o.object_type.clone(),
        object_number: o.object_number,
        name: o.name.clone(),
        source_table_name: o.source_table_name.clone(),
        extends_target_name: o.extends_target_name.clone(),
        implements_interfaces: o.implements_interfaces.clone(),
    }
}

/// Convert one projected dep field into the L3 field shape.
fn dep_field_to_l3(f: &crate::engine::deps::projection::ProjectedField) -> L3Field {
    L3Field {
        id: f.id.clone(),
        physical_table_id: f.physical_table_id.clone(),
        declaring_object_id: f.declaring_object_id.clone(),
        declaring_app_id: f.declaring_app_id.clone(),
        field_number: f.field_number,
        name: f.name.clone(),
        field_class: f.field_class.clone(),
        data_type: f.data_type.clone(),
        is_blob_like: f.is_blob_like,
    }
}

/// Convert one projected dep table into the L3 table shape.
fn dep_table_to_l3(t: &ProjectedTable) -> L3Table {
    L3Table {
        id: t.id.clone(),
        app_guid: t.app_guid.clone(),
        table_number: t.table_number,
        name: t.name.clone(),
        fields: t.fields.iter().map(dep_field_to_l3).collect(),
    }
}

/// Convert one projected dep routine into the L3 routine shape. Dep routines carry
/// EMPTY features (no record vars / ops / variables / call sites) — matching
/// al-sem's dep routines under `noDepSummaries:true`. The `parameters` (for arity +
/// the event-graph publisher param shape), `attributes_parsed` (the event-graph
/// inputs), `kind`, and identity fields ARE carried.
fn dep_routine_to_l3(r: &ProjectedRoutine, object_type: &str) -> L3Routine {
    // objectId = `${appGuid}/${objectType}/${objectNumber}` — recover the parts.
    let parts: Vec<&str> = r.object_id.split('/').collect();
    let app_guid = parts.first().copied().unwrap_or("").to_string();
    let object_number = parts
        .get(2)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);

    let parameters = r
        .parameters
        .iter()
        .map(|p| L3Parameter {
            index: p.index as u32,
            name: p.name.clone(),
            type_text: p.type_text.clone(),
            is_var: p.is_var,
            is_record: p.is_record,
            // PARITY (R2.5b-c): al-sem's dep-routine parameters come from the ABI
            // symbol-reference projection (`dependency-projection.ts`), which does NOT
            // populate `tableName` on a record parameter (only the NATIVE grammar path
            // sets it). The event-graph publisher param shape (R2.5b-c) is the first
            // golden consumer of a dep routine's param `tableName`, and al-sem omits it
            // there — so a dep record param's `table_name` MUST be `None` to byte-match.
            // (Deriving it from `type_text` here over-populated vs al-sem; corrected.)
            table_name: None,
        })
        .collect();

    L3Routine {
        id: r.id.clone(),
        stable_routine_id: r.stable_routine_id.clone(),
        object_id: r.object_id.clone(),
        object_type: object_type.to_string(),
        name: r.name.clone(),
        kind: r.kind.clone(),
        attributes_parsed: r.attributes_parsed.clone(),
        app_guid,
        object_number,
        normalized_signature_hash: r.signature_fingerprint.clone(),
        body_available: r.body_available, // false for dep routines.
        parse_incomplete: false,
        record_variables: Vec::new(),
        record_operations: Vec::new(),
        variables: Vec::new(),
        parameters,
        return_type: r.return_type.clone(),
        call_sites: Vec::new(),
    }
}

/// Append the dep entities (objects/tables/routines) onto an already-assembled
/// native workspace, in al-sem's `withDependencyArtifacts` order: workspace FIRST
/// (already present), deps LAST. The routine→object_type lookup is built from the
/// dep objects so each dep routine carries its owning object's type.
fn append_dep_entities(
    workspace: &mut L3Workspace,
    objects: &[ProjectedObject],
    tables: &[ProjectedTable],
    routines: &[ProjectedRoutine],
) {
    use std::collections::HashMap;
    let object_type_by_id: HashMap<&str, &str> = objects
        .iter()
        .map(|o| (o.id.as_str(), o.object_type.as_str()))
        .collect();

    for o in objects {
        workspace.objects.push(dep_object_to_l3(o));
    }
    for t in tables {
        workspace.tables.push(dep_table_to_l3(t));
    }
    for r in routines {
        let object_type = object_type_by_id
            .get(r.object_id.as_str())
            .copied()
            .unwrap_or("");
        workspace.routines.push(dep_routine_to_l3(r, object_type));
    }
}

/// Build the cross-app L3 from a disk workspace (native `.al` source) + its dep
/// `.app`(s). `declared_dep_app_guids` is the workspace app.json `dependencies[]`
/// app-guid list (some may be UNFETCHED — declared but no `.app` present → the
/// opaque-vs-external-target split). `alpackages_path` is where the dep `.app`(s)
/// live (typically `<workspace>/.alpackages`).
///
/// Fail-closed: an unsound/empty native layout yields `None` (mirrors
/// `assemble_and_resolve_workspace`); a bad `.app` contributes nothing. Never panics.
pub fn build_cross_app_l3(
    workspace: &Path,
    alpackages_path: &Path,
    declared_dep_app_guids: &[String],
    model_instance_id: &str,
) -> Option<CrossAppL3> {
    // 1. Assemble the NATIVE workspace L3 model from `.al` source (pre-resolve).
    let mut ws = assemble_l3_workspace_from_disk(workspace, model_instance_id)?;

    // 2. Read + project the dep `.app`(s). Collect (appGuid, sourceKind) for the
    //    apps ledger; track which dep app guids were actually fetched.
    let mut dep_objects: Vec<ProjectedObject> = Vec::new();
    let mut dep_tables: Vec<ProjectedTable> = Vec::new();
    let mut dep_routines: Vec<ProjectedRoutine> = Vec::new();
    let mut apps: Vec<(String, String)> = Vec::new();
    let mut fetched_app_guids: Vec<String> = Vec::new();

    // The workspace app(s) are "source". Derive from the native objects' app guid.
    let mut ws_app_guids: Vec<String> = ws
        .objects
        .iter()
        .map(|o| o.app_guid.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    ws_app_guids.sort();
    for g in &ws_app_guids {
        apps.push((g.clone(), "source".to_string()));
    }

    for app_path in collect_app_paths(alpackages_path) {
        let Ok(bytes) = std::fs::read(&app_path) else {
            continue;
        };
        let Some(parsed) =
            crate::engine::deps::merged_index::parse_dep_app_public(&bytes, model_instance_id)
        else {
            continue;
        };
        fetched_app_guids.push(parsed.app_guid.clone());
        apps.push((
            parsed.app_guid.clone(),
            if parsed.includes_source {
                "app-source".to_string()
            } else {
                "symbol-only".to_string()
            },
        ));
        dep_objects.extend(parsed.objects);
        dep_tables.extend(parsed.tables);
        dep_routines.extend(parsed.routines);
    }

    // 3. MERGE: append dep entities (workspace first, deps last).
    append_dep_entities(&mut ws, &dep_objects, &dep_tables, &dep_routines);

    // 4. RESOLVE the merged whole (build_symbol_table → resolve_record_types →
    //    merge_extension_fields). Same `resolve` the native path runs — no new algo.
    resolve(&mut ws);

    Some(CrossAppL3 {
        resolved: L3Resolved { workspace: ws },
        declared_dep_app_guids: declared_dep_app_guids.to_vec(),
        fetched_app_guids,
        apps,
    })
}

/// Read the workspace root `app.json` `dependencies[].id` app guids (the DECLARED
/// deps, some of which may be UNfetched). Returns `[]` on any read/parse miss
/// (fail-closed). Mirrors al-sem `parseWorkspaceDependencies` (the app-guid subset).
pub fn read_workspace_declared_dep_app_guids(workspace: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(workspace.join("app.json")) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let Some(deps) = value.get("dependencies").and_then(|d| d.as_array()) else {
        return Vec::new();
    };
    deps.iter()
        .filter_map(|d| {
            // al-sem accepts `id` (modern) or `appId` (legacy) — match its parser.
            d.get("id")
                .or_else(|| d.get("appId"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        })
        .collect()
}

/// Convenience: build the cross-app L3 over a disk workspace that has its dep
/// `.app`(s) under `<workspace>/.alpackages`, reading the declared deps from the
/// workspace `app.json`. The single entry point the `aldump` cross-app mode drives.
pub fn build_cross_app_l3_from_workspace(
    workspace: &Path,
    model_instance_id: &str,
) -> Option<CrossAppL3> {
    let declared = read_workspace_declared_dep_app_guids(workspace);
    let alpackages = workspace.join(".alpackages");
    build_cross_app_l3(workspace, &alpackages, &declared, model_instance_id)
}

impl CrossAppL3 {
    /// Cross-app L3 record-type projection (R2.5b-a) — record vars now bind to dep
    /// StableTableIds; dep / ws TableExtension fields merged onto the dep base table.
    /// Identical to the source-only `project()` (record-types need no dep ledger).
    pub fn project_record_types(&self) -> crate::engine::l3::l3_workspace::L3RecordTypeProjection {
        self.resolved.project()
    }

    /// Cross-app L3 call-graph projection (R2.5b-b) — cross-app member calls resolve
    /// to dep StableRoutineIds; the dep ledger drives the opaque/external-target split.
    pub fn project_call_graph(
        &self,
    ) -> crate::engine::l3::call_graph_projection::L3CallGraphProjection {
        self.resolved
            .project_call_graph_cross_app(&self.declared_dep_app_guids, &self.fetched_app_guids)
    }

    /// Cross-app L3 event-graph projection (R2.5b-c) — ws subscriber → dep publisher,
    /// dep subscriber → ws publisher. Identical to the source-only `project_event_graph`
    /// (the event graph reads dep `attributes_parsed`, no dep ledger needed).
    pub fn project_event_graph(&self) -> crate::engine::l3::event_graph::L3EventGraphProjection {
        self.resolved.project_event_graph()
    }

    /// Cross-app L3 coverage projection (R2.5b-d). `opaqueApps` lists the symbol-only
    /// dep app guids (R3a-0 Fix 2, al-sem `81d538a`+`f1650ba`): `buildCoverage` reads
    /// `index.identity.apps.filter(sourceKind == "symbol-only")`, and
    /// `withDependencyArtifacts` now stamps the dep `AppIdentity`s (with `sourceKind`)
    /// into `identity.apps`, so the symbol-only deps are present. The observable
    /// cross-app coverage signal also includes the `unresolvedCallsites` /
    /// `dynamicDispatchSites` multiset delta (cross-app member calls that RESOLVED drop
    /// OUT; the external-target miss stays IN), and `routinesTotal` counts dep routines.
    ///
    /// The call resolution INSIDE this projection threads the REAL declared/fetched ledger
    /// (Fix 1: al-sem reads `identity.primaryDependencies` DURING resolve, in production AND
    /// the capture harness as of `93e360d`). On the all-fetched corpus the `gone.M()` member
    /// miss is `external-target` (genuinely — all declared deps fetched) and stays IN
    /// `unresolvedCallsites`; the unfetched-declared-dep member-`opaque` branch is proven by
    /// `tests/r3a0_unfetched_dep_opaque.rs`.
    pub fn project_coverage(
        &self,
        units: &[crate::engine::l3::coverage::CoverageUnit],
        index_diagnostics: &[crate::engine::l3::coverage::CoverageDiagnostic],
    ) -> crate::engine::l3::coverage::AnalysisCoverage {
        self.resolved.project_coverage_cross_app(
            units,
            index_diagnostics,
            &self.apps,
            &self.declared_dep_app_guids,
            &self.fetched_app_guids,
        )
    }

    /// Disk-backed cross-app coverage capture: re-discover the workspace `.al` files
    /// as source units (the dep `.app`s are NOT source units), then build coverage.
    pub fn project_coverage_disk(
        &self,
        workspace: &Path,
    ) -> crate::engine::l3::coverage::AnalysisCoverage {
        let units = crate::engine::l3::coverage::coverage_source_units_for_workspace(workspace);
        let diagnostics: Vec<crate::engine::l3::coverage::CoverageDiagnostic> = Vec::new();
        self.project_coverage(&units, &diagnostics)
    }
}
