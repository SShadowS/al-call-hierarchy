//! R2b Task 3 — L3 call-graph RESOLUTION vector parity.
//!
//! Loads the committed `tests/r2b-vectors/l3cg-vectors.json` and exercises the
//! 19 RESOLUTION vectors (`resolutionVectors`). For each: parse the inline
//! workspace → assemble_and_resolve → build the symbol table → resolve_calls →
//! project to the vector's expected GROUP shape (callsiteId → sorted CallEdge[]
//! + group dispatchMeta) + the upgraded argumentBindings, and assert equality.
//!
//! The vectors are the oracle: a failure means the Rust resolver diverged from
//! al-sem, and the fix is in the Rust code (never the vector).

use std::collections::HashMap;

use al_call_hierarchy::engine::ids::to_stable_object_id;
use al_call_hierarchy::engine::l3::call_resolver::{
    CallEdge, DeclaredDependency, UpgradedBinding, resolve_calls,
};
use al_call_hierarchy::engine::l3::l3_workspace::{assemble_and_resolve, to_stable_table_id};
use al_call_hierarchy::engine::l3::symbol_table::SymbolTable;

// ---------------------------------------------------------------------------
// Vector document shape (resolution vectors only).
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct VectorsDoc {
    #[serde(rename = "modelInstanceId")]
    model_instance_id: String,
    #[serde(rename = "appGuid")]
    app_guid: String,
    #[serde(rename = "resolutionVectorCount")]
    resolution_vector_count: usize,
    #[serde(rename = "resolutionVectors")]
    resolution_vectors: Vec<ResolutionVector>,
}

#[derive(serde::Deserialize)]
struct ResolutionVector {
    name: String,
    files: Vec<(String, String)>,
    #[serde(rename = "primaryDependencies", default)]
    primary_dependencies: Vec<PrimaryDep>,
    expected: Expected,
}

#[derive(serde::Deserialize)]
struct PrimaryDep {
    #[serde(rename = "appGuid")]
    app_guid: String,
}

#[derive(serde::Deserialize)]
struct Expected {
    groups: Vec<ExpectedGroup>,
    #[serde(default)]
    bindings: Vec<ExpectedBindingGroup>,
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq, Eq)]
struct ExpectedGroup {
    #[serde(rename = "callsiteId")]
    callsite_id: String,
    edges: Vec<ExpectedEdge>,
    #[serde(rename = "dispatchMeta", default)]
    dispatch_meta: Option<ExpectedDispatchMeta>,
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq, Eq)]
struct ExpectedEdge {
    from: String,
    #[serde(default)]
    to: Option<String>,
    #[serde(rename = "operationId")]
    operation_id: String,
    #[serde(rename = "dispatchKind")]
    dispatch_kind: String,
    resolution: String,
    #[serde(default)]
    candidates: Option<Vec<String>>,
    #[serde(rename = "externalTypeRef", default)]
    external_type_ref: Option<ExpectedExternalRef>,
    #[serde(rename = "receiverType", default)]
    receiver_type: Option<String>,
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq, Eq)]
struct ExpectedExternalRef {
    kind: String,
    name: String,
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq, Eq)]
struct ExpectedDispatchMeta {
    #[serde(rename = "interfaceName")]
    interface_name: String,
    #[serde(rename = "totalImpls")]
    total_impls: usize,
    #[serde(rename = "unresolvedImpls")]
    unresolved_impls: Vec<ExpectedUnresolvedImpl>,
    #[serde(rename = "enumImplementers")]
    enum_implementers: Vec<String>,
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq, Eq)]
struct ExpectedUnresolvedImpl {
    #[serde(rename = "objectId")]
    object_id: String,
    reason: String,
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq, Eq)]
struct ExpectedBindingGroup {
    #[serde(rename = "callsiteId")]
    callsite_id: String,
    bindings: Vec<ExpectedBinding>,
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq, Eq)]
struct ExpectedBinding {
    #[serde(rename = "parameterIndex")]
    parameter_index: u32,
    #[serde(rename = "calleeParameterIsVar")]
    callee_parameter_is_var: bool,
    #[serde(rename = "bindingResolution")]
    binding_resolution: String,
}

fn load() -> VectorsDoc {
    let raw = include_str!("../r2b-vectors/l3cg-vectors.json");
    serde_json::from_str(raw).expect("l3cg-vectors.json parses")
}

// ---------------------------------------------------------------------------
// Projection: internal ids → stable ids; group + lift dispatchMeta.
// ---------------------------------------------------------------------------

/// Map an internal routine id → StableRoutineId via the assembled routines.
struct StableMap {
    by_internal: HashMap<String, String>,
}

impl StableMap {
    fn stable_routine(&self, internal: &str) -> String {
        self.by_internal
            .get(internal)
            .cloned()
            .unwrap_or_else(|| internal.to_string())
    }

