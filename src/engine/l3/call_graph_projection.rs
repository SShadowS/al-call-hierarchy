//! L3 CALL-GRAPH projection (R2b Task 4) — the golden-shaped, stable-id projection
//! of the resolved call graph + upgraded argument bindings.
//!
//! This is the Rust mirror of al-sem's `scripts/r2b-l3cg-projection.ts`
//! (`projectL3CallGraph`). It READS the resolved call graph (`resolve_calls`
//! output) — it NEVER re-resolves — and projects it to the EXACT shape the
//! `*.l3cg.golden.json` files carry:
//!
//!   - Per callsite GROUP (`callsiteId → CallEdge[]`, NEVER collapsed): interface
//!     dispatch is MULTI-edge. Per edge: `from` / `to?` / `operationId` /
//!     `dispatchKind` / `resolution` / `candidates?` / `externalTypeRef?` /
//!     `receiverType?` — all ids in STABLE form.
//!   - `dispatchMeta` lifted to the GROUP level (one per interface callsite); the
//!     resolver attaches it to one edge only (an internal-RoutineId sort artifact),
//!     so group-level projection makes multi-edge comparison order-robust. NO
//!     `openWorld` field (it does not exist in shipped source).
//!   - The UPGRADED `argumentBindings` per callsite (ALL bindings whose callsite has
//!     ≥1 binding, including `non-record-arg`), keyed by stable callsite id.
//!
//! === Sort semantics (plan Rev 2 MUST-FIX #7) ===
//! Convert ids to STABLE form FIRST, then sort with ONE explicit byte-order string
//! comparator (`cmp_stable`) used IDENTICALLY to al-sem `cmpStable`:
//!   - edges within a group → by `edge_sort_key`
//!   - candidates           → by StableRoutineId
//!   - unresolvedImpls      → by (stable objectId, reason)
//!   - enumImplementers     → by stable objectId
//!   - groups               → by stable callsiteId
//!   - bindings             → by stable callsiteId
//!
//! FORBIDDEN later-gate / L4 fields are structurally absent from the serde types
//! (built key-by-key), so they can never leak in.

use std::collections::HashMap;

use super::call_resolver::{
    resolve_calls, CallEdge, DeclaredDependency, ResolvedCalls, UpgradedBinding,
};
use super::l3_workspace::L3Resolved;
use super::symbol_table::SymbolTable;
use crate::engine::ids::to_stable_object_id;

// ---------------------------------------------------------------------------
// Serde projection shape — matches `*.l3cg.golden.json` exactly. Field ORDER is
// chosen to match al-sem's `projectEdge` key order so the ws-d2 smoke is
// byte-identical: from, operationId, dispatchKind, resolution, then the optionals
// to / candidates / externalTypeRef / receiverType (skipped when absent).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PExternalTypeRef {
    pub kind: String,
    pub name: String,
}

/// One projected CallEdge (stable id form). `dispatchMeta` is NOT here — it is at
/// the group level.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PCallEdge {
    pub from: String,
    #[serde(rename = "operationId")]
    pub operation_id: String,
    #[serde(rename = "dispatchKind")]
    pub dispatch_kind: String,
    pub resolution: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidates: Option<Vec<String>>,
    #[serde(rename = "externalTypeRef", skip_serializing_if = "Option::is_none")]
    pub external_type_ref: Option<PExternalTypeRef>,
    #[serde(rename = "receiverType", skip_serializing_if = "Option::is_none")]
    pub receiver_type: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PUnresolvedImpl {
    #[serde(rename = "objectId")]
    pub object_id: String,
    pub reason: String,
}

/// dispatchMeta projected at the GROUP level (one per interface callsite).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PDispatchMeta {
    #[serde(rename = "interfaceName")]
    pub interface_name: String,
    #[serde(rename = "totalImpls")]
    pub total_impls: usize,
    #[serde(rename = "unresolvedImpls")]
    pub unresolved_impls: Vec<PUnresolvedImpl>,
    #[serde(rename = "enumImplementers")]
    pub enum_implementers: Vec<String>,
}

