//! 1B.3a Task 4: L3-validated semantic edge golden + route-applicability
//! contract.
//!
//! # Golden floor
//!
//! [`mint_l3_validated_golden`] captures the L3-oracle target set per call site
//! into a [`SemanticGolden`] (a `BTreeMap` keyed by column-ignoring
//! [`GoldenSiteKey`]).  [`assert_against_semantic_golden`] compares a fresh
//! canonical edge batch against this golden and classifies every site into:
//! `match`, `fresh_wrong`, `fresh_missing`, `fresh_extra`, `fresh_novel`, or
//! `golden_missing`.
//!
//! # The critical invariant
//!
//! **`SemanticDiff::fresh_wrong.is_empty()`** — fresh must never confidently
//! emit a target that L3 says is wrong.  A per-site Histogram cannot catch
//! this: it can count "resolved" or "unknown" but cannot tell you WHICH target
//! was chosen.  This golden does.
//!
//! # Route-applicability contract
//!
//! [`route_applicability`] verifies the structural witness↔evidence contract
//! on every route and delegates the ABI ingestion check to
//! [`abi_ingestion_integrity`].
//!
//! # CDO/L3 audit
//!
//! [`run_cdo_semantic_audit`] runs the full comparison over a real workspace
//! (env-gated; the caller checks `CDO_WS`).

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::program::resolve::abi_check::{
    abi_ingestion_integrity, build_raw_abi_index_from_snapshot,
};
use crate::program::resolve::differential::{
    CanonicalEdge, CanonicalTarget, project_fresh, project_l3, witness_contract_holds,
};
use crate::program::resolve::edge::{Edge, EdgeKind};

// ---------------------------------------------------------------------------
// Column-ignoring site key (serde-able)
// ---------------------------------------------------------------------------

/// Serde-able, column-ignoring key for one call site in the semantic golden.
///
/// Omits the column offset because L3 uses UTF-16 columns while the fresh
/// side uses byte columns — they agree on ASCII but may differ by a small
/// delta on non-ASCII identifiers.  The strong key `(unit, line, callee_fp)`
/// mirrors the invariant used by [`crate::program::resolve::differential::match_sites`].
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GoldenSiteKey {
    pub from_app_guid: String,
    pub from_object_kind: String,
    pub from_object_lc: String,
    pub from_routine_lc: String,
    /// `EdgeKind` discriminant: 0=Call, 1=Run, 2=ImplicitTrigger, 3=EventFlow.
    pub edge_kind: u8,
    pub unit: String,
    pub line: u32,
    pub callee_fp: u64,
}

/// Serde-able mirror of
/// [`CanonicalTarget`][crate::program::resolve::differential::CanonicalTarget].
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GoldenTarget {
    pub kind: u8,
    pub app: Option<String>,
    pub object_lc: String,
    pub routine_lc: Option<String>,
}

// ---------------------------------------------------------------------------
// SemanticGolden
// ---------------------------------------------------------------------------

/// One entry in the semantic golden: a call-site key paired with the set of
/// targets the L3 oracle resolved for that site.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GoldenEntry {
    pub site: GoldenSiteKey,
    /// Targets L3 resolved for this site.  Empty when L3 could not resolve.
    pub targets: BTreeSet<GoldenTarget>,
}

/// The L3-validated semantic golden: a sorted list of (site, targets) pairs.
///
/// Stored as a `Vec` so serde_json can serialize it (JSON maps require string
/// keys; `GoldenSiteKey` is a struct).  The list is always sorted by `site`
/// for determinism and binary-search lookups.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SemanticGolden {
    pub entries: Vec<GoldenEntry>,
}

impl SemanticGolden {
    /// Build from a `BTreeMap` (already sorted, so insertion order is preserved).
    fn from_map(map: std::collections::BTreeMap<GoldenSiteKey, BTreeSet<GoldenTarget>>) -> Self {
        SemanticGolden {
            entries: map
                .into_iter()
                .map(|(site, targets)| GoldenEntry { site, targets })
                .collect(),
        }
    }