    /// Convert a callsite/operation id `{internalRoutineId}/csN` (or `/opN`) to
    /// `{stableRoutineId}/csN`. The internal routine id is everything before the
    /// LAST `/`; the suffix (`csN` / `opN`) is preserved verbatim.
    fn stable_site(&self, site_id: &str) -> String {
        match site_id.rsplit_once('/') {
            Some((prefix, suffix)) => format!("{}/{}", self.stable_routine(prefix), suffix),
            None => site_id.to_string(),
        }
    }

    /// An object internal id → StableObjectId (for dispatchMeta unresolvedImpls /
    /// enumImplementers).
    fn stable_object(&self, internal: &str) -> String {
        to_stable_object_id(internal)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjEdge {
    from: String,
    to: Option<String>,
    operation_id: String,
    dispatch_kind: String,
    resolution: String,
    candidates: Option<Vec<String>>,
    external_type_ref: Option<ExpectedExternalRef>,
    receiver_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjGroup {
    callsite_id: String,
    edges: Vec<ProjEdge>,
    dispatch_meta: Option<ExpectedDispatchMeta>,
}

/// Project a resolver `CallEdge` to the stable-id edge shape (no callsiteId — it
/// is the group key; no dispatchMeta — lifted to the group).
fn project_edge(e: &CallEdge, m: &StableMap) -> ProjEdge {
    ProjEdge {
        from: m.stable_routine(&e.from),
        to: e.to.as_deref().map(|t| m.stable_routine(t)),
        operation_id: m.stable_site(&e.operation_id),
        dispatch_kind: e.dispatch_kind.as_str().to_string(),
        resolution: e.resolution.as_str().to_string(),
        // Per Rev2 note 7: project ids to StableRoutineId FIRST, then sort.
        candidates: e.candidates.as_ref().map(|cs| {
            let mut v: Vec<String> = cs.iter().map(|c| m.stable_routine(c)).collect();
            v.sort();
            v
        }),
        external_type_ref: e.external_type_ref.as_ref().map(|x| ExpectedExternalRef {
            kind: x.kind.clone(),
            name: x.name.clone(),
        }),
        receiver_type: e.receiver_type.clone(),
    }
}

/// Group edges by callsiteId, sort each group's edges by `to` (then dispatchKind),
/// and lift the dispatchMeta (which the resolver attached to one member edge) to
/// the group level — projecting its object ids to stable form.
fn project_groups(edges: &[CallEdge], m: &StableMap) -> Vec<ProjGroup> {
    // Preserve first-seen callsite order for stability.
    let mut order: Vec<String> = Vec::new();
    let mut by_site: HashMap<String, Vec<&CallEdge>> = HashMap::new();
    for e in edges {
        let key = m.stable_site(&e.callsite_id);
        if !by_site.contains_key(&key) {
            order.push(key.clone());
        }
        by_site.entry(key).or_default().push(e);
    }

    let mut groups: Vec<ProjGroup> = Vec::new();
    for callsite_id in order {
        let raw = &by_site[&callsite_id];
        // Lift dispatchMeta from whichever edge holds it.
        let dispatch_meta =
            raw.iter()
                .find_map(|e| e.dispatch_meta.as_ref())
                .map(|dm| ExpectedDispatchMeta {
                    interface_name: dm.interface_name.clone(),
                    total_impls: dm.total_impls,
                    unresolved_impls: {
                        let mut v: Vec<ExpectedUnresolvedImpl> = dm
                            .unresolved_impls
                            .iter()
                            .map(|(oid, reason)| ExpectedUnresolvedImpl {
                                object_id: m.stable_object(oid),
                                reason: reason.clone(),
                            })
                            .collect();
                        v.sort_by(|a, b| {
                            a.object_id
                                .cmp(&b.object_id)
                                .then_with(|| a.reason.cmp(&b.reason))
                        });
                        v
                    },
                    enum_implementers: {
                        let mut v: Vec<String> = dm
                            .enum_implementers
                            .iter()
                            .map(|oid| m.stable_object(oid))
                            .collect();
                        v.sort();
                        v
                    },
                });
        let mut edges: Vec<ProjEdge> = raw.iter().map(|e| project_edge(e, m)).collect();
        // Sort a multi-edge group deterministically by (to, dispatchKind).
        edges.sort_by(|a, b| {
            a.to.clone()
                .unwrap_or_default()
                .cmp(&b.to.clone().unwrap_or_default())
                .then_with(|| a.dispatch_kind.cmp(&b.dispatch_kind))
        });
        groups.push(ProjGroup {
            callsite_id,
            edges,
            dispatch_meta,
        });
    }
    // Sort groups by callsiteId for order-robust comparison.
    groups.sort_by(|a, b| a.callsite_id.cmp(&b.callsite_id));
    groups
}

/// Project the expected groups into the same comparable shape.
fn expected_to_proj(groups: &[ExpectedGroup]) -> Vec<ProjGroup> {
    let mut out: Vec<ProjGroup> = groups
        .iter()
        .map(|g| {
            let mut edges: Vec<ProjEdge> = g
                .edges
                .iter()
                .map(|e| ProjEdge {
                    from: e.from.clone(),
                    to: e.to.clone(),
                    operation_id: e.operation_id.clone(),
                    dispatch_kind: e.dispatch_kind.clone(),
                    resolution: e.resolution.clone(),
                    candidates: e.candidates.clone(),
                    external_type_ref: e.external_type_ref.clone(),
                    receiver_type: e.receiver_type.clone(),
                })
                .collect();
            edges.sort_by(|a, b| {
                a.to.clone()
                    .unwrap_or_default()
                    .cmp(&b.to.clone().unwrap_or_default())
                    .then_with(|| a.dispatch_kind.cmp(&b.dispatch_kind))
            });
            ProjGroup {
                callsite_id: g.callsite_id.clone(),
                edges,
                dispatch_meta: g.dispatch_meta.clone(),
            }
        })
        .collect();
    out.sort_by(|a, b| a.callsite_id.cmp(&b.callsite_id));
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjBindingGroup {
    callsite_id: String,
    bindings: Vec<ExpectedBinding>,
}

/// Project the resolver's upgraded bindings to the expected shape — only
/// callsites with ≥1 binding, keyed by stable callsite id, sorted by callsiteId.
fn project_bindings(
    upgraded: &HashMap<String, Vec<UpgradedBinding>>,
    m: &StableMap,
) -> Vec<ProjBindingGroup> {
    let mut out: Vec<ProjBindingGroup> = upgraded
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(internal_id, v)| ProjBindingGroup {
            callsite_id: m.stable_site(internal_id),
            bindings: v
                .iter()
                .map(|b| ExpectedBinding {
                    parameter_index: b.parameter_index,
                    callee_parameter_is_var: b.callee_parameter_is_var,
                    binding_resolution: b.binding_resolution.clone(),
                })
                .collect(),
        })
        .collect();
    out.sort_by(|a, b| a.callsite_id.cmp(&b.callsite_id));
    out
}

fn expected_bindings_to_proj(groups: &[ExpectedBindingGroup]) -> Vec<ProjBindingGroup> {
    let mut out: Vec<ProjBindingGroup> = groups
        .iter()
        .map(|g| ProjBindingGroup {
            callsite_id: g.callsite_id.clone(),
            bindings: g.bindings.clone(),
        })
        .collect();
    out.sort_by(|a, b| a.callsite_id.cmp(&b.callsite_id));
    out
}

// ---------------------------------------------------------------------------
// The parity test.
// ---------------------------------------------------------------------------

#[test]
fn all_resolution_vectors_match() {
    let doc = load();
    assert_eq!(
        doc.resolution_vectors.len(),
        doc.resolution_vector_count,
        "declared resolutionVectorCount must equal the vector array length"
    );

    let mut failures: Vec<String> = Vec::new();

    for v in &doc.resolution_vectors {
        let resolved = assemble_and_resolve(&v.files, &doc.app_guid, &doc.model_instance_id);
        let ws = &resolved.workspace;
        let symbols = SymbolTable::build(&ws.objects, &ws.tables, &ws.routines);

        // internal routine id → stable routine id map.
        let by_internal: HashMap<String, String> = ws
            .routines
            .iter()
            .map(|r| (r.id.clone(), r.stable_routine_id.clone()))
            .collect();
        let smap = StableMap { by_internal };

        // Declared deps (opaque vs external-target). In the source-only inline
        // path the fetched-app set is empty, so any declared dep is "unfetched".
        let deps: Vec<DeclaredDependency> = v
            .primary_dependencies
            .iter()
            .map(|d| DeclaredDependency {
                app_guid: d.app_guid.clone(),
            })
            .collect();
        let fetched: Vec<String> = Vec::new();

        let result = resolve_calls(ws, &symbols, &deps, &fetched);

        let actual_groups = project_groups(&result.edges, &smap);
        let expected_groups = expected_to_proj(&v.expected.groups);
        if actual_groups != expected_groups {
            failures.push(format!(
                "[{}] GROUPS mismatch\n  expected: {:#?}\n  actual:   {:#?}",
                v.name, expected_groups, actual_groups
            ));
        }

        let actual_bindings = project_bindings(&result.upgraded_bindings, &smap);
        let expected_bindings = expected_bindings_to_proj(&v.expected.bindings);
        if actual_bindings != expected_bindings {
            failures.push(format!(
                "[{}] BINDINGS mismatch\n  expected: {:#?}\n  actual:   {:#?}",
                v.name, expected_bindings, actual_bindings
            ));
        }

        // No double-upgrade diagnostic should ever fire in a single resolve pass.
        if !result.diagnostics.is_empty() {
            failures.push(format!(
                "[{}] unexpected diagnostics: {:?}",
                v.name, result.diagnostics
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "L3 resolution-vector parity failures ({}):\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

/// Stable table id helper is exercised indirectly; keep a reference so the import
/// is never flagged unused if a future projection path drops it.
#[allow(dead_code)]
fn _table_id_ref(internal: &str) -> String {
    to_stable_table_id(internal)
}