/// A callsite group: all edges with the same (stable) callsiteId.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PCallsiteGroup {
    #[serde(rename = "callsiteId")]
    pub callsite_id: String,
    pub edges: Vec<PCallEdge>,
    #[serde(rename = "dispatchMeta", skip_serializing_if = "Option::is_none")]
    pub dispatch_meta: Option<PDispatchMeta>,
}

/// Per-callsite upgraded argumentBinding (the L3-resolved fields only).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PArgumentBinding {
    #[serde(rename = "parameterIndex")]
    pub parameter_index: u32,
    #[serde(rename = "calleeParameterIsVar")]
    pub callee_parameter_is_var: bool,
    #[serde(rename = "bindingResolution")]
    pub binding_resolution: String,
}

/// Per-callsite argumentBindings record, keyed by stable callsiteId.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PCallsiteBindings {
    #[serde(rename = "callsiteId")]
    pub callsite_id: String,
    pub bindings: Vec<PArgumentBinding>,
}

/// The full L3 call-graph projection — the golden document shape.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct L3CallGraphProjection {
    pub groups: Vec<PCallsiteGroup>,
    pub bindings: Vec<PCallsiteBindings>,
}

// ---------------------------------------------------------------------------
// The ONE byte-order comparator (al-sem `cmpStable`), used identically at every
// sort point on the projected stable strings.
// ---------------------------------------------------------------------------

/// Byte-order string compare. (Rust `str::cmp` is byte-lexicographic on UTF-8,
/// which matches TS `<`/`>` on the BMP-only stable id alphabet used here.)
pub fn cmp_stable(a: &str, b: &str) -> std::cmp::Ordering {
    a.cmp(b)
}

/// Edge sort key WITHIN a callsite group — al-sem `edgeSortKey`. Edges share a
/// callsiteId; order them by (resolution, to?, dispatchKind, candidates-joined,
/// externalTypeRef, receiverType) so the multiset has a canonical order independent
/// of source emission order. Joined with a single space (a byte that cannot appear
/// in a stable id) to avoid delimiter collisions.
fn edge_sort_key(e: &PCallEdge) -> String {
    let candidates = e
        .candidates
        .as_ref()
        .map(|c| c.join(","))
        .unwrap_or_default();
    let ext = e
        .external_type_ref
        .as_ref()
        .map(|x| format!("{} {}", x.kind, x.name))
        .unwrap_or_default();
    [
        e.resolution.clone(),
        e.to.clone().unwrap_or_default(),
        e.dispatch_kind.clone(),
        candidates,
        ext,
        e.receiver_type.clone().unwrap_or_default(),
    ]
    .join(" ")
}

// ---------------------------------------------------------------------------
// Stable-id mapping. Internal RoutineId is `${modelInstanceId}/${hash}` and
// carries NO object identity; map it through the routine to its StableRoutineId.
// callsiteId / operationId embed the internal RoutineId prefix; rewrite that.
// ---------------------------------------------------------------------------

struct StableMap {
    by_internal: HashMap<String, String>,
}

impl StableMap {
    /// Internal routine id → StableRoutineId, or the raw id when absent (a `to`
    /// that doesn't map should never happen for a resolved edge; keep the raw id
    /// so a divergence is VISIBLE rather than silently dropped).
    fn stable_routine(&self, internal: &str) -> String {
        self.by_internal
            .get(internal)
            .cloned()
            .unwrap_or_else(|| internal.to_string())
    }

    /// Rewrite `${routineId}/<suffix>` (callsiteId `/csN`, operationId `/opN`) to
    /// stable form. The internal RoutineId is exactly two `/`-parts, so the suffix
    /// is everything after the SECOND `/` — i.e. after the LAST `/` here.
    fn stable_site(&self, site_id: &str) -> String {
        match site_id.rsplit_once('/') {
            Some((prefix, suffix)) => format!("{}/{}", self.stable_routine(prefix), suffix),
            None => site_id.to_string(),
        }
    }
}