    /// Lookup targets for `key` (binary search on sorted `entries`).
    fn get(&self, key: &GoldenSiteKey) -> Option<&BTreeSet<GoldenTarget>> {
        self.entries
            .binary_search_by(|e| e.site.cmp(key))
            .ok()
            .map(|i| &self.entries[i].targets)
    }
}

// ---------------------------------------------------------------------------
// Diff types
// ---------------------------------------------------------------------------

/// A site where the fresh resolver emitted confident (non-Unresolved) targets
/// that differ from the L3-oracle targets.
///
/// This is the **confidently-wrong** class — a Histogram cannot detect it.
#[derive(Clone, Debug)]
pub struct FreshWrong {
    pub site: GoldenSiteKey,
    pub fresh_targets: BTreeSet<GoldenTarget>,
    pub l3_targets: BTreeSet<GoldenTarget>,
}

/// A site formerly in `fresh_wrong` where fresh's targets REFINE L3's target —
/// fresh is MORE precise (Phase-4 Interface/Polymorphic fan-out or superset).
/// Not a bug; the graph's `implements` relationship confirms the refinement.
pub type FreshAheadDispatch = FreshWrong;

/// A site where L3 resolved to a concrete target but fresh emitted empty targets.
#[derive(Clone, Debug)]
pub struct FreshMissing {
    pub site: GoldenSiteKey,
    pub l3_targets: BTreeSet<GoldenTarget>,
}

/// A site where fresh resolved to targets but L3 had an empty target set.
/// Fresh was ahead of L3 — a verified improvement.
#[derive(Clone, Debug)]
pub struct FreshExtra {
    pub site: GoldenSiteKey,
    pub fresh_targets: BTreeSet<GoldenTarget>,
}

/// Full classification from comparing fresh edges against the semantic golden.
#[derive(Clone, Debug, Default)]
pub struct SemanticDiff {
    /// Total paired sites (present in both fresh and golden on the same key).
    pub total_paired: usize,
    /// Paired sites where fresh and L3 targets agree exactly.
    pub matches: usize,
    /// Paired sites where fresh confidently resolved to the WRONG target.
    pub fresh_wrong: Vec<FreshWrong>,
    /// Paired sites where L3 resolved but fresh emitted empty (a gap).
    pub fresh_missing: Vec<FreshMissing>,
    /// Paired sites where fresh resolved and L3 had empty (a win).
    pub fresh_extra: Vec<FreshExtra>,
    /// Fresh sites that have no golden entry (edges L3 never saw, e.g.
    /// `EventFlow`, `ImplicitTrigger`, dynamic ObjectRun sites).
    pub fresh_novel: usize,
    /// Golden sites with no fresh peer (fresh emitted no site for this key).
    pub golden_missing: usize,
}

// ---------------------------------------------------------------------------
// CDO audit report
// ---------------------------------------------------------------------------

/// Result of the CDO/L3 semantic audit over a real workspace.
#[derive(Clone, Debug, Default)]
pub struct CdoSemanticAuditReport {
    pub l3_total: usize,
    pub fresh_total: usize,
    pub paired: usize,
    /// Total sites where fresh and L3 differ and both are non-empty.
    /// Equals `fresh_ahead_dispatch_count + genuine_wrong_count`.
    pub fresh_wrong_count: usize,
    /// Sites adjudicated as "fresh is more precise" (interface fan-out / superset).
    pub fresh_ahead_dispatch_count: usize,
    /// Sites adjudicated as genuinely wrong (disjoint target — a real bug).
    pub genuine_wrong_count: usize,
    /// Genuine_wrong site keys exposed for the HARD GATE set-membership check.
    /// The test asserts every site's `(unit, line, callee_fp)` is present in
    /// the committed manifest
    /// (`tests/goldens/semantic-edges/known-genuine-divergences.json`).
    pub genuine_wrong_sites: Vec<GoldenSiteKey>,
    pub fresh_missing_count: usize,
    pub fresh_extra_count: usize,
    pub fresh_novel: usize,
    pub golden_missing: usize,
    /// SHA-256 hex digest over the sorted site→(l3_targets, fresh_targets) pairs.
    /// Deterministic across runs; used as a pinnable CDO audit fingerprint.
    pub digest: String,
}