fn project_edge(e: &CallEdge, m: &StableMap) -> PCallEdge {
    PCallEdge {
        from: m.stable_routine(&e.from),
        operation_id: m.stable_site(&e.operation_id),
        dispatch_kind: e.dispatch_kind.as_str().to_string(),
        resolution: e.resolution.as_str().to_string(),
        to: e.to.as_deref().map(|t| m.stable_routine(t)),
        // Per Rev2 #7: project ids to StableRoutineId FIRST, then sort. al-sem only
        // emits `candidates` when non-empty.
        candidates: e.candidates.as_ref().and_then(|cs| {
            if cs.is_empty() {
                None
            } else {
                let mut v: Vec<String> = cs.iter().map(|c| m.stable_routine(c)).collect();
                v.sort_by(|a, b| cmp_stable(a, b));
                Some(v)
            }
        }),
        external_type_ref: e.external_type_ref.as_ref().map(|x| PExternalTypeRef {
            kind: x.kind.clone(),
            name: x.name.clone(),
        }),
        receiver_type: e.receiver_type.clone(),
    }
}

fn project_dispatch_meta(dm: &super::call_resolver::DispatchMeta, m: &StableMap) -> PDispatchMeta {
    let mut unresolved_impls: Vec<PUnresolvedImpl> = dm
        .unresolved_impls
        .iter()
        .map(|(oid, reason)| PUnresolvedImpl {
            object_id: to_stable_object_id(oid),
            reason: reason.clone(),
        })
        .collect();
    unresolved_impls.sort_by(|a, b| {
        cmp_stable(&a.object_id, &b.object_id).then_with(|| cmp_stable(&a.reason, &b.reason))
    });
    let mut enum_implementers: Vec<String> = dm
        .enum_implementers
        .iter()
        .map(|oid| to_stable_object_id(oid))
        .collect();
    enum_implementers.sort_by(|a, b| cmp_stable(a, b));
    let _ = m; // object ids are stable-projected directly; map kept for symmetry.
    PDispatchMeta {
        interface_name: dm.interface_name.clone(),
        total_impls: dm.total_impls,
        unresolved_impls,
        enum_implementers,
    }
}

// ---------------------------------------------------------------------------
// Top-level projection.
// ---------------------------------------------------------------------------

/// Project a resolved call graph (`resolve_calls` output) + the workspace routines
/// (for the internal→stable id map) to the golden L3 call-graph shape.
fn project_call_graph_inner(
    resolved: &ResolvedCalls,
    routines_stable_map: &StableMap,
) -> L3CallGraphProjection {
    let m = routines_stable_map;

    // --- Group call edges by STABLE callsiteId. ---
    let mut order: Vec<String> = Vec::new();
    let mut by_site: HashMap<String, Vec<&CallEdge>> = HashMap::new();
    for e in &resolved.edges {
        let key = m.stable_site(&e.callsite_id);
        if !by_site.contains_key(&key) {
            order.push(key.clone());
        }
        by_site.entry(key).or_default().push(e);
    }

    let mut groups: Vec<PCallsiteGroup> = Vec::new();
    for callsite_id in order {
        let raw = &by_site[&callsite_id];
        // Lift dispatchMeta from whichever edge holds it (source attaches it to one).
        let dispatch_meta = raw
            .iter()
            .find_map(|e| e.dispatch_meta.as_ref())
            .map(|dm| project_dispatch_meta(dm, m));
        let mut edges: Vec<PCallEdge> = raw.iter().map(|e| project_edge(e, m)).collect();
        edges.sort_by(|a, b| cmp_stable(&edge_sort_key(a), &edge_sort_key(b)));
        groups.push(PCallsiteGroup {
            callsite_id,
            edges,
            dispatch_meta,
        });
    }
    groups.sort_by(|a, b| cmp_stable(&a.callsite_id, &b.callsite_id));

    // --- Upgraded argumentBindings, keyed by stable callsiteId. ALL callsites with
    //     ≥1 binding (al-sem emits `non-record-arg` bindings too). ---
    let mut bindings: Vec<PCallsiteBindings> = resolved
        .upgraded_bindings
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(internal_id, v)| PCallsiteBindings {
            callsite_id: m.stable_site(internal_id),
            bindings: v.iter().map(project_binding).collect(),
        })
        .collect();
    bindings.sort_by(|a, b| cmp_stable(&a.callsite_id, &b.callsite_id));

    L3CallGraphProjection { groups, bindings }
}

fn project_binding(b: &UpgradedBinding) -> PArgumentBinding {
    PArgumentBinding {
        parameter_index: b.parameter_index,
        callee_parameter_is_var: b.callee_parameter_is_var,
        binding_resolution: b.binding_resolution.clone(),
    }
}