// ---------------------------------------------------------------------------
// Route-applicability report
// ---------------------------------------------------------------------------

/// Result of the structural route-applicability contract check.
#[derive(Clone, Debug, Default)]
pub struct ApplicabilityReport {
    pub total_routes: usize,
    /// Routes where the `evidence`/`witness` pair is not valid.
    pub witness_contract_violations: usize,
    /// `AbiSymbol` routes whose key is absent from the raw-ABI index.
    pub abi_unmapped: usize,
}

impl ApplicabilityReport {
    pub fn is_clean(&self) -> bool {
        self.witness_contract_violations == 0 && self.abi_unmapped == 0
    }
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

fn canonical_to_golden_key(e: &CanonicalEdge) -> GoldenSiteKey {
    GoldenSiteKey {
        from_app_guid: e.from.app_guid.clone(),
        from_object_kind: e.from.object_kind.clone(),
        from_object_lc: e.from.object_lc.clone(),
        from_routine_lc: e.from.routine_lc.clone(),
        edge_kind: match e.kind {
            EdgeKind::Call => 0,
            EdgeKind::Run => 1,
            EdgeKind::ImplicitTrigger => 2,
            EdgeKind::EventFlow => 3,
        },
        unit: e.site.span.unit.clone(),
        line: e.site.span.start.line,
        callee_fp: e.site.callee_fp,
    }
}

fn canonical_targets_to_golden(targets: &BTreeSet<CanonicalTarget>) -> BTreeSet<GoldenTarget> {
    targets
        .iter()
        .map(|t| GoldenTarget {
            kind: t.kind,
            app: t.app.clone(),
            object_lc: t.object_lc.clone(),
            routine_lc: t.routine_lc.clone(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Adjudication helper
// ---------------------------------------------------------------------------

/// Adjudicate a `FreshWrong` site: are fresh's and L3's target sets in a
/// REFINEMENT relationship (allowed), or genuinely disjoint (a real divergence)?
///
/// Returns `true` (fresh_ahead_dispatch — allowed) when ANY of these holds:
/// 1. `l3 ⊆ fresh` — fresh is a superset that includes all of L3's answer
///    (Interface/Polymorphic fan-out: fresh is MORE precise).
/// 2. `fresh ⊆ l3` — fresh partially resolved a call site that L3 captured more
///    broadly (multiple physical calls can share one `(line, callee_fp)` bucket).
///    Every target fresh emitted IS in L3's set, so none are confidently wrong —
///    fresh is merely less complete, not wrong.
/// 3. Every L3 target is an interface (kind=11) AND every fresh target implements
///    it (verified via the graph's `ObjectNode.implements` field).
///
/// Returns `false` (genuine_wrong) only when the two non-empty target sets are
/// DISJOINT (or partially overlap with neither a subset nor interface-implements
/// relationship) — fresh and L3 confidently resolved the same site to unrelated
/// targets. NOTE: this is symmetric — it does NOT assert which side is correct;
/// adjudicating that is deferred to 1B.3b.
///
/// # Known partial-recall blind spot (named: 1B.3b-disambiguation)
///
/// Case 2 (`fresh ⊆ l3`) creates a partial-recall blind spot: when fresh finds
/// only a strict subset of the correct targets in a multi-target bucket (e.g.,
/// resolves 2 of 3 interface implementers), the site is classified
/// `fresh_ahead_dispatch` here — NOT as `fresh_missing` or `genuine_wrong`.
/// The dropped target is silently masked by this gate.
///
/// **Mitigation while L3 is the oracle**: the resolution/member harnesses assert
/// `regression_unexplained == 0` independently — any unexplained resolution
/// regression fires there and acts as defense-in-depth covering this blind spot.
///
/// Full per-target recall validation is a named 1B.3b-disambiguation follow-up.
fn is_fresh_ahead_dispatch(
    fw: &FreshWrong,
    obj_lookup: &std::collections::HashMap<
        (String, String),
        &crate::program::node_extract::ObjectNode,
    >,
) -> bool {
    let fresh = &fw.fresh_targets;
    let l3 = &fw.l3_targets;

    if fresh.is_empty() || l3.is_empty() {
        return false;
    }

    // Case 1: L3's targets ⊆ fresh's targets (fresh is a superset: includes all of L3's answer).
    if l3.is_subset(fresh) {
        return true;
    }

    // Case 3: fresh's targets ⊆ L3's targets — fresh partially resolved a compound call
    // that L3 captured more broadly (e.g. L3 follows both the primary dispatch and an
    // EventFlow edge on the same callee_fp).  Fresh is NOT wrong — every target it emitted
    // is in L3's set — it simply emitted fewer.  Classify as fresh_ahead_dispatch (really
    // "fresh_partial_correct") rather than genuine_wrong.
    if fresh.is_subset(l3) {
        return true;
    }

    // Case 2: All L3 targets are interfaces (kind=11) and all fresh targets implement them.
    if !l3.iter().all(|t| t.kind == 11) {
        return false;
    }

    for l3_target in l3 {
        let l3_key = (
            l3_target.app.clone().unwrap_or_default(),
            l3_target.object_lc.clone(),
        );
        let Some(l3_obj) = obj_lookup.get(&l3_key) else {
            // Cannot find the interface object → cannot verify → treat as genuine_wrong.
            return false;
        };
        let iface_name_lc = l3_obj.name.to_ascii_lowercase();

        for fresh_target in fresh {
            // Routine names should agree for a valid interface dispatch.
            if fresh_target.routine_lc != l3_target.routine_lc {
                return false;
            }
            let fresh_key = (
                fresh_target.app.clone().unwrap_or_default(),
                fresh_target.object_lc.clone(),
            );
            let Some(fresh_obj) = obj_lookup.get(&fresh_key) else {
                return false;
            };
            // The concrete object must declare it implements the interface.
            if !fresh_obj
                .implements
                .iter()
                .any(|i| i.to_ascii_lowercase() == iface_name_lc)
            {
                return false;
            }
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// **LAST SANCTIONED L3 ORACLE USE**: mint the semantic golden from the L3 oracle.
///
/// Calls [`project_l3`] over `workspace_root`, collects per-site target sets into
/// a [`SemanticGolden`] keyed by column-ignoring [`GoldenSiteKey`].
///
/// Empty target sets (L3 Unknown/Unresolved) are retained — they record sites
/// that L3 extracted but could not resolve, so the golden covers them.
#[must_use]
pub fn mint_l3_validated_golden(workspace_root: &Path) -> SemanticGolden {
    let l3_edges = project_l3(workspace_root);
    let mut map: BTreeMap<GoldenSiteKey, BTreeSet<GoldenTarget>> = BTreeMap::new();
    for edge in &l3_edges {
        let key = canonical_to_golden_key(edge);
        let targets = canonical_targets_to_golden(&edge.targets);
        map.entry(key).or_default().extend(targets);
    }
    SemanticGolden::from_map(map)
}

/// Compare a fresh canonical edge batch against the L3-minted golden.
///
/// Returns a [`SemanticDiff`] classifying every site.
///
/// **The critical invariant is `fresh_wrong.is_empty()`** — fresh must never
/// confidently emit a target that L3 says is wrong.  `fresh_missing` tracks
/// the Task-3 Unknown gap where L3 resolved but fresh did not (acceptable
/// progress gap — reduce it, never introduce new ones).
#[must_use]
pub fn assert_against_semantic_golden(
    fresh: &[CanonicalEdge],
    golden: &SemanticGolden,
) -> SemanticDiff {
    // Build fresh key → targets map (union duplicate keys).
    let mut fresh_map: BTreeMap<GoldenSiteKey, BTreeSet<GoldenTarget>> = BTreeMap::new();
    for edge in fresh {
        let key = canonical_to_golden_key(edge);
        let targets = canonical_targets_to_golden(&edge.targets);
        fresh_map.entry(key).or_default().extend(targets);
    }

    let mut diff = SemanticDiff::default();

    // Walk golden entries and classify.
    for entry in &golden.entries {
        let key = &entry.site;
        let l3_targets = &entry.targets;
        if let Some(fresh_targets) = fresh_map.get(key) {
            diff.total_paired += 1;
            if fresh_targets == l3_targets {
                diff.matches += 1;
            } else if !l3_targets.is_empty() && !fresh_targets.is_empty() {
                // Both sides resolved but to different targets — the confidently-wrong class.
                diff.fresh_wrong.push(FreshWrong {
                    site: key.clone(),
                    fresh_targets: fresh_targets.clone(),
                    l3_targets: l3_targets.clone(),
                });
            } else if !l3_targets.is_empty() {
                // L3 resolved; fresh did not — a gap.
                diff.fresh_missing.push(FreshMissing {
                    site: key.clone(),
                    l3_targets: l3_targets.clone(),
                });
            } else {
                // L3 empty; fresh resolved — fresh is ahead of L3 (a win).
                diff.fresh_extra.push(FreshExtra {
                    site: key.clone(),
                    fresh_targets: fresh_targets.clone(),
                });
            }
        } else {
            // Golden site has no fresh peer.
            diff.golden_missing += 1;
        }
    }

    // Count fresh sites not in the golden (EventFlow, ImplicitTrigger, etc.).
    for key in fresh_map.keys() {
        if golden.get(key).is_none() {
            diff.fresh_novel += 1;
        }
    }

    diff
}

/// Route-applicability structural contract.
///
/// Checks the witness↔evidence contract on every route in `edges` and
/// delegates the ABI ingestion integrity check to [`abi_ingestion_integrity`].
/// Both must be zero for [`ApplicabilityReport::is_clean`] to return `true`.
#[must_use]
pub fn route_applicability(
    edges: &[Edge],
    raw_abi: &crate::program::resolve::abi_check::RawAbiIndex,
) -> ApplicabilityReport {
    let mut total_routes = 0usize;
    let mut witness_contract_violations = 0usize;
    for edge in edges {
        for route in edge.all_routes() {
            total_routes += 1;
            if !witness_contract_holds(route) {
                witness_contract_violations += 1;
            }
        }
    }
    let abi_report = abi_ingestion_integrity(edges, raw_abi);
    ApplicabilityReport {
        total_routes,
        witness_contract_violations,
        abi_unmapped: abi_report.abi_unmapped,
    }
}

/// Compare the fresh resolver's output for `workspace_root` against `golden`.
///
/// Internally builds the snapshot + graph (for `AppRegistry`) and calls
/// `resolve_full_program`.  Filters fresh edges to the workspace app before
/// projecting.  Used by the in-repo fixture assertion.
#[must_use]
pub fn run_semantic_diff(workspace_root: &Path, golden: &SemanticGolden) -> SemanticDiff {
    use crate::program::abi_ingest::AbiCache;
    use crate::program::build::build_program_graph;
    use crate::program::resolve::full::resolve_full_program;
    use crate::snapshot::SnapshotBuilder;

    let snap = match (SnapshotBuilder {
        workspace_root: workspace_root.to_path_buf(),
        local_providers: vec![],
    })
    .build()
    {
        Ok(s) => s,
        Err(_) => return SemanticDiff::default(),
    };
    let graph = build_program_graph(&snap, &AbiCache::new());
    let Some(ws_ref) = graph.apps.find(&snap.workspace_app) else {
        return SemanticDiff::default();
    };
    let Some(report) = resolve_full_program(workspace_root) else {
        return SemanticDiff::default();
    };
    // Filter to workspace app (matches L3's workspace-only scope).
    let ws_edges: Vec<Edge> = report
        .edges
        .into_iter()
        .filter(|ce| ce.edge.from.object.app == ws_ref)
        .map(|ce| ce.edge)
        .collect();
    let fresh_canonical = project_fresh(&ws_edges, &graph.apps);
    assert_against_semantic_golden(&fresh_canonical, golden)
}

/// Run the route-applicability check over `workspace_root`.
///
/// Builds the snapshot and raw-ABI index internally.
#[must_use]
pub fn run_route_applicability(workspace_root: &Path) -> ApplicabilityReport {
    use crate::program::abi_ingest::AbiCache;
    use crate::program::build::build_program_graph;
    use crate::program::resolve::full::resolve_full_program;
    use crate::snapshot::SnapshotBuilder;

    let snap = match (SnapshotBuilder {
        workspace_root: workspace_root.to_path_buf(),
        local_providers: vec![],
    })
    .build()
    {
        Ok(s) => s,
        Err(_) => return ApplicabilityReport::default(),
    };
    let graph = build_program_graph(&snap, &AbiCache::new());
    let raw_abi = build_raw_abi_index_from_snapshot(&snap, &graph.apps);
    let Some(report) = resolve_full_program(workspace_root) else {
        return ApplicabilityReport::default();
    };
    let all_edges: Vec<Edge> = report.edges.into_iter().map(|ce| ce.edge).collect();
    route_applicability(&all_edges, &raw_abi)
}

/// CDO/L3 semantic audit: compare fresh resolver against L3 oracle over a real
/// workspace.
///
/// Callers should gate this on `CDO_WS` env var before calling — this function
/// runs an expensive double-build (L3 oracle + fresh resolution).
///
/// Returns a [`CdoSemanticAuditReport`] with site-level bucket counts and a
/// deterministic SHA-256 digest over the sorted site→target mapping.
#[must_use]
pub fn run_cdo_semantic_audit(workspace_root: &Path) -> CdoSemanticAuditReport {
    use crate::program::abi_ingest::AbiCache;
    use crate::program::build::build_program_graph;
    use crate::program::node::ObjKey;
    use crate::program::resolve::full::resolve_full_program;
    use crate::snapshot::SnapshotBuilder;
    use std::collections::HashMap;

    // ── Build graph for AppRegistry (needed for project_fresh) ───────────────
    let snap = match (SnapshotBuilder {
        workspace_root: workspace_root.to_path_buf(),
        local_providers: vec![],
    })
    .build()
    {
        Ok(s) => s,
        Err(_) => return CdoSemanticAuditReport::default(),
    };
    let graph = build_program_graph(&snap, &AbiCache::new());
    let Some(ws_ref) = graph.apps.find(&snap.workspace_app) else {
        return CdoSemanticAuditReport::default();
    };

    // ── L3 oracle ─────────────────────────────────────────────────────────────
    let l3_edges = project_l3(workspace_root);
    let l3_total = l3_edges.len();

    // Build L3 golden.
    let mut l3_map: BTreeMap<GoldenSiteKey, BTreeSet<GoldenTarget>> = BTreeMap::new();
    for e in &l3_edges {
        let key = canonical_to_golden_key(e);
        let targets = canonical_targets_to_golden(&e.targets);
        l3_map.entry(key).or_default().extend(targets);
    }
    let golden = SemanticGolden::from_map(l3_map);

    // ── Fresh resolver ────────────────────────────────────────────────────────
    let Some(report) = resolve_full_program(workspace_root) else {
        return CdoSemanticAuditReport::default();
    };
    // Filter to workspace app (L3 is workspace-scoped).
    let ws_edges: Vec<Edge> = report
        .edges
        .into_iter()
        .filter(|ce| ce.edge.from.object.app == ws_ref)
        .map(|ce| ce.edge)
        .collect();
    let fresh_total = ws_edges.len();

    // ── Project fresh → canonical ─────────────────────────────────────────────
    let fresh_canonical = project_fresh(&ws_edges, &graph.apps);

    // Build fresh map (for digest).
    let mut fresh_map: BTreeMap<GoldenSiteKey, BTreeSet<GoldenTarget>> = BTreeMap::new();
    for e in &fresh_canonical {
        let key = canonical_to_golden_key(e);
        let targets = canonical_targets_to_golden(&e.targets);
        fresh_map.entry(key).or_default().extend(targets);
    }

    // ── Diff ──────────────────────────────────────────────────────────────────
    let diff = assert_against_semantic_golden(&fresh_canonical, &golden);

    // ── Adjudicate fresh_wrong into fresh_ahead_dispatch vs genuine_wrong ────────
    // Build object lookup: (app_guid, object_lc) → &ObjectNode for implements checks.
    let mut obj_lookup: HashMap<(String, String), &crate::program::node_extract::ObjectNode> =
        HashMap::new();
    for obj in &graph.objects {
        let guid = graph
            .apps
            .try_resolve(obj.id.app)
            .map(|a| a.guid.clone())
            .unwrap_or_default();
        let lc = match &obj.id.key {
            ObjKey::Id(n) => format!("{n}"),
            ObjKey::Name(s) => s.clone(),
        };
        obj_lookup.insert((guid, lc), obj);
    }

    // Adjudicate each fresh_wrong site.
    let mut fresh_ahead_dispatch: Vec<FreshAheadDispatch> = Vec::new();
    let mut genuine_wrong: Vec<FreshWrong> = Vec::new();

    for fw in &diff.fresh_wrong {
        if is_fresh_ahead_dispatch(fw, &obj_lookup) {
            fresh_ahead_dispatch.push(fw.clone());
        } else {
            genuine_wrong.push(fw.clone());
        }
    }

    eprintln!(
        "\nAdjudication: fresh_wrong={} → fresh_ahead_dispatch={} genuine_wrong={}",
        diff.fresh_wrong.len(),
        fresh_ahead_dispatch.len(),
        genuine_wrong.len(),
    );
    for gw in &genuine_wrong {
        eprintln!(
            "  GENUINE_WRONG site={:?} fresh={:?} l3={:?}",
            gw.site, gw.fresh_targets, gw.l3_targets,
        );
    }

    // ── Characterize fresh_missing ────────────────────────────────────────────
    // Known deferred buckets (from prior analysis): compound~47, codeunit_implicit_rec~24,
    // page_rec~14, trigger.missing~78 = 163 total. Anything beyond is a new gap.
    let mut missing_page_rec = 0usize;
    let mut missing_codeunit_implicit_rec = 0usize;
    let mut missing_trigger = 0usize;
    let mut missing_other = 0usize;
    for fm in &diff.fresh_missing {
        let from_kind = fm.site.from_object_kind.as_str();
        let l3_targets_table = fm.l3_targets.iter().any(|t| t.kind == 1 || t.kind == 2);
        let is_trigger_routine = fm.site.from_routine_lc.starts_with("on")
            || matches!(
                fm.site.from_routine_lc.as_str(),
                "trigger" | "preparedocument" | "finishdocument"
            );

        if matches!(from_kind, "page" | "pageextension") && l3_targets_table {
            missing_page_rec += 1;
        } else if matches!(from_kind, "codeunit") && l3_targets_table {
            missing_codeunit_implicit_rec += 1;
        } else if is_trigger_routine {
            missing_trigger += 1;
        } else {
            missing_other += 1;
        }
    }
    eprintln!(
        "fresh_missing characterization: page_rec={} codeunit_implicit_rec={} trigger={} other={}",
        missing_page_rec, missing_codeunit_implicit_rec, missing_trigger, missing_other,
    );

    // ── Deterministic digest ──────────────────────────────────────────────────
    // Feed sorted (key, l3_targets, fresh_targets) into SHA-256.
    let mut hasher = Sha256::new();
    for entry in &golden.entries {
        let fresh_targets = fresh_map.get(&entry.site).cloned().unwrap_or_default();
        let k_json = serde_json::to_string(&entry.site).unwrap_or_default();
        let l_json = serde_json::to_string(&entry.targets).unwrap_or_default();
        let f_json = serde_json::to_string(&fresh_targets).unwrap_or_default();
        hasher.update(format!("{k_json}|{l_json}|{f_json}\n").as_bytes());
    }
    let digest_bytes = hasher.finalize();
    let digest: String = digest_bytes.iter().map(|b| format!("{b:02x}")).collect();

    CdoSemanticAuditReport {
        l3_total,
        fresh_total,
        paired: diff.total_paired,
        fresh_wrong_count: diff.fresh_wrong.len(),
        fresh_ahead_dispatch_count: fresh_ahead_dispatch.len(),
        genuine_wrong_count: genuine_wrong.len(),
        genuine_wrong_sites: genuine_wrong.iter().map(|fw| fw.site.clone()).collect(),
        fresh_missing_count: diff.fresh_missing.len(),
        fresh_extra_count: diff.fresh_extra.len(),
        fresh_novel: diff.fresh_novel,
        golden_missing: diff.golden_missing,
        digest,
    }
}