impl L3Resolved {
    /// Project the resolved workspace's CALL GRAPH to the golden L3 call-graph shape.
    ///
    /// This builds the symbol table and runs `resolve_calls` ONCE (the resolved
    /// model is captured fresh here — al-sem's `resolveModel` likewise runs it once;
    /// `upgrade_bindings` is non-idempotent but each callsite's binding state is
    /// constructed fresh per call, so a single `project_call_graph` is the
    /// "post-resolve, read-once" capture). SOURCE-ONLY: empty declared deps + empty
    /// fetched set → `has_unfetched_declared_dependency` is false (member-opaque is
    /// structurally unreachable in this bare path; object-run misses are opaque).
    pub fn project_call_graph(&self) -> L3CallGraphProjection {
        let ws = &self.workspace;
        let symbols = SymbolTable::build(&ws.objects, &ws.tables, &ws.routines);
        let no_deps: Vec<DeclaredDependency> = Vec::new();
        let no_fetched: Vec<String> = Vec::new();
        let resolved = resolve_calls(ws, &symbols, &no_deps, &no_fetched);

        let by_internal: HashMap<String, String> = ws
            .routines
            .iter()
            .map(|r| (r.id.clone(), r.stable_routine_id.clone()))
            .collect();
        let smap = StableMap { by_internal };

        project_call_graph_inner(&resolved, &smap)
    }

    /// CROSS-APP (R2.5b) call-graph projection: the merged workspace+dep model.
    ///
    /// The MEMBER-call opaque-vs-external-target split gates on
    /// `has_unfetched_declared_dependency`, which reads `index.identity.primaryDependencies`.
    /// As of the R3a-0 semantic-oracle epoch (al-sem `81d538a` + capture fix `93e360d`),
    /// PRODUCTION `analyzeWorkspace` AND the R2.5b capture harness both stamp
    /// `identity.primaryDependencies` onto the merged index *BEFORE* `resolveModel`, so the
    /// resolver reads the REAL declared deps DURING resolution (Fix 1): a member miss into an
    /// UNFETCHED declared dep classifies `opaque` (the member might live there), a fetched-dep
    /// member miss stays `member-not-found`, and a non-declared-dep object miss stays
    /// `external-target`. We thread the REAL declared/fetched ledger here to mirror that.
    ///
    /// On the ALL-FETCHED R2.5b corpus (declared = fetched = {Lib Core, Lib Ext}; the prior
    /// `Lib Absent` unfetched dep was removed in al-sem `93e360d`) this is BYTE-INVARIANT:
    /// `has_unfetched_declared_dependency` is false, so the `gone.M()` member miss into an
    /// absent object stays `external-target` GENUINELY (preserving the cg matrix's
    /// external-target axis), while the `Codeunit.Run("Absent Dep Cu")` object-run miss is
    /// `opaque` (ledger-independent, `call-resolver.ts:596`). The unfetched-declared-dep
    /// member-`opaque` branch is proven out-of-corpus by `tests/r3a0_unfetched_dep_opaque.rs`.
    pub fn project_call_graph_cross_app(
        &self,
        declared_dep_app_guids: &[String],
        fetched_app_guids: &[String],
    ) -> L3CallGraphProjection {
        let ws = &self.workspace;
        let symbols = SymbolTable::build(&ws.objects, &ws.tables, &ws.routines);
        // Thread the REAL declared/fetched ledger — mirrors fixed production al-sem (reads
        // primaryDependencies DURING resolve). Byte-invariant on the all-fetched corpus.
        let declared: Vec<DeclaredDependency> = declared_dep_app_guids
            .iter()
            .map(|app_guid| DeclaredDependency {
                app_guid: app_guid.clone(),
            })
            .collect();
        let resolved = resolve_calls(ws, &symbols, &declared, fetched_app_guids);

        let by_internal: HashMap<String, String> = ws
            .routines
            .iter()
            .map(|r| (r.id.clone(), r.stable_routine_id.clone()))
            .collect();
        let smap = StableMap { by_internal };

        project_call_graph_inner(&resolved, &smap)
    }
}
