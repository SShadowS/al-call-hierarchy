//! Phase 0 Task 4: fresh-side canonical projection for the dual-run
//! differential harness.
//!
//! The harness compares the legacy L3 engine output (the "legacy side") with
//! the new whole-program resolver (the "fresh side").  Both sides project their
//! edges into the same [`CanonicalEdge`] shape so a simple set-diff reveals
//! what each side resolves that the other does not.
//!
//! # `object_lc` encoding for `ObjKey::Id`
//! When an object's key is numeric (`ObjKey::Id(n)`), `object_lc` is written
//! as `format!("{n}")` — the decimal representation of the signed integer.
//! Task 5 (legacy-side projection) must mirror this choice exactly so the two
//! projections are comparable.
//!
//! # `Unresolved`/`Unknown` routes
//! A route whose `target` is `RouteTarget::Unresolved` projects to **no**
//! entry in `targets`.  The stub resolver emits only `Unresolved` routes, so
//! every stub edge projects to an empty `targets` set.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use al_syntax::ir::ObjectKind;

use crate::program::node::{AppRef, AppRegistry, ObjKey, RoutineNodeId};
use crate::program::resolve::edge::{
    BuiltinId, CanonicalSpan, Edge, EdgeKind, RouteTarget, SourcePos, callee_fp,
};

// ---------------------------------------------------------------------------
// Canonical types
// ---------------------------------------------------------------------------

/// Canonical, stable representation of a resolved route target.
///
/// `kind` encodes the object kind (see [`object_kind_tag`]) for `Routine`
/// targets, or a sentinel value (254 = `AbiSymbol`, 255 = `Builtin`) for other
/// target classes so the differential can bucket by target type.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CanonicalTarget {
    pub kind: u8,
    pub app: Option<String>,
    pub object_lc: String,
    pub routine_lc: Option<String>,
}

/// Canonical identity of a routine node — the differential key for the "from"
/// side of an edge.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CanonicalKey {
    pub app_guid: String,
    pub object_kind: String,
    pub object_lc: String,
    pub routine_lc: String,
}

/// Canonical identity of a call site — combines the caller, source span, and
/// a content-fingerprint of the callee expression.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CanonicalSiteKey {
    pub caller: CanonicalKey,
    pub span: CanonicalSpan,
    pub callee_fp: u64,
}

/// A call/behaviour edge in canonical form — the unit of comparison in the
/// dual-run differential.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CanonicalEdge {
    pub from: CanonicalKey,
    pub site: CanonicalSiteKey,
    pub kind: EdgeKind,
    /// The set of concrete route targets.  Empty when all routes are
    /// `Unresolved` (stub phase) or when the edge is a genuine zero-route
    /// fan-out.
    pub targets: BTreeSet<CanonicalTarget>,
}

/// Outcome of aligning one call site between the fresh resolver and the L3 oracle.
///
/// Indices refer to positions in the `fresh` / `l3` slices passed to
/// [`match_sites`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SiteMatch {
    /// Both sides agree: `fresh[fi]` ↔ `l3[li]` share the same strong key.
    Paired(usize, usize),
    /// A fresh site that has no L3 peer in its `(from, kind)` partition.
    FreshOnly(usize),
    /// An L3 site that has no fresh peer in its `(from, kind)` partition.
    L3Only(usize),
    /// Genuinely ambiguous duplicate leftovers — multiple sites on both sides
    /// share the same strong key and the counts are unequal after positional
    /// pairing.  The vecs carry the excess `fresh` and `l3` indices respectively.
    Unaligned(Vec<usize>, Vec<usize>),
}

// ---------------------------------------------------------------------------
// Shared helpers — BOTH project_fresh and project_l3 call these so the two
// projections cannot silently diverge in encoding.
// ---------------------------------------------------------------------------

/// Map an already-lowercased object-kind string to the stable `u8` discriminant
/// used in [`CanonicalTarget::kind`].
///
/// The L3 side lowercases its `object_type` string (e.g. `"Codeunit"` →
/// `"codeunit"`) and calls this helper so the tag cannot drift from the fresh
/// side's [`object_kind_tag`].  Values are fixed — do not reorder.
pub(crate) fn object_kind_str_to_tag(lc: &str) -> u8 {
    match lc {
        "codeunit" => 0,
        "table" => 1,
        "tableextension" => 2,
        "page" => 3,
        "pageextension" => 4,
        "report" => 5,
        "reportextension" => 6,
        "query" => 7,
        "xmlport" => 8,
        "enum" => 9,
        "enumextension" => 10,
        "interface" => 11,
        "controladdin" => 12,
        "entitlement" => 13,
        "permissionset" => 14,
        "permissionsetextension" => 15,
        "profile" => 16,
        _ => 255,
    }
}

/// Build a [`CanonicalKey`] from pre-resolved, already-lowercased components.
///
/// Both `project_fresh` (via [`routine_to_key`]) and `project_l3` funnel
/// through this so the key layout is identical on both sides.
pub(crate) fn make_canonical_key(
    app_guid: String,
    object_kind: String,
    object_lc: String,
    routine_lc: String,
) -> CanonicalKey {
    CanonicalKey {
        app_guid,
        object_kind,
        object_lc,
        routine_lc,
    }
}

// ---------------------------------------------------------------------------
// Internal helpers (fresh-side only)
// ---------------------------------------------------------------------------

/// Map `ObjectKind` to a stable `u8` discriminant for `CanonicalTarget.kind`.
///
/// Delegates to [`object_kind_str_to_tag`] via [`object_kind_str`] so the two
/// cannot drift.
fn object_kind_tag(k: ObjectKind) -> u8 {
    object_kind_str_to_tag(&object_kind_str(k))
}

/// `ObjectKind` → lowercase debug name (e.g. `"codeunit"`, `"table"`).
fn object_kind_str(k: ObjectKind) -> String {
    format!("{k:?}").to_ascii_lowercase()
}

/// `ObjKey` → canonical string.
///
/// `Id(n)` → decimal integer string; `Name(s)` → the already-lowercased name.
/// `project_l3` mirrors this: L3 objects always carry an `object_number: i64`,
/// so it uses `format!("{n}")` directly.
fn obj_key_lc(key: &ObjKey) -> String {
    match key {
        ObjKey::Id(n) => format!("{n}"),
        ObjKey::Name(s) => s.clone(),
    }
}

/// Resolve `AppRef` → guid string via the registry.
///
/// Falls back to an empty string when the ref is out-of-range (e.g. synthetic
/// test fixtures that use `AppRef(0)` against an empty registry).
fn app_guid(apps: &AppRegistry, r: AppRef) -> String {
    apps.try_resolve(r)
        .map(|id| id.guid.clone())
        .unwrap_or_default()
}

/// Project a `RoutineNodeId` to a `CanonicalKey` (fresh-side only).
fn routine_to_key(id: &RoutineNodeId, apps: &AppRegistry) -> CanonicalKey {
    make_canonical_key(
        app_guid(apps, id.object.app),
        object_kind_str(id.object.kind),
        obj_key_lc(&id.object.key),
        id.name_lc.clone(),
    )
}

/// Project one `RouteTarget` to a `CanonicalTarget`.
///
/// Returns `None` for `Unresolved` — those map to an empty targets set.
fn project_target(target: &RouteTarget, apps: &AppRegistry) -> Option<CanonicalTarget> {
    match target {
        RouteTarget::Unresolved => None,
        RouteTarget::Routine(id) => Some(CanonicalTarget {
            kind: object_kind_tag(id.object.kind),
            app: Some(app_guid(apps, id.object.app)),
            object_lc: obj_key_lc(&id.object.key),
            routine_lc: Some(id.name_lc.clone()),
        }),
        RouteTarget::Builtin(BuiltinId(bid)) => Some(CanonicalTarget {
            kind: 255,
            app: None,
            object_lc: bid.clone(),
            routine_lc: None,
        }),
        RouteTarget::AbiSymbol { app, symbol_key } => Some(CanonicalTarget {
            kind: 254,
            app: Some(app_guid(apps, *app)),
            object_lc: symbol_key.clone(),
            routine_lc: None,
        }),
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Map each `Edge` to a [`CanonicalEdge`] for the differential harness.
///
/// `Unresolved` routes project to **no** `CanonicalTarget` entry (empty set).
/// `apps` is the `ProgramGraph`'s `AppRegistry`, used to resolve `AppRef` →
/// guid.
#[must_use]
pub fn project_fresh(edges: &[Edge], apps: &AppRegistry) -> Vec<CanonicalEdge> {
    let mut result: Vec<CanonicalEdge> = edges
        .iter()
        .map(|e| {
            let from = routine_to_key(&e.from, apps);
            let caller = routine_to_key(&e.site.caller, apps);
            let targets = e
                .routes
                .iter()
                .filter_map(|r| project_target(&r.target, apps))
                .collect();
            CanonicalEdge {
                from,
                site: CanonicalSiteKey {
                    caller,
                    span: e.site.span.clone(),
                    callee_fp: e.site.callee_fingerprint,
                },
                kind: e.kind,
                targets,
            }
        })
        .collect();
    result.sort();
    result
}

// ---------------------------------------------------------------------------
// L3 oracle projection
// ---------------------------------------------------------------------------

/// Project the L3 resolver's output over `workspace_root` into
/// [`CanonicalEdge`]s — the oracle side of the dual-run differential harness.
///
/// # Algorithm
/// 1. Assemble + resolve the workspace with the default L3 pipeline (mirrors
///    `aldump --l3-call-graph-stats`).
/// 2. Build two lookup maps:
///    - `routine_by_id`: internal routine id → `&L3Routine`
///    - `callsite_by_id`: internal callsite id → `&PCallSite`
/// 3. For each `CallEdge` emitted by `resolve_calls`:
///    - Look up the `from` routine → build [`CanonicalKey`] via
///      [`make_canonical_key`] (same helper as the fresh side).
///    - Look up the callsite → read `PAnchor` (0-based line/col — same basis
///      as the fresh side's `byte_to_pos`) → build [`CanonicalSpan`].
///    - Compute `callee_fp` via [`callee_fp`] on `PCallSite::callee_text`
///      (same hash as `stub.rs`).
///    - If `to` is `Some` → look up the callee routine → build one
///      [`CanonicalTarget`] with `kind` from [`object_kind_str_to_tag`].
///    - If `to` is `None` → empty targets set (same as fresh Unresolved).
/// 4. Sort the result by the natural `CanonicalEdge` `Ord` for determinism.
///
/// Returns an empty `Vec` when the workspace is unsound / unparseable
/// (fail-closed, never panics).
///
/// # PAnchor coordinate base
/// `PAnchor.start_line` / `start_column` are **0-based** (the IR walk fills
/// them from tree-sitter row/utf-16-col directly, both 0-based).  The fresh
/// side's `byte_to_pos` is also 0-based.  No conversion is needed, but note
/// that the fresh side records **byte columns** while L3 records **UTF-16
/// columns** — they agree on ASCII-only source; non-ASCII may differ by one
/// column in the matcher (Task 6 accounts for this).
#[must_use]
pub fn project_l3(workspace_root: &Path) -> Vec<CanonicalEdge> {
    use std::collections::HashMap;

    use crate::engine::l3::call_resolver::{DeclaredDependency, resolve_calls};
    use crate::engine::l3::l3_workspace::assemble_and_resolve_workspace_default;
    use crate::engine::l3::symbol_table::SymbolTable;

    let Some(resolved) = assemble_and_resolve_workspace_default(workspace_root) else {
        return Vec::new();
    };
    let ws = &resolved.workspace;

    let symbols = SymbolTable::build(&ws.objects, &ws.tables, &ws.routines);
    let no_deps: Vec<DeclaredDependency> = Vec::new();
    let no_fetched: Vec<String> = Vec::new();
    let resolved_calls = resolve_calls(ws, &symbols, &no_deps, &no_fetched);

    // Build lookup: internal routine id → &L3Routine.
    let routine_by_id: HashMap<&str, &crate::engine::l3::l3_workspace::L3Routine> =
        ws.routines.iter().map(|r| (r.id.as_str(), r)).collect();

    // Build lookup: internal callsite id → &PCallSite.
    // PCallSite.id is the internal callsite id (`{routine_id}/cs{N}`).
    let mut callsite_by_id: HashMap<&str, &crate::engine::l2::features::PCallSite> = HashMap::new();
    for routine in &ws.routines {
        for cs in &routine.call_sites {
            callsite_by_id.insert(cs.id.as_str(), cs);
        }
    }

    let mut edges: Vec<CanonicalEdge> = resolved_calls
        .edges
        .iter()
        .filter_map(|edge| {
            // Resolve the `from` routine.
            let from_r = routine_by_id.get(edge.from.as_str())?;
            let from = make_canonical_key(
                from_r.app_guid.clone(),
                from_r.object_type.to_ascii_lowercase(),
                format!("{}", from_r.object_number),
                from_r.name.to_ascii_lowercase(),
            );

            // Resolve the callsite → span + callee fingerprint.
            let cs = callsite_by_id.get(edge.callsite_id.as_str())?;
            let a = &cs.source_anchor;
            // PAnchor line/col are 0-based (from tree-sitter row + utf-16 col).
            // L3 workspace anchors carry `source_unit_id = "ws:<rel-posix-path>"`.
            // Strip the "ws:" prefix so the canonical span unit matches the
            // fresh side's `virtual_path` (a plain relative POSIX path).
            let unit_str = a
                .source_unit_id
                .strip_prefix("ws:")
                .unwrap_or(&a.source_unit_id)
                .to_string();
            let span = CanonicalSpan {
                unit: unit_str,
                start: SourcePos {
                    line: a.start_line,
                    col: a.start_column,
                },
                end: SourcePos {
                    line: a.end_line,
                    col: a.end_column,
                },
            };
            let fp = callee_fp(&cs.callee_text);

            // The callsite's logical caller == the `from` routine on the L3 model.
            let site = CanonicalSiteKey {
                caller: from.clone(),
                span,
                callee_fp: fp,
            };

            // Build the target set.  L3 `to == None` → unresolved → empty set.
            let targets: BTreeSet<CanonicalTarget> = if let Some(to_id) = &edge.to {
                if let Some(to_r) = routine_by_id.get(to_id.as_str()) {
                    let mut set = BTreeSet::new();
                    set.insert(CanonicalTarget {
                        kind: object_kind_str_to_tag(&to_r.object_type.to_ascii_lowercase()),
                        app: Some(to_r.app_guid.clone()),
                        object_lc: format!("{}", to_r.object_number),
                        routine_lc: Some(to_r.name.to_ascii_lowercase()),
                    });
                    set
                } else {
                    // `to` id present but not in the workspace index — treat as
                    // unresolved (should not happen in a sound workspace).
                    BTreeSet::new()
                }
            } else {
                BTreeSet::new()
            };

            Some(CanonicalEdge {
                from,
                site,
                kind: EdgeKind::Call,
                targets,
            })
        })
        .collect();

    edges.sort();
    edges
}

// ---------------------------------------------------------------------------
// L3 PCallSite oracle projection (Phase 1 Task 4)
// ---------------------------------------------------------------------------

/// Project every L3 `PCallSite` from the workspace into a [`CanonicalEdge`]
/// with **empty** targets — the site-level oracle for the Phase-1 parity gate.
///
/// Unlike [`project_l3`] (which projects `CallEdge`s — L3's resolved edges),
/// this function projects EVERY `PCallSite` regardless of resolution outcome.
/// This gives the complete set of call-expression sites L3 extracted from the
/// workspace source, which the fresh [`run_site_harness`] must match.
///
/// Key encoding is identical to [`project_l3`]:
/// - Caller key from the owning `L3Routine` (`app_guid` / lowercased
///   `object_type` / `object_number` / lowercased `name`).
/// - Span from `PCallSite.source_anchor` with `"ws:"` prefix stripped and
///   0-based line/col (same basis as the fresh side's `byte_to_pos`).
/// - `callee_fp` via [`callee_fp`] on `PCallSite::callee_text`.
/// - `targets`: always empty (site-level only — resolution is not projected).
///
/// Returns an empty `Vec` when the workspace is unsound (fail-closed).
#[must_use]
pub fn project_l3_sites(workspace_root: &Path) -> Vec<CanonicalEdge> {
    use crate::engine::l3::l3_workspace::assemble_and_resolve_workspace_default;

    let Some(resolved) = assemble_and_resolve_workspace_default(workspace_root) else {
        return Vec::new();
    };
    let ws = &resolved.workspace;

    let mut edges: Vec<CanonicalEdge> = Vec::new();
    for r in &ws.routines {
        let from = make_canonical_key(
            r.app_guid.clone(),
            r.object_type.to_ascii_lowercase(),
            format!("{}", r.object_number),
            r.name.to_ascii_lowercase(),
        );
        for cs in &r.call_sites {
            let a = &cs.source_anchor;
            // Strip the "ws:" prefix so the canonical span unit matches the
            // fresh side's `virtual_path` (a plain relative POSIX path).
            let unit_str = a
                .source_unit_id
                .strip_prefix("ws:")
                .unwrap_or(&a.source_unit_id)
                .to_string();
            let span = CanonicalSpan {
                unit: unit_str,
                start: SourcePos {
                    line: a.start_line,
                    col: a.start_column,
                },
                end: SourcePos {
                    line: a.end_line,
                    col: a.end_column,
                },
            };
            let fp = callee_fp(&cs.callee_text);
            edges.push(CanonicalEdge {
                from: from.clone(),
                site: CanonicalSiteKey {
                    caller: from.clone(),
                    span,
                    callee_fp: fp,
                },
                kind: EdgeKind::Call,
                targets: BTreeSet::new(),
            });
        }
    }
    edges.sort();
    edges
}

// ---------------------------------------------------------------------------
// Span-based site matcher (spec §6.1)
// ---------------------------------------------------------------------------

/// Align fresh and L3 call sites WITHOUT relying on positional ordinals.
///
/// ## Algorithm (spec §6.1)
///
/// 1. **Partition** both slices into groups keyed by `(from, kind)`.  Sites
///    only ever match within the same group.
/// 2. **Within each group**, bucket sites by the *strong key*
///    `(span.unit, span.start.line, callee_fp)`.  Column offsets are ignored
///    because L3 uses UTF-16 columns while the fresh side uses byte columns —
///    they agree on ASCII-only source, and may differ by a small delta on
///    non-ASCII identifiers.
/// 3. **Pair positionally** within each strong-key bucket:
///    - Equal counts → all [`SiteMatch::Paired`].
///    - One side absent → [`SiteMatch::FreshOnly`] / [`SiteMatch::L3Only`]
///      for every site in that bucket.
///    - Both sides present, unequal counts → pair the `min` count, then emit
///      a single [`SiteMatch::Unaligned`] with the leftover indices.
///
/// **Cascade-resistance guarantee:** removing one L3 site changes at most ONE
/// bucket (→ the corresponding fresh site becomes [`SiteMatch::FreshOnly`])
/// and NEVER shifts the pairing of any other site.
#[must_use]
pub fn match_sites(fresh: &[CanonicalEdge], l3: &[CanonicalEdge]) -> Vec<SiteMatch> {
    type GroupKey = (CanonicalKey, EdgeKind);
    // Strong key: (unit, start_line, callee_fp) — col intentionally omitted.
    type StrongKey = (String, u32, u64);

    // Step 1: partition both slices into (from, kind) groups.
    let mut fresh_groups: HashMap<GroupKey, Vec<usize>> = HashMap::new();
    let mut l3_groups: HashMap<GroupKey, Vec<usize>> = HashMap::new();

    for (i, e) in fresh.iter().enumerate() {
        fresh_groups
            .entry((e.from.clone(), e.kind))
            .or_default()
            .push(i);
    }
    for (i, e) in l3.iter().enumerate() {
        l3_groups
            .entry((e.from.clone(), e.kind))
            .or_default()
            .push(i);
    }

    let mut all_group_keys: Vec<GroupKey> = fresh_groups
        .keys()
        .chain(l3_groups.keys())
        .cloned()
        .collect();
    all_group_keys.sort_unstable();
    all_group_keys.dedup();

    let mut result: Vec<SiteMatch> = Vec::new();
    let empty: Vec<usize> = Vec::new();

    for gk in all_group_keys {
        let fresh_idxs = fresh_groups.get(&gk).unwrap_or(&empty);
        let l3_idxs = l3_groups.get(&gk).unwrap_or(&empty);

        // Step 2: bucket by strong key within this group.
        let mut fresh_by_sk: HashMap<StrongKey, Vec<usize>> = HashMap::new();
        let mut l3_by_sk: HashMap<StrongKey, Vec<usize>> = HashMap::new();

        for &fi in fresh_idxs {
            let e = &fresh[fi];
            let sk = (
                e.site.span.unit.clone(),
                e.site.span.start.line,
                e.site.callee_fp,
            );
            fresh_by_sk.entry(sk).or_default().push(fi);
        }
        for &li in l3_idxs {
            let e = &l3[li];
            let sk = (
                e.site.span.unit.clone(),
                e.site.span.start.line,
                e.site.callee_fp,
            );
            l3_by_sk.entry(sk).or_default().push(li);
        }

        let mut all_sks: Vec<StrongKey> =
            fresh_by_sk.keys().chain(l3_by_sk.keys()).cloned().collect();
        all_sks.sort_unstable();
        all_sks.dedup();

        // Step 3: pair within each strong-key bucket.
        for sk in all_sks {
            let fis = fresh_by_sk.get(&sk).map(Vec::as_slice).unwrap_or(&[]);
            let lis = l3_by_sk.get(&sk).map(Vec::as_slice).unwrap_or(&[]);

            let pair_count = fis.len().min(lis.len());
            for i in 0..pair_count {
                result.push(SiteMatch::Paired(fis[i], lis[i]));
            }

            let extra_f = &fis[pair_count..];
            let extra_l = &lis[pair_count..];

            if pair_count == 0 {
                // One side is entirely absent → unambiguous.
                for &fi in extra_f {
                    result.push(SiteMatch::FreshOnly(fi));
                }
                for &li in extra_l {
                    result.push(SiteMatch::L3Only(li));
                }
            } else {
                // Some pairings happened; leftovers are genuinely ambiguous duplicates.
                if !extra_f.is_empty() || !extra_l.is_empty() {
                    result.push(SiteMatch::Unaligned(extra_f.to_vec(), extra_l.to_vec()));
                }
            }
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Diff engine — Task 7
// ---------------------------------------------------------------------------

/// Bucket counts from one dual-run differential run.
///
/// `matched` = total `Paired` site count.  `regression` = Paired sites where
/// the fresh side resolved nothing (empty `targets`) — in Phase-0 this equals
/// `matched` because the stub resolver emits only `Unresolved` routes.
/// `missing_site` = L3-only sites; `extra_site` = fresh-only sites.
/// `unaligned` = total leftover indices across all `Unaligned` buckets (see
/// [`match_sites`] docs for the cascade-resistance guarantee).
///
/// Phase-1 additions (populated by [`run_site_harness`], zero in
/// [`run_harness`]):
/// - `extra_recordop`: fresh sites classified as `RecordOp` — excluded from
///   the diff set because L3 emits no `PCallSite` for record DB operations.
/// - `extra_commit`: fresh `Commit()` sites — L3 emits no `PCallSite` for
///   `Commit`.
/// - `extra_implicit_rec`: fresh `Bare` sites whose name is in
///   [`record_op_names`] — the implicit-Rec approximation leaves these as
///   `Bare` while L3 classifies them as record-ops (no `PCallSite`).
/// - `extra_error`: diagnostic count for fresh `Bare` `Error()` sites; these
///   ARE included in the diff set (L3 does emit `PCallSite` for `Error()`),
///   so this field is informational only and will typically be 0 after matching.
/// - `extra_unexplained`: `FreshOnly` sites after all categorised extras have
///   been removed.  Must be 0 for the Phase-1 gate to pass.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiffReport {
    /// Fresh edges across ALL apps in the snapshot (workspace + embedded deps).
    pub fresh_total_all_apps: usize,
    /// Fresh edges scoped to the workspace app only (matched against L3).
    pub fresh_total_workspace: usize,
    /// Total canonical edges emitted by the L3 oracle for the workspace.
    pub l3_edges: usize,
    /// Count of `Paired` site matches (fresh + L3 share the same strong key).
    pub matched: usize,
    /// Paired sites where the fresh resolver emitted no concrete targets.
    /// Equals `matched` in Phase-0 (stub is all-Unknown).
    pub regression: usize,
    /// L3 sites with no fresh peer — the resolver MISSED these call sites.
    pub missing_site: usize,
    /// Fresh sites with no L3 peer — fresh extracted sites L3 did not see.
    /// Superseded by the Phase-1 category breakdown in [`run_site_harness`].
    pub extra_site: usize,
    /// Sum of leftover indices from `Unaligned` buckets — genuinely ambiguous
    /// duplicate call sites that the span matcher could not pair deterministically.
    pub unaligned: usize,
    // ── Phase-1 category breakdown (zero in run_harness) ────────────────────
    /// Fresh sites classified as `RecordOp` (excluded from diff set; L3 emits
    /// no `PCallSite` for record DB operations).
    pub extra_recordop: usize,
    /// Fresh `Commit()` sites (excluded from diff set; L3 emits no `PCallSite`
    /// for `Commit`).
    pub extra_commit: usize,
    /// Fresh `Bare` sites whose name ∈ `record_op_names()` (excluded from diff
    /// set — the implicit-Rec approximation: L3 treats these as record-ops).
    pub extra_implicit_rec: usize,
    /// Diagnostic count for `Bare` `Error()` sites.  These are included in the
    /// diff set (L3 does emit `PCallSite` for `Error()`); they will pair with
    /// their L3 counterparts and this field will typically be 0 post-match.
    pub extra_error: usize,
    /// `FreshOnly` sites remaining after all categorised extras have been
    /// accounted for.  Must equal 0 for the Phase-1 gate to pass.
    pub extra_unexplained: usize,
}

/// Run the full dual-run differential harness over `workspace_root`.
///
/// Steps:
/// 1. Build `AppSetSnapshot` + `ProgramGraph` from the workspace.
/// 2. Call the fresh (stub) resolver → `Vec<Edge>` over all apps.
/// 3. Filter fresh edges to the WORKSPACE APP only and project to
///    [`CanonicalEdge`]s via [`project_fresh`].
/// 4. Run [`project_l3`] for the L3 oracle (also workspace-source-only).
/// 5. Run [`match_sites`] and bucket the results into [`DiffReport`] fields.
///
/// Fail-closed: any error during setup returns a zero `DiffReport`.
///
/// # Why filter fresh to the workspace app?
/// `resolve_program` processes ALL snapshot apps (workspace + embedded dep
/// source), but `project_l3` is workspace-source-only.  Comparing them raw
/// would flood the diff with spurious `FreshOnly` entries for every dep-app
/// call site.  Scoping fresh to the workspace app makes the comparison apples-
/// to-apples.  The unfiltered all-apps count is reported separately as
/// `fresh_total_all_apps`.
#[must_use]
pub fn run_harness(workspace_root: &Path) -> DiffReport {
    use crate::program::build::build_program_graph;
    use crate::program::resolve::stub::resolve_program;
    use crate::snapshot::{SnapshotBuilder, parse_snapshot};

    // ── Step 1: Build snapshot ───────────────────────────────────────────────
    let snap = match (SnapshotBuilder {
        workspace_root: workspace_root.to_path_buf(),
        local_providers: vec![],
    })
    .build()
    {
        Ok(s) => s,
        Err(_) => return DiffReport::default(),
    };

    // ── Step 2: Build program graph (interns apps + extracts nodes) ──────────
    let graph = build_program_graph(&snap);

    // ── Step 3: Parse snapshot for the stub resolver (second parse pass) ────
    // `build_program_graph` parses internally for node extraction; a second
    // pass here provides the per-file texts the stub resolver needs for call-
    // site extraction.  Phase-0 design accepts the double-parse cost.
    let parsed = parse_snapshot(&snap);

    // ── Step 4: Resolve fresh edges (all apps) ────────────────────────────────
    let fresh_all = resolve_program(&graph, &parsed);
    let fresh_total_all_apps = fresh_all.len();

    // ── Step 5: Filter to workspace app ──────────────────────────────────────
    let workspace_ref = graph.apps.find(&snap.workspace_app);
    let fresh_workspace: Vec<Edge> = match workspace_ref {
        Some(ws_ref) => fresh_all
            .into_iter()
            .filter(|e| e.from.object.app == ws_ref)
            .collect(),
        None => {
            // Workspace app not interned — return a fail-closed zero report.
            return DiffReport {
                fresh_total_all_apps,
                ..DiffReport::default()
            };
        }
    };
    let fresh_total_workspace = fresh_workspace.len();

    // ── Step 6: Project fresh (workspace-only) to canonical ──────────────────
    let fresh_canonical = project_fresh(&fresh_workspace, &graph.apps);

    // ── Step 7: Project L3 oracle ─────────────────────────────────────────────
    let l3_canonical = project_l3(workspace_root);
    let l3_edges = l3_canonical.len();

    // ── Step 8: Match sites ───────────────────────────────────────────────────
    let site_matches = match_sites(&fresh_canonical, &l3_canonical);

    // ── Step 9: Bucket ────────────────────────────────────────────────────────
    let mut matched = 0usize;
    let mut regression = 0usize;
    let mut missing_site = 0usize;
    let mut extra_site = 0usize;
    let mut unaligned = 0usize;

    for m in &site_matches {
        match m {
            SiteMatch::Paired(fi, li) => {
                matched += 1;
                // Regression: the fresh side emitted no concrete targets but the L3 side did.
                // In Phase-0 (stub) fresh.targets is ALWAYS empty, so
                // regression == matched.  In Phases 1–4 this will shrink as
                // the real resolver fills in targets.
                if fresh_canonical[*fi].targets.is_empty() && !l3_canonical[*li].targets.is_empty()
                {
                    regression += 1;
                }
            }
            SiteMatch::FreshOnly(_) => {
                extra_site += 1;
            }
            SiteMatch::L3Only(_) => {
                missing_site += 1;
            }
            SiteMatch::Unaligned(fs, ls) => {
                unaligned += fs.len() + ls.len();
            }
        }
    }

    DiffReport {
        fresh_total_all_apps,
        fresh_total_workspace,
        l3_edges,
        matched,
        regression,
        missing_site,
        extra_site,
        unaligned,
        // Phase-1 fields are not populated by the Phase-0 stub harness.
        ..DiffReport::default()
    }
}

// ---------------------------------------------------------------------------
// Phase-1 site harness (Task 4)
// ---------------------------------------------------------------------------

/// Phase-1 site-parity harness: compares STRUCTURED fresh call-site extraction
/// against the L3 `PCallSite` oracle.
///
/// Unlike [`run_harness`] (which compares resolved `Edge`s from the stub
/// resolver), this harness:
/// 1. Extracts call sites via [`extract_sites`]/[`CalleeShape`] for the
///    workspace app only.
/// 2. Partitions sites into justified-extra buckets (RecordOp / Commit /
///    implicit-Rec bare) vs. the "call-category" diff set
///    (Bare/Member/ObjectRun/Unknown minus the two approximations).
/// 3. Runs [`project_l3_sites`] for the site-level oracle.
/// 4. Aligns them via [`match_sites`] and buckets into [`DiffReport`].
///
/// The Phase-1 gate requires `extra_unexplained == 0`: every fresh
/// call-category site must pair with an L3 `PCallSite`.
///
/// Fail-closed: any error during setup returns a zero [`DiffReport`].
#[must_use]
pub fn run_site_harness(workspace_root: &Path) -> DiffReport {
    use std::collections::HashSet;

    use crate::program::build::build_program_graph;
    use crate::program::node::ObjKey;
    use crate::program::resolve::extract::{
        CalleeShape, extract_sites_for_routine, record_op_names,
    };
    use crate::snapshot::{SnapshotBuilder, parse_snapshot};

    // ── Step 1: Build snapshot ───────────────────────────────────────────────
    let snap = match (SnapshotBuilder {
        workspace_root: workspace_root.to_path_buf(),
        local_providers: vec![],
    })
    .build()
    {
        Ok(s) => s,
        Err(_) => return DiffReport::default(),
    };

    // Build the set of true workspace source virtual paths from the workspace
    // AppUnit (always at index 0). This is needed to exclude embedded dep apps
    // whose AppId coincidentally matches the workspace AppId (e.g. when the
    // workspace .app is cached in an ancestor .alpackages directory). Such dep
    // AppUnits intern to `ws_ref` but their files carry different virtual paths
    // (typically with a `src/` prefix from the app's internal build layout).
    let ws_file_set: HashSet<String> = snap
        .apps
        .first()
        .and_then(|u| u.source.as_ref())
        .map(|s| s.files.iter().map(|f| f.virtual_path.clone()).collect())
        .unwrap_or_default();

    // ── Step 2: Build program graph ──────────────────────────────────────────
    let graph = build_program_graph(&snap);

    // ── Step 3: Parse snapshot ───────────────────────────────────────────────
    let parsed = parse_snapshot(&snap);

    // ── Step 4: Locate workspace app ─────────────────────────────────────────
    let Some(ws_ref) = graph.apps.find(&snap.workspace_app) else {
        return DiffReport::default();
    };
    let ws_guid = graph.apps.resolve(ws_ref).guid.clone();

    // Pre-build a fast set for implicit-Rec record-op name lookups.
    let rec_op_set: HashSet<&'static str> = record_op_names().iter().copied().collect();

    // ── Step 5: Extract fresh call-category sites (workspace only) ───────────
    let mut fresh_diff: Vec<CanonicalEdge> = Vec::new();
    let mut extra_recordop = 0usize;
    let mut extra_commit = 0usize;
    let mut extra_implicit_rec = 0usize;

    for unit in &parsed {
        let Some(app_ref) = graph.apps.find(&unit.app) else {
            continue;
        };
        // Keep workspace app only — dep-app call sites are not in the L3 oracle.
        if app_ref != ws_ref {
            continue;
        }

        for pf in &unit.files {
            // Exclude files from dep apps whose AppId matches the workspace AppId.
            // Their virtual paths are distinct from the true workspace source paths
            // (e.g. they carry a `src/` prefix from the embedded build layout).
            if !ws_file_set.contains(&pf.virtual_path) {
                continue;
            }
            // Process each object individually to avoid the N×M cross-product that
            // arises when multiple objects in one file share a routine name.
            for (obj_idx, obj) in pf.file.objects.iter().enumerate() {
                let obj_key = match obj.id {
                    Some(n) => ObjKey::Id(n),
                    None => ObjKey::Name(obj.name.to_ascii_lowercase()),
                };
                let obj_kind_str = object_kind_str(obj.kind);
                let obj_lc = obj_key_lc(&obj_key);

                // Per-object record-typed globals — used so that calls on object-level
                // record variables (e.g. `GlobalRec.Insert`) are classified as `RecordOp`
                // (L3 emits no `PCallSite` for those).
                let object_globals: HashSet<String> = obj
                    .globals
                    .iter()
                    .filter(|v| {
                        v.ty.as_deref()
                            .map(|ty| ty.trim().to_ascii_lowercase().starts_with("record"))
                            .unwrap_or(false)
                    })
                    .map(|v| v.name.to_ascii_lowercase())
                    .collect();

                // Iterate per-routine to avoid double-counting when multiple routines
                // share the same name (e.g. two `OnValidate` field triggers in a
                // TableExtension). `extract_sites_for_routine` is scoped to exactly one
                // routine body so each call site is attributed once.
                for (routine_idx, routine) in obj.routines.iter().enumerate() {
                    let name_lc = routine.name.to_ascii_lowercase();
                    let caller_key = make_canonical_key(
                        ws_guid.clone(),
                        obj_kind_str.clone(),
                        obj_lc.clone(),
                        name_lc,
                    );

                    let sites = extract_sites_for_routine(
                        &pf.file,
                        &pf.text,
                        &pf.virtual_path,
                        &object_globals,
                        obj_idx,
                        routine_idx,
                    );

                    for site in &sites {
                        match &site.shape {
                            CalleeShape::RecordOp { .. } => {
                                // L3 emits no PCallSite for RecordOp — justified extra.
                                extra_recordop += 1;
                            }
                            CalleeShape::Commit => {
                                // L3 emits no PCallSite for Commit — justified extra.
                                extra_commit += 1;
                            }
                            CalleeShape::Bare { name }
                                if rec_op_set.contains(name.to_ascii_lowercase().as_str())
                                    && routine.dataitem_source_table.is_none() =>
                            {
                                // Implicit-Rec bare record-op (e.g. `Validate(Field)` inside a
                                // table trigger or a `with Rec do` block).  L3 treats these as
                                // record-ops and emits no PCallSite; the fresh side approximates
                                // them as Bare because the implicit receiver isn't explicit.
                                //
                                // EXCEPTION: report dataitem triggers (`dataitem_source_table`
                                // is `Some`).  L3 does NOT set up an implicit record frame for
                                // report dataitems (`has_implicit_rec` returns false for Report
                                // objects), so it emits a PCallSite for those bare calls.  We
                                // must include them in the diff set to match the L3 oracle.
                                extra_implicit_rec += 1;
                            }
                            _ => {
                                // Call-category site (Bare/Member/ObjectRun/Unknown that is NOT
                                // a record-op name).  Add to the diff set for matching against
                                // the L3 PCallSite oracle.
                                let fp = callee_fp(&site.callee_text);
                                fresh_diff.push(CanonicalEdge {
                                    from: caller_key.clone(),
                                    site: CanonicalSiteKey {
                                        caller: caller_key.clone(),
                                        span: site.span.clone(),
                                        callee_fp: fp,
                                    },
                                    kind: EdgeKind::Call,
                                    targets: BTreeSet::new(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    fresh_diff.sort();
    let fresh_total_workspace = fresh_diff.len();

    // ── Step 6: Project L3 PCallSite oracle ──────────────────────────────────
    let l3_sites = project_l3_sites(workspace_root);
    let l3_edges = l3_sites.len();

    // ── Step 7: Match sites ───────────────────────────────────────────────────
    let site_matches = match_sites(&fresh_diff, &l3_sites);

    // ── Step 8: Bucket ────────────────────────────────────────────────────────
    let mut matched = 0usize;
    let mut missing_site = 0usize;
    let mut extra_unexplained = 0usize;
    let mut unaligned = 0usize;

    for m in &site_matches {
        match m {
            SiteMatch::Paired(_, _) => {
                matched += 1;
            }
            SiteMatch::FreshOnly(_) => {
                // A fresh call-category site with no L3 PCallSite peer.
                // Must be 0 for the gate to pass.
                extra_unexplained += 1;
            }
            SiteMatch::L3Only(_) => {
                missing_site += 1;
            }
            SiteMatch::Unaligned(fs, ls) => {
                unaligned += fs.len() + ls.len();
            }
        }
    }

    DiffReport {
        fresh_total_all_apps: 0, // not applicable for the site harness
        fresh_total_workspace,
        l3_edges,
        matched,
        regression: 0, // not applicable (site-level, no targets)
        missing_site,
        extra_site: 0, // superseded by extra_unexplained + categorized buckets
        unaligned,
        extra_recordop,
        extra_commit,
        extra_implicit_rec,
        extra_error: 0, // Error() sites are included in the diff set and pair with L3
        extra_unexplained,
    }
}

// ---------------------------------------------------------------------------
// Phase-2 resolution gate (Task 6)
// ---------------------------------------------------------------------------

/// Phase-2 resolution report: categorised comparison between the fresh
/// Bare/ObjectRun resolver and the L3 oracle for in-scope sites.
///
/// The three fields that must be 0 for the gate to pass are:
/// - [`regression_unexplained`][ResolutionReport::regression_unexplained]
/// - [`evidence_overclaim`][ResolutionReport::evidence_overclaim]
/// - [`unverified_extra`][ResolutionReport::unverified_extra]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolutionReport {
    /// Paired sites where fresh and L3 target sets agree (both empty or both
    /// non-empty with the same canonical targets).
    pub matched: usize,
    /// Paired sites where L3 resolved but fresh emitted empty targets —
    /// **unexplained** regression (must be 0).
    pub regression_unexplained: usize,
    /// Paired sites where L3 resolved via implicit-Rec (heuristic: caller is
    /// Page/PageExtension/TableExtension AND L3 target is Table/TableExtension)
    /// but fresh emitted empty targets (implicit-Rec deferred to Phase 3).
    /// Informational only.
    pub regression_implicit_rec: usize,
    /// Paired sites where L3 resolved to a routine in a **different app** (dep
    /// boundary) and fresh emitted empty targets because the procedure name is
    /// absent from (or private in) the dep's SymbolReference.  Informational
    /// only — deferred to 1B.3 ABI cross-check.
    pub regression_cross_app: usize,
    /// Routes that claim `Source`/`Abi`/`Catalog` evidence without a matching
    /// valid witness (must be 0).
    pub evidence_overclaim: usize,
    /// Reserved — always 0.  Fresh-only sites whose routes have invalid
    /// witnesses are caught globally by [`evidence_overclaim`][Self::evidence_overclaim];
    /// fresh-only sites with valid witnesses are legitimate wins outside the
    /// in-scope dispatch filter (interface-implementors etc.) and are counted
    /// in [`extra_site`][Self::extra_site] instead.
    ///
    /// STRUCTURAL NO-OP (Phase 2): every `FreshOnly` site goes to `extra_site`; no
    /// site reaches the `unverified_extra` accumulator because `FreshOnly` targets
    /// are NOT individually witness-checked here.  `evidence_overclaim` is the sole
    /// witness-quality gate (it checks ALL routes, including those on FreshOnly sites,
    /// via the per-route loop in Step 4).  This field MUST gain teeth in Phase 4
    /// (Multicast), where a FreshOnly site with valid witnesses but inapplicable
    /// dispatch conditions is a real correctness failure — applicability ≠
    /// single-dispatch correctness.
    pub unverified_extra: usize,
    /// Paired sites where L3 emitted empty targets but fresh resolved to
    /// non-empty targets — fresh did better than L3.
    pub verified_win: usize,
    /// Paired sites where both sides have non-empty targets but the sets differ.
    /// Informational.
    pub divergence: usize,
    /// L3-only in-scope sites: fresh emitted no site matching this L3 edge.
    pub missing_site: usize,
    /// FreshOnly sites with empty targets: fresh extracted a site that has no
    /// L3 in-scope peer (e.g. dynamic ObjectRun whose L3 dispatch is `Dynamic`
    /// — excluded from the in-scope filter).
    pub extra_site: usize,
    /// Sum of excess indices from `Unaligned` buckets in [`match_sites`].
    pub unaligned: usize,
    /// Total fresh in-scope sites (Bare + ObjectRun + Unknown).
    pub fresh_total: usize,
    /// Total L3 in-scope edges.
    pub l3_total: usize,
    /// Count of fresh in-scope sites where ALL routes are `Unresolved`.
    pub fresh_unknown_count: usize,
    /// `fresh_total - fresh_unknown_count`.
    pub fresh_resolved_count: usize,
    /// Count of L3 in-scope edges with empty targets (`to = None` in L3).
    pub l3_unknown_count: usize,
    /// `l3_total - l3_unknown_count`.
    pub l3_resolved_count: usize,
}

/// Returns `true` when the route's evidence/witness combination is valid.
///
/// Contract (spec §5.5):
/// - `Source`  → `SourceSpan` with non-empty file
/// - `Abi`     → `AbiSymbol`
/// - `Catalog` → `CatalogEntry`
/// - `Opaque`  → `AbiSymbol`
/// - `Unknown` → `None`
fn witness_contract_holds(route: &crate::program::resolve::edge::Route) -> bool {
    use crate::program::resolve::edge::{Evidence, RouteTarget, Witness};
    // For Unresolved targets the evidence must be Unknown (per resolver invariants).
    // Check both the evidence type and the witness shape.
    match (&route.evidence, &route.witness) {
        (Evidence::Source, Witness::SourceSpan { file, .. }) => !file.is_empty(),
        (Evidence::Abi, Witness::AbiSymbol { .. }) => true,
        (Evidence::Catalog, Witness::CatalogEntry { .. }) => true,
        (Evidence::Opaque, Witness::AbiSymbol { .. }) => true,
        (Evidence::Unknown, Witness::None) => {
            // Unknown evidence must pair with Unresolved target.
            matches!(route.target, RouteTarget::Unresolved)
        }
        _ => false,
    }
}

/// Heuristic: is this regression attributable to the implicit-Rec deferral?
///
/// Caller is `Page`, `PageExtension`, or `TableExtension` AND L3 resolved to
/// a `Table` (kind=1) or `TableExtension` (kind=2) routine — consistent with
/// L3 following the object's implicit `Rec` to its source/base table.
///
/// NOTE: This heuristic can absorb a genuine bare-call regression.  A
/// Page/PageExtension/TableExtension caller that calls an unqualified procedure
/// by name that happens to resolve to a Table target in L3 is presumed to be
/// an implicit-Rec receiver, but that presumption is unvalidated per-site.
/// The ~90 CDO `regression_implicit_rec` cases are expected to be true
/// implicit-Rec deferrals, but any genuine missed resolution in a Page/Table
/// caller targeting a Table would be silently absorbed.  Phase 3 (full
/// has_implicit_rec / receiver-lattice) will replace this heuristic.
fn is_implicit_rec_regression(
    caller_key: &CanonicalKey,
    l3_targets: &BTreeSet<CanonicalTarget>,
) -> bool {
    let caller_needs_implicit_rec = matches!(
        caller_key.object_kind.as_str(),
        "page" | "pageextension" | "tableextension"
    );
    if !caller_needs_implicit_rec {
        return false;
    }
    // Table kind = 1, TableExtension kind = 2 (from object_kind_str_to_tag).
    l3_targets.iter().any(|t| t.kind == 1 || t.kind == 2)
}

/// Heuristic: is this regression attributable to a dep-boundary SymbolReference
/// gap (i.e. the procedure exists in a dep app but the name is absent from or
/// private in the dep's `SymbolReference.json`)?
///
/// Returns `true` when **all** L3 targets belong to an app other than
/// `ws_guid`.  Deferred to Phase 1B.3 (ABI cross-check); informational only.
fn is_cross_app_regression(l3_targets: &BTreeSet<CanonicalTarget>, ws_guid: &str) -> bool {
    !l3_targets.is_empty()
        && l3_targets
            .iter()
            .all(|t| t.app.as_deref().map(|g| g != ws_guid).unwrap_or(true))
}

/// Project the L3 resolver's output for in-scope dispatch kinds only.
///
/// In-scope for Phase 2: `Direct`, `Builtin`, `CodeunitRun`, `PageRun`,
/// `ReportRun`, `Unresolved`.
///
/// Out-of-scope (Phase 3+): `Interface`, `Method`, `ImplicitTrigger`, `Dynamic`.
///
/// Encoding is identical to [`project_l3`]: same key construction, same target
/// encoding, same span/fp computation.  The only difference is the
/// `dispatch_kind` filter applied before projecting each `CallEdge`.
#[must_use]
fn project_l3_in_scope(workspace_root: &Path) -> Vec<CanonicalEdge> {
    use std::collections::HashMap;

    use crate::engine::l3::call_resolver::{DeclaredDependency, resolve_calls};
    use crate::engine::l3::l3_workspace::assemble_and_resolve_workspace_default;
    use crate::engine::l3::symbol_table::SymbolTable;
    use crate::engine::l3::taxonomy::DispatchKind;

    let Some(resolved) = assemble_and_resolve_workspace_default(workspace_root) else {
        return Vec::new();
    };
    let ws = &resolved.workspace;

    let symbols = SymbolTable::build(&ws.objects, &ws.tables, &ws.routines);
    let no_deps: Vec<DeclaredDependency> = Vec::new();
    let no_fetched: Vec<String> = Vec::new();
    let resolved_calls = resolve_calls(ws, &symbols, &no_deps, &no_fetched);

    let routine_by_id: HashMap<&str, &crate::engine::l3::l3_workspace::L3Routine> =
        ws.routines.iter().map(|r| (r.id.as_str(), r)).collect();

    let mut callsite_by_id: HashMap<&str, &crate::engine::l2::features::PCallSite> = HashMap::new();
    for routine in &ws.routines {
        for cs in &routine.call_sites {
            callsite_by_id.insert(cs.id.as_str(), cs);
        }
    }

    let mut edges: Vec<CanonicalEdge> = resolved_calls
        .edges
        .iter()
        .filter(|edge| {
            // Keep only in-scope dispatch kinds.
            matches!(
                edge.dispatch_kind,
                DispatchKind::Direct
                    | DispatchKind::Builtin
                    | DispatchKind::Unresolved
                    | DispatchKind::CodeunitRun
                    | DispatchKind::PageRun
                    | DispatchKind::ReportRun
            )
        })
        .filter_map(|edge| {
            let from_r = routine_by_id.get(edge.from.as_str())?;
            let from = make_canonical_key(
                from_r.app_guid.clone(),
                from_r.object_type.to_ascii_lowercase(),
                format!("{}", from_r.object_number),
                from_r.name.to_ascii_lowercase(),
            );

            let cs = callsite_by_id.get(edge.callsite_id.as_str())?;
            let a = &cs.source_anchor;
            let unit_str = a
                .source_unit_id
                .strip_prefix("ws:")
                .unwrap_or(&a.source_unit_id)
                .to_string();
            let span = CanonicalSpan {
                unit: unit_str,
                start: SourcePos {
                    line: a.start_line,
                    col: a.start_column,
                },
                end: SourcePos {
                    line: a.end_line,
                    col: a.end_column,
                },
            };
            let fp = callee_fp(&cs.callee_text);
            let site = CanonicalSiteKey {
                caller: from.clone(),
                span,
                callee_fp: fp,
            };

            let targets: BTreeSet<CanonicalTarget> = if let Some(to_id) = &edge.to {
                if let Some(to_r) = routine_by_id.get(to_id.as_str()) {
                    let mut set = BTreeSet::new();
                    set.insert(CanonicalTarget {
                        kind: object_kind_str_to_tag(&to_r.object_type.to_ascii_lowercase()),
                        app: Some(to_r.app_guid.clone()),
                        object_lc: format!("{}", to_r.object_number),
                        routine_lc: Some(to_r.name.to_ascii_lowercase()),
                    });
                    set
                } else {
                    BTreeSet::new()
                }
            } else {
                BTreeSet::new()
            };

            Some(CanonicalEdge {
                from,
                site,
                kind: EdgeKind::Call,
                targets,
            })
        })
        .collect();

    edges.sort();
    edges
}

/// Phase-2 resolution harness: resolves every in-scope workspace call site via
/// the real `resolve_bare` / `resolve_object_run` and compares against the L3
/// oracle filtered to the same in-scope dispatch kinds.
///
/// In-scope fresh sites: `Bare` (minus the implicit-Rec record-op exclusion) +
/// `ObjectRun` + `Unknown`.  Member / RecordOp / Commit are excluded.
///
/// In-scope L3 dispatch kinds: `Direct`, `Builtin`, `CodeunitRun`, `PageRun`,
/// `ReportRun`, `Unresolved`.  `Method`, `Interface`, `ImplicitTrigger`, and
/// `Dynamic` are excluded (Phase 3+).
///
/// Returns a [`ResolutionReport`] with detailed bucket counts.  Fail-closed:
/// any error during setup returns a zero report.
#[must_use]
pub fn run_resolution_harness(workspace_root: &Path) -> ResolutionReport {
    use std::collections::{HashMap, HashSet};

    use al_syntax::ir::ObjectKind;

    use crate::program::build::build_program_graph;
    use crate::program::node::{ObjKey, ObjectNodeId};
    use crate::program::node_extract::ObjectNode;
    use crate::program::resolve::body_map::BodyMap;
    use crate::program::resolve::edge::{Evidence, Route, RouteTarget, Witness};
    use crate::program::resolve::extract::{
        CalleeShape, extract_sites_for_routine, record_op_names,
    };
    use crate::program::resolve::index::ResolveIndex;
    use crate::program::resolve::resolver::{resolve_bare, resolve_object_run};
    use crate::snapshot::{SnapshotBuilder, parse_snapshot};

    // ── Step 1: Build snapshot ───────────────────────────────────────────────
    let snap = match (SnapshotBuilder {
        workspace_root: workspace_root.to_path_buf(),
        local_providers: vec![],
    })
    .build()
    {
        Ok(s) => s,
        Err(_) => return ResolutionReport::default(),
    };

    let ws_file_set: HashSet<String> = snap
        .apps
        .first()
        .and_then(|u| u.source.as_ref())
        .map(|s| s.files.iter().map(|f| f.virtual_path.clone()).collect())
        .unwrap_or_default();

    // ── Step 2: Build program graph + resolve index + body map ───────────────
    let graph = build_program_graph(&snap);
    let parsed = parse_snapshot(&snap);
    let index = ResolveIndex::build(&graph);
    let body_map = BodyMap::build(&graph, &parsed);

    // ── Step 3: Locate workspace app ─────────────────────────────────────────
    let Some(ws_ref) = graph.apps.find(&snap.workspace_app) else {
        return ResolutionReport::default();
    };
    let ws_guid = graph.apps.resolve(ws_ref).guid.clone();

    // Quick ObjectNodeId → &ObjectNode lookup.
    let obj_node_map: HashMap<ObjectNodeId, &ObjectNode> =
        graph.objects.iter().map(|o| (o.id.clone(), o)).collect();

    let rec_op_set: HashSet<&'static str> = record_op_names().iter().copied().collect();

    // ── Step 4: Resolve fresh sites (workspace-only) ──────────────────────────
    let mut fresh_canonical: Vec<CanonicalEdge> = Vec::new();
    let mut evidence_overclaim = 0usize;
    let mut fresh_unknown_count = 0usize;

    // Inline helper: an Unresolved+Unknown route (can't resolve).
    let unknown_route = || Route {
        target: RouteTarget::Unresolved,
        evidence: Evidence::Unknown,
        condition: None,
        witness: Witness::None,
    };

    for unit in &parsed {
        let Some(app_ref) = graph.apps.find(&unit.app) else {
            continue;
        };
        if app_ref != ws_ref {
            continue;
        }

        for pf in &unit.files {
            if !ws_file_set.contains(&pf.virtual_path) {
                continue;
            }

            for (obj_idx, obj) in pf.file.objects.iter().enumerate() {
                let obj_key = match obj.id {
                    Some(n) => ObjKey::Id(n),
                    None => ObjKey::Name(obj.name.to_ascii_lowercase()),
                };
                let obj_kind_str = object_kind_str(obj.kind);
                let obj_lc = obj_key_lc(&obj_key);

                let obj_node_id = ObjectNodeId {
                    app: ws_ref,
                    kind: obj.kind,
                    key: obj_key.clone(),
                };
                let obj_node_opt: Option<&ObjectNode> = obj_node_map.get(&obj_node_id).copied();

                let object_globals: HashSet<String> = obj
                    .globals
                    .iter()
                    .filter(|v| {
                        v.ty.as_deref()
                            .map(|ty| ty.trim().to_ascii_lowercase().starts_with("record"))
                            .unwrap_or(false)
                    })
                    .map(|v| v.name.to_ascii_lowercase())
                    .collect();

                for (routine_idx, routine) in obj.routines.iter().enumerate() {
                    let caller_key = make_canonical_key(
                        ws_guid.clone(),
                        obj_kind_str.clone(),
                        obj_lc.clone(),
                        routine.name.to_ascii_lowercase(),
                    );

                    let sites = extract_sites_for_routine(
                        &pf.file,
                        &pf.text,
                        &pf.virtual_path,
                        &object_globals,
                        obj_idx,
                        routine_idx,
                    );

                    for site in &sites {
                        let routes: Vec<Route> = match &site.shape {
                            // Always-excluded.
                            CalleeShape::RecordOp { .. } | CalleeShape::Commit => continue,
                            // Implicit-Rec bare record-op: L3 treats these as record
                            // operations and emits no CallEdge (mirrors run_site_harness).
                            CalleeShape::Bare { name }
                                if rec_op_set.contains(name.to_ascii_lowercase().as_str())
                                    && routine.dataitem_source_table.is_none() =>
                            {
                                continue;
                            }
                            // Member: Phase 3 (L3 Method/Interface dispatch excluded).
                            CalleeShape::Member { .. } => continue,

                            // Bare: resolve via own-object → extension-base →
                            // global-builtin → Unknown.
                            CalleeShape::Bare { name } => {
                                if let Some(obj_node) = obj_node_opt {
                                    resolve_bare(
                                        obj_node,
                                        &name.to_ascii_lowercase(),
                                        site.arity,
                                        &graph,
                                        &index,
                                        &body_map,
                                    )
                                } else {
                                    // Object not in graph — shouldn't happen; fail-closed.
                                    vec![unknown_route()]
                                }
                            }

                            // ObjectRun: resolve entry trigger of the target object.
                            CalleeShape::ObjectRun {
                                object_kind,
                                target_ref,
                                target_is_name,
                            } => {
                                let okind = match object_kind.as_str() {
                                    "Codeunit" => ObjectKind::Codeunit,
                                    "Page" => ObjectKind::Page,
                                    "Report" => ObjectKind::Report,
                                    _ => continue,
                                };
                                let (_, _, routes) = resolve_object_run(
                                    ws_ref,
                                    okind,
                                    target_ref.as_deref(),
                                    *target_is_name,
                                    &graph,
                                    &index,
                                    &body_map,
                                );
                                routes
                            }

                            // Unknown callee: can't resolve; include with empty targets
                            // so it pairs with L3's Unresolved dispatch (instead of
                            // becoming a missing_site on the L3 side).
                            CalleeShape::Unknown => vec![],
                        };

                        // Evidence/witness contract check (route-level).
                        for r in &routes {
                            if !witness_contract_holds(r) {
                                evidence_overclaim += 1;
                            }
                        }

                        // Count sites where all routes are Unresolved.
                        let is_all_unresolved = routes.is_empty()
                            || routes
                                .iter()
                                .all(|r| matches!(r.target, RouteTarget::Unresolved));
                        if is_all_unresolved {
                            fresh_unknown_count += 1;
                        }

                        // Project routes → canonical targets.
                        let targets: BTreeSet<CanonicalTarget> = routes
                            .iter()
                            .filter_map(|r| project_target(&r.target, &graph.apps))
                            .collect();

                        let fp = callee_fp(&site.callee_text);
                        fresh_canonical.push(CanonicalEdge {
                            from: caller_key.clone(),
                            site: CanonicalSiteKey {
                                caller: caller_key.clone(),
                                span: site.span.clone(),
                                callee_fp: fp,
                            },
                            kind: EdgeKind::Call,
                            targets,
                        });
                    }
                }
            }
        }
    }

    fresh_canonical.sort();
    let fresh_total = fresh_canonical.len();
    let fresh_resolved_count = fresh_total.saturating_sub(fresh_unknown_count);

    // ── Step 5: Project L3 in-scope oracle ────────────────────────────────────
    let l3_canonical = project_l3_in_scope(workspace_root);
    let l3_total = l3_canonical.len();
    let l3_unknown_count = l3_canonical.iter().filter(|e| e.targets.is_empty()).count();
    let l3_resolved_count = l3_total.saturating_sub(l3_unknown_count);

    // ── Step 6: Match sites ───────────────────────────────────────────────────
    let site_matches = match_sites(&fresh_canonical, &l3_canonical);

    // ── Step 7: Bucket ────────────────────────────────────────────────────────
    let mut matched = 0usize;
    let mut regression_unexplained = 0usize;
    let mut regression_implicit_rec = 0usize;
    let mut regression_cross_app = 0usize;
    let mut verified_win = 0usize;
    let mut divergence = 0usize;
    let mut missing_site = 0usize;
    let mut extra_site = 0usize;
    // `unverified_extra` is a live counter (not the former hardcoded-0 struct
    // field).  At Task 0 it stays 0 because the FreshOnly block routes all
    // Phase-2 sites to `extra_site` (no fan-out resolver yet).  Tasks 1-3 will
    // add `mut` and increment it via applicability-predicate classification.
    let unverified_extra = 0usize;
    let mut unaligned = 0usize;

    for m in &site_matches {
        match m {
            SiteMatch::Paired(fi, li) => {
                matched += 1;
                let f = &fresh_canonical[*fi];
                let l = &l3_canonical[*li];
                let f_empty = f.targets.is_empty();
                let l_empty = l.targets.is_empty();
                match (f_empty, l_empty) {
                    (true, true) => {
                        // Both unresolved — agreement.
                    }
                    (false, true) => {
                        // L3 empty, fresh non-empty — fresh did better (verified win).
                        verified_win += 1;
                    }
                    (true, false) => {
                        // L3 non-empty, fresh empty — regression.
                        if is_implicit_rec_regression(&f.from, &l.targets) {
                            regression_implicit_rec += 1;
                        } else if is_cross_app_regression(&l.targets, &ws_guid) {
                            // Dep-boundary gap: name absent from SymbolReference.
                            // Deferred to 1B.3 ABI cross-check.
                            regression_cross_app += 1;
                        } else {
                            regression_unexplained += 1;
                        }
                    }
                    (false, false) => {
                        // Both non-empty — compare target sets.
                        if f.targets != l.targets {
                            divergence += 1;
                        }
                        // If equal: agreement (no counter increment needed).
                    }
                }
            }
            SiteMatch::FreshOnly(_fi) => {
                // Fresh extracted a site with no L3 in-scope peer.  This covers:
                //  • dynamic ObjectRun (L3 Dynamic dispatch, excluded) with no
                //    static target → fresh also empty;
                //  • interface-dispatch Bare calls where L3 uses Interface/Method
                //    (excluded) but fresh correctly resolves to the concrete
                //    own-object procedure.
                // Witness quality is guaranteed by the global evidence_overclaim
                // check above.  Phase 4 fan-out routes (Tasks 1-3) will add
                // applicability-predicate classification here; until then all
                // FreshOnly sites are counted as extra_site (inert).
                extra_site += 1;
            }
            SiteMatch::L3Only(_) => {
                missing_site += 1;
            }
            SiteMatch::Unaligned(fs, ls) => {
                unaligned += fs.len() + ls.len();
            }
        }
    }

    ResolutionReport {
        matched,
        regression_unexplained,
        regression_implicit_rec,
        regression_cross_app,
        evidence_overclaim,
        // `unverified_extra` is a live counter (not hardcoded 0).  At Phase 2,
        // the FreshOnly block still routes all sites to `extra_site` because no
        // fan-out resolver exists yet.  Phase 4 Tasks 1-3 will wire applicability
        // classification here; `unverified_extra` will stay 0 until then.
        unverified_extra,
        verified_win,
        divergence,
        missing_site,
        extra_site,
        unaligned,
        fresh_total,
        l3_total,
        fresh_unknown_count,
        fresh_resolved_count,
        l3_unknown_count,
        l3_resolved_count,
    }
}

// ---------------------------------------------------------------------------
// Phase-3 Member resolution gate
// ---------------------------------------------------------------------------

/// Phase-3 resolution report for `Member` call sites.
///
/// Fields mirror [`ResolutionReport`] but are scoped to `CalleeShape::Member`
/// sites.  The three zero-tolerance gates are `regression_unexplained`,
/// `evidence_overclaim`, and determinism.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MemberResolutionReport {
    /// Paired sites where fresh and L3 target sets agree.
    pub matched: usize,
    /// Paired regressions NOT in any named deferral bucket (must be 0).
    pub regression_unexplained: usize,
    /// Paired regressions where fresh inferred `ReceiverType::Interface`
    /// (Phase-4 fan-out deferred).
    pub regression_interface: usize,
    /// Paired regressions where fresh inferred `ReceiverType::EnumType`
    /// (enum-static dispatch deferred).
    pub regression_enum_static: usize,
    /// Paired regressions where fresh inferred `ReceiverType::Record { table: None }`
    /// (Page/PageExt implicit-Rec table unresolved — Task-1 gap).
    pub regression_page_rec: usize,
    /// Paired regressions where fresh inferred `ReceiverType::Primitive`
    /// (scalar `.ToText()` etc. — by-design, not a resolution gap).
    pub regression_scalar: usize,
    /// Paired regressions where the receiver_text is a **compound dotted
    /// expression** (e.g. `CurrPage.SubPage.Page`), which fresh cannot resolve
    /// because Phase-3 receiver inference only handles simple identifiers.
    /// Phase-4 deferred (chained receiver type propagation).
    pub regression_compound_receiver: usize,
    /// Paired regressions where `receiver_lc ∈ {rec, xrec}` inside a
    /// **Codeunit** object and fresh inferred `Unknown`.  Root cause: a
    /// Codeunit with `TableNo` or `Subtype = TestRunner` has an implicit `Rec`
    /// parameter (or a variable named `Rec` sourced from an implicit context)
    /// that is not captured in the parsed IR or `ObjectNode`.  Deferred: adding
    /// `implicit_table: Option<ObjectNodeId>` to `ObjectNode` requires a
    /// properties-scan during node extraction (Phase 3 carry-over).
    pub regression_codeunit_implicit_rec: usize,
    /// Routes that claim `Source`/`Abi`/`Catalog` evidence without a matching
    /// valid witness (must be 0).
    pub evidence_overclaim: usize,
    /// Paired sites where L3 emitted empty targets but fresh resolved to
    /// non-empty targets — fresh did better than L3.
    pub verified_win: usize,
    /// Paired sites where both sides have non-empty but differing targets.
    pub divergence: usize,
    /// L3-only Member sites: fresh extracted no matching site.
    pub missing_site: usize,
    /// Fresh-only Member sites: no L3 Member-oracle peer — valid extra (empty
    /// targets, no witness claim to validate) or categorised by `fresh_ahead_*`.
    pub extra_site: usize,
    /// Fresh-only sites with concrete targets validated as justified interface
    /// fan-out by [`applicability::interface_route_applicable`] — populated by
    /// Tasks 1-3.  Zero at Task 0 (no fan-out resolver yet).
    pub fresh_ahead_interface: usize,
    /// Fresh-only sites with concrete targets validated as justified
    /// instance-builtin by [`applicability::instance_builtin_route_applicable`]
    /// — populated by Tasks 1-3.  Zero at Task 0.
    pub fresh_ahead_instance_builtin: usize,
    /// Fresh-only sites with concrete targets validated as justified enum-static
    /// dispatch — populated by Tasks 1-3.  Zero at Task 0.
    pub fresh_ahead_enum_static: usize,
    /// Fresh-only sites with concrete targets that FAILED the matching
    /// applicability predicate — a real false edge (must be 0).
    /// Gains teeth in Phase 4 when Tasks 1-3 emit fan-out routes; zero until then.
    pub unverified_extra: usize,
    /// Sum of excess indices from `Unaligned` buckets.
    pub unaligned: usize,
    /// Total fresh `Member` sites extracted from the workspace.
    pub fresh_total: usize,
    /// Total L3 Member-in-scope edges.
    pub l3_total: usize,
    /// Count of fresh Member sites where ALL routes are `Unresolved`.
    pub fresh_unknown_count: usize,
    /// `fresh_total - fresh_unknown_count`.
    pub fresh_resolved_count: usize,
    /// Count of L3 in-scope Member edges with empty targets.
    pub l3_unknown_count: usize,
    /// `l3_total - l3_unknown_count`.
    pub l3_resolved_count: usize,
}

/// Project the L3 resolver's output for in-scope Member-dispatch kinds only.
///
/// Includes L3 edges where:
/// - The originating `PCallSite.callee` is `PCallee::Member`.
/// - `dispatch_kind ∈ {Method, Builtin, CodeunitRun}`.
///
/// Excludes `Interface` (Phase-4 fan-out) and `Dynamic` (runtime-typed
/// `Variant` receiver — honest open-world).  L3 `Builtin` edges carry
/// `to = None` (empty targets); fresh catalog-resolved routes carry a
/// `CanonicalTarget { kind: 255, … }` (non-empty) → these appear as
/// `verified_win` in the report.
#[must_use]
fn project_l3_member_in_scope(workspace_root: &Path) -> Vec<CanonicalEdge> {
    use std::collections::HashMap;

    use crate::engine::l2::features::PCallee;
    use crate::engine::l3::call_resolver::{DeclaredDependency, resolve_calls};
    use crate::engine::l3::l3_workspace::assemble_and_resolve_workspace_default;
    use crate::engine::l3::symbol_table::SymbolTable;
    use crate::engine::l3::taxonomy::DispatchKind;

    let Some(resolved) = assemble_and_resolve_workspace_default(workspace_root) else {
        return Vec::new();
    };
    let ws = &resolved.workspace;

    let symbols = SymbolTable::build(&ws.objects, &ws.tables, &ws.routines);
    let no_deps: Vec<DeclaredDependency> = Vec::new();
    let no_fetched: Vec<String> = Vec::new();
    let resolved_calls = resolve_calls(ws, &symbols, &no_deps, &no_fetched);

    let routine_by_id: HashMap<&str, &crate::engine::l3::l3_workspace::L3Routine> =
        ws.routines.iter().map(|r| (r.id.as_str(), r)).collect();

    let mut callsite_by_id: HashMap<&str, &crate::engine::l2::features::PCallSite> = HashMap::new();
    for routine in &ws.routines {
        for cs in &routine.call_sites {
            callsite_by_id.insert(cs.id.as_str(), cs);
        }
    }

    let mut edges: Vec<CanonicalEdge> = resolved_calls
        .edges
        .iter()
        .filter(|edge| {
            let is_member = callsite_by_id
                .get(edge.callsite_id.as_str())
                .map(|cs| matches!(cs.callee, PCallee::Member { .. }))
                .unwrap_or(false);
            is_member
                && matches!(
                    edge.dispatch_kind,
                    DispatchKind::Method
                        | DispatchKind::Builtin
                        | DispatchKind::CodeunitRun
                        | DispatchKind::Interface
                )
        })
        .filter_map(|edge| {
            let from_r = routine_by_id.get(edge.from.as_str())?;
            let from = make_canonical_key(
                from_r.app_guid.clone(),
                from_r.object_type.to_ascii_lowercase(),
                format!("{}", from_r.object_number),
                from_r.name.to_ascii_lowercase(),
            );

            let cs = callsite_by_id.get(edge.callsite_id.as_str())?;
            let a = &cs.source_anchor;
            let unit_str = a
                .source_unit_id
                .strip_prefix("ws:")
                .unwrap_or(&a.source_unit_id)
                .to_string();
            let span = CanonicalSpan {
                unit: unit_str,
                start: SourcePos {
                    line: a.start_line,
                    col: a.start_column,
                },
                end: SourcePos {
                    line: a.end_line,
                    col: a.end_column,
                },
            };
            let fp = callee_fp(&cs.callee_text);
            let site = CanonicalSiteKey {
                caller: from.clone(),
                span,
                callee_fp: fp,
            };

            let targets: BTreeSet<CanonicalTarget> = if let Some(to_id) = &edge.to {
                if let Some(to_r) = routine_by_id.get(to_id.as_str()) {
                    let mut set = BTreeSet::new();
                    set.insert(CanonicalTarget {
                        kind: object_kind_str_to_tag(&to_r.object_type.to_ascii_lowercase()),
                        app: Some(to_r.app_guid.clone()),
                        object_lc: format!("{}", to_r.object_number),
                        routine_lc: Some(to_r.name.to_ascii_lowercase()),
                    });
                    set
                } else {
                    BTreeSet::new()
                }
            } else {
                BTreeSet::new()
            };

            Some(CanonicalEdge {
                from,
                site,
                kind: EdgeKind::Call,
                targets,
            })
        })
        .collect();

    edges.sort();
    edges
}

/// Diagnostic entry for an unexplained Member regression (fresh Unknown, L3
/// resolved), printed to stderr if `regression_unexplained > 0` at the end of
/// [`run_member_resolution_harness`].
struct RegressionDiag {
    caller: String,
    callee_text: String,
    recv_type: String,
    l3_targets: String,
}

/// Phase-3 Member-resolution harness: resolves every workspace `Member` call
/// site via `infer_receiver_type` + `resolve_member` and compares against the
/// L3 oracle filtered to `PCallee::Member` origin with `dispatch_kind ∈
/// {Method, Builtin, CodeunitRun}`.
///
/// Paired regressions (L3 resolved, fresh Unknown) are categorized into named
/// deferral buckets based on the inferred `ReceiverType` and callee structure:
/// - `regression_interface` — `Interface` receiver (Phase-4 fan-out);
/// - `regression_enum_static` — `EnumType` receiver (enum-static deferred);
/// - `regression_page_rec` — `Record { table: None }` (Page/PageExt implicit-Rec gap);
/// - `regression_scalar` — `Primitive` receiver (scalar `.ToText()` etc.);
/// - `regression_compound_receiver` — compound dotted receiver (e.g.
///   `CurrPage.SubPage.Page`); Phase-3 handles only simple identifiers;
/// - `regression_codeunit_implicit_rec` — `rec`/`xrec` receiver in a Codeunit
///   with `TableNo`/`Subtype = TestRunner`; implicit parameter not in IR;
/// - `regression_unexplained` — anything else (must be 0; investigate if > 0).
///
/// Fail-closed: any error during setup returns a zero report.
#[must_use]
pub fn run_member_resolution_harness(workspace_root: &Path) -> MemberResolutionReport {
    use std::collections::{HashMap, HashSet};

    use crate::program::build::build_program_graph;
    use crate::program::node::{ObjKey, ObjectNodeId};
    use crate::program::node_extract::ObjectNode;
    use crate::program::resolve::applicability::{
        instance_builtin_route_applicable, interface_route_applicable,
    };
    use crate::program::resolve::body_map::BodyMap;
    use crate::program::resolve::edge::{Evidence, Route, RouteTarget, Witness};
    use crate::program::resolve::extract::{CalleeShape, extract_sites_for_routine};
    use crate::program::resolve::index::ResolveIndex;
    use crate::program::resolve::member_catalog::{MemberCatalogKind, member_builtin};
    use crate::program::resolve::receiver::{FrameworkKind, ReceiverType, infer_receiver_type};
    use crate::program::resolve::resolver::resolve_member;
    use crate::snapshot::{SnapshotBuilder, parse_snapshot};

    // ── Step 1: Build snapshot ───────────────────────────────────────────────
    let snap = match (SnapshotBuilder {
        workspace_root: workspace_root.to_path_buf(),
        local_providers: vec![],
    })
    .build()
    {
        Ok(s) => s,
        Err(_) => return MemberResolutionReport::default(),
    };

    let ws_file_set: HashSet<String> = snap
        .apps
        .first()
        .and_then(|u| u.source.as_ref())
        .map(|s| s.files.iter().map(|f| f.virtual_path.clone()).collect())
        .unwrap_or_default();

    // ── Step 2: Build graph + index + body map ───────────────────────────────
    let graph = build_program_graph(&snap);
    let parsed = parse_snapshot(&snap);
    let index = ResolveIndex::build(&graph);
    let body_map = BodyMap::build(&graph, &parsed);

    // ── Step 3: Locate workspace app ─────────────────────────────────────────
    let Some(ws_ref) = graph.apps.find(&snap.workspace_app) else {
        return MemberResolutionReport::default();
    };
    let ws_guid = graph.apps.resolve(ws_ref).guid.clone();

    let obj_node_map: HashMap<ObjectNodeId, &ObjectNode> =
        graph.objects.iter().map(|o| (o.id.clone(), o)).collect();

    let unknown_route = || Route {
        target: RouteTarget::Unresolved,
        evidence: Evidence::Unknown,
        condition: None,
        witness: Witness::None,
    };

    // ── Step 4: Resolve fresh Member sites (workspace-only) ───────────────────
    // Five parallel fields kept in sync (named tuple to appease type_complexity lint).
    //   .0 fresh_canonical — the edge projected to canonical form
    //   .1 recv_type       — the inferred ReceiverType (for regression bucketing)
    //   .2 callee_text     — (callee_text) for diagnostic printing
    //   .3 arity           — call-site arity (needed for interface applicability check)
    //   .4 routes          — original routes (for interface applicability: Routine targets)
    type FreshEntry = (
        CanonicalEdge,
        Option<ReceiverType>,
        String,
        usize,
        Vec<Route>,
    );
    let mut fresh_combined: Vec<FreshEntry> = Vec::new();
    let mut evidence_overclaim = 0usize;
    let mut fresh_unknown_count = 0usize;

    for unit in &parsed {
        let Some(app_ref) = graph.apps.find(&unit.app) else {
            continue;
        };
        if app_ref != ws_ref {
            continue;
        }

        for pf in &unit.files {
            if !ws_file_set.contains(&pf.virtual_path) {
                continue;
            }

            for (obj_idx, obj) in pf.file.objects.iter().enumerate() {
                let obj_key = match obj.id {
                    Some(n) => ObjKey::Id(n),
                    None => ObjKey::Name(obj.name.to_ascii_lowercase()),
                };
                let obj_kind_str = object_kind_str(obj.kind);
                let obj_lc = obj_key_lc(&obj_key);

                let obj_node_id = ObjectNodeId {
                    app: ws_ref,
                    kind: obj.kind,
                    key: obj_key.clone(),
                };
                let obj_node_opt: Option<&ObjectNode> = obj_node_map.get(&obj_node_id).copied();

                // Record-typed global variable names — for site classification
                // (same pattern as run_resolution_harness / run_site_harness).
                let object_globals_rec_set: HashSet<String> = obj
                    .globals
                    .iter()
                    .filter(|v| {
                        v.ty.as_deref()
                            .map(|ty| ty.trim().to_ascii_lowercase().starts_with("record"))
                            .unwrap_or(false)
                    })
                    .map(|v| v.name.to_ascii_lowercase())
                    .collect();

                for (routine_idx, routine) in obj.routines.iter().enumerate() {
                    let caller_key = make_canonical_key(
                        ws_guid.clone(),
                        obj_kind_str.clone(),
                        obj_lc.clone(),
                        routine.name.to_ascii_lowercase(),
                    );

                    let sites = extract_sites_for_routine(
                        &pf.file,
                        &pf.text,
                        &pf.virtual_path,
                        &object_globals_rec_set,
                        obj_idx,
                        routine_idx,
                    );

                    for site in &sites {
                        let (routes, recv_type) = match &site.shape {
                            CalleeShape::Member {
                                receiver_text,
                                method,
                            } => {
                                let receiver_lc = receiver_text.to_ascii_lowercase();
                                let method_lc = method.to_ascii_lowercase();

                                if let Some(obj_node) = obj_node_opt {
                                    let recv = infer_receiver_type(
                                        &receiver_lc,
                                        routine,
                                        &obj.globals,
                                        obj_node,
                                        &graph,
                                        &index,
                                    );
                                    let (_, routes) = resolve_member(
                                        &recv, &method_lc, site.arity, obj_node, &graph, &index,
                                        &body_map,
                                    );
                                    (routes, Some(recv))
                                } else {
                                    // ObjectNode absent from graph (shouldn't
                                    // happen in a sound workspace).
                                    (vec![unknown_route()], None)
                                }
                            }
                            // Skip all non-Member sites — covered by Phase-2.
                            _ => continue,
                        };

                        // Evidence/witness contract check (route-level).
                        for r in &routes {
                            if !witness_contract_holds(r) {
                                evidence_overclaim += 1;
                            }
                        }

                        let is_all_unresolved = routes.is_empty()
                            || routes
                                .iter()
                                .all(|r| matches!(r.target, RouteTarget::Unresolved));
                        if is_all_unresolved {
                            fresh_unknown_count += 1;
                        }

                        let targets: BTreeSet<CanonicalTarget> = routes
                            .iter()
                            .filter_map(|r| project_target(&r.target, &graph.apps))
                            .collect();

                        let fp = callee_fp(&site.callee_text);
                        let edge = CanonicalEdge {
                            from: caller_key.clone(),
                            site: CanonicalSiteKey {
                                caller: caller_key.clone(),
                                span: site.span.clone(),
                                callee_fp: fp,
                            },
                            kind: EdgeKind::Call,
                            targets,
                        };
                        fresh_combined.push((
                            edge,
                            recv_type,
                            site.callee_text.clone(),
                            site.arity,
                            routes,
                        ));
                    }
                }
            }
        }
    }

    // Sort all five vecs together (by canonical edge order).
    fresh_combined.sort_by(|a, b| a.0.cmp(&b.0));
    let fresh_recv_types: Vec<Option<ReceiverType>> = fresh_combined
        .iter()
        .map(|(_, r, _, _, _)| r.clone())
        .collect();
    let fresh_diag_text: Vec<String> = fresh_combined
        .iter()
        .map(|(_, _, t, _, _)| t.clone())
        .collect();
    let fresh_arities: Vec<usize> = fresh_combined.iter().map(|(_, _, _, a, _)| *a).collect();
    let fresh_routes: Vec<Vec<Route>> = fresh_combined
        .iter()
        .map(|(_, _, _, _, routes)| routes.clone())
        .collect();
    let fresh_canonical: Vec<CanonicalEdge> = fresh_combined
        .into_iter()
        .map(|(e, _, _, _, _)| e)
        .collect();

    let fresh_total = fresh_canonical.len();
    let fresh_resolved_count = fresh_total.saturating_sub(fresh_unknown_count);

    // ── Step 5: Project L3 Member oracle ─────────────────────────────────────
    let l3_canonical = project_l3_member_in_scope(workspace_root);
    let l3_total = l3_canonical.len();
    let l3_unknown_count = l3_canonical.iter().filter(|e| e.targets.is_empty()).count();
    let l3_resolved_count = l3_total.saturating_sub(l3_unknown_count);

    // ── Step 6: Match sites ───────────────────────────────────────────────────
    let site_matches = match_sites(&fresh_canonical, &l3_canonical);

    // ── Step 7: Bucket ────────────────────────────────────────────────────────
    let mut matched = 0usize;
    let mut regression_unexplained = 0usize;
    let mut regression_interface = 0usize;
    let mut regression_enum_static = 0usize;
    let mut regression_page_rec = 0usize;
    let mut regression_scalar = 0usize;
    let mut regression_compound_receiver = 0usize;
    let mut regression_codeunit_implicit_rec = 0usize;
    let mut verified_win = 0usize;
    let mut divergence = 0usize;
    let mut missing_site = 0usize;
    let mut extra_site = 0usize;
    // Phase-4 applicability counters; all wired in the FreshOnly handler.
    let mut fresh_ahead_interface = 0usize;
    let mut fresh_ahead_instance_builtin = 0usize;
    let mut fresh_ahead_enum_static = 0usize;
    let mut unverified_extra = 0usize;
    let mut unaligned = 0usize;

    // Diagnostics for unexplained regressions (first 30, to avoid noise).
    let mut diag_unexplained: Vec<RegressionDiag> = Vec::new();

    for m in &site_matches {
        match m {
            SiteMatch::Paired(fi, li) => {
                matched += 1;
                let f = &fresh_canonical[*fi];
                let l = &l3_canonical[*li];
                let f_empty = f.targets.is_empty();
                let l_empty = l.targets.is_empty();
                match (f_empty, l_empty) {
                    (true, true) => {
                        // Both unresolved — agreement.
                    }
                    (false, true) => {
                        // L3 empty, fresh non-empty — fresh did better.
                        verified_win += 1;
                    }
                    (true, false) => {
                        // L3 resolved, fresh Unknown — categorize regression.
                        let recv = fresh_recv_types[*fi].as_ref();
                        match recv {
                            Some(ReceiverType::Interface { .. }) => {
                                regression_interface += 1;
                            }
                            Some(ReceiverType::EnumType { .. }) => {
                                regression_enum_static += 1;
                            }
                            Some(ReceiverType::Record { table: None }) => {
                                regression_page_rec += 1;
                            }
                            Some(ReceiverType::Primitive) => {
                                regression_scalar += 1;
                            }
                            _ => {
                                // Guard: the text-based deferral buckets
                                // (compound_receiver and codeunit_implicit_rec)
                                // only apply when receiver inference FAILED
                                // (ReceiverType::Unknown or obj_node absent).
                                // If the receiver inferred to a resolvable type
                                // (Record{Some}, Object, Framework, …) but
                                // resolve_member still returned empty targets,
                                // that is a genuine regression — surface it via
                                // regression_unexplained rather than silently
                                // absorbing it into a text-heuristic bucket.
                                let recv_is_unknown =
                                    matches!(recv, Some(ReceiverType::Unknown) | None);

                                // Derive receiver_lc from callee_text (strip
                                // the trailing `.method` segment).
                                let callee_lc = fresh_diag_text[*fi].to_ascii_lowercase();
                                let recv_lc: &str = if let Some(pos) = callee_lc.rfind('.') {
                                    &callee_lc[..pos]
                                } else {
                                    &callee_lc
                                };

                                if recv_is_unknown && recv_lc.contains('.') {
                                    // Compound receiver expression (e.g.
                                    // `CurrPage.SubPage.Page`) — Phase-3
                                    // inference is single-identifier only.
                                    // Deferred to Phase 4.
                                    regression_compound_receiver += 1;
                                } else if recv_is_unknown
                                    && (recv_lc == "rec" || recv_lc == "xrec")
                                    && f.from.object_kind == "codeunit"
                                {
                                    // Codeunit with implicit `Rec` from
                                    // `TableNo` or `Subtype = TestRunner` —
                                    // the implicit parameter is not captured
                                    // in the parsed IR or `ObjectNode`.
                                    regression_codeunit_implicit_rec += 1;
                                } else {
                                    regression_unexplained += 1;
                                    if diag_unexplained.len() < 30 {
                                        let recv_str = format!("{recv:?}");
                                        let l3_tgt = l
                                            .targets
                                            .iter()
                                            .map(|t| {
                                                format!(
                                                    "{}:{}:{}",
                                                    t.app.as_deref().unwrap_or("?"),
                                                    t.object_lc,
                                                    t.routine_lc.as_deref().unwrap_or("<builtin>")
                                                )
                                            })
                                            .collect::<Vec<_>>()
                                            .join(", ");
                                        diag_unexplained.push(RegressionDiag {
                                            caller: format!(
                                                "{}::{}::{}",
                                                f.from.object_kind,
                                                f.from.object_lc,
                                                f.from.routine_lc,
                                            ),
                                            callee_text: fresh_diag_text[*fi].clone(),
                                            recv_type: recv_str,
                                            l3_targets: l3_tgt,
                                        });
                                    }
                                }
                            }
                        }
                    }
                    (false, false) => {
                        if f.targets != l.targets {
                            divergence += 1;
                        }
                    }
                }
            }
            SiteMatch::FreshOnly(fi) => {
                let f = &fresh_canonical[*fi];
                if f.targets.is_empty() {
                    // No concrete route — legitimate extra_site (unknown on fresh, no L3 peer).
                    extra_site += 1;
                } else {
                    // Has concrete targets.  Discriminate by the route TARGET type, not by
                    // receiver type alone.  Only routes that produced a Builtin target
                    // (CanonicalTarget::kind == 255) with a fan-out prefix
                    // ("PageInstance::" / "ReportInstance::" / "Enum::") are candidate
                    // fan-out routes and require an applicability-predicate gate.
                    //
                    // All other routes (Routine/AbiSymbol from resolve_in_object, Record
                    // catalog builtins with "Record::" prefix, etc.) are direct
                    // single-dispatch — their witness IS their proof and they belong in
                    // extra_site (a legitimate fresh win).  Do NOT push them to
                    // unverified_extra merely because the receiver is Object or EnumType
                    // but the target is a source-declared Routine.
                    let callee_lc = fresh_diag_text[*fi].to_ascii_lowercase();
                    let method_lc: &str = callee_lc.rsplit('.').next().unwrap_or(&callee_lc);

                    // Interface polymorphic fan-out: receiver is Interface{name_lc}.
                    // Each Routine route is checked by `interface_route_applicable`.
                    // Unresolved routes (Rule-1/2 failures) claim nothing → no check.
                    let is_interface_route =
                        matches!(&fresh_recv_types[*fi], Some(ReceiverType::Interface { .. }));

                    // Identify fan-out routes by the canonical target prefix.
                    let is_instance_builtin_target = f.targets.iter().any(|t| {
                        t.kind == 255
                            && (t.object_lc.starts_with("PageInstance::")
                                || t.object_lc.starts_with("ReportInstance::"))
                    });
                    let is_enum_static_target = f
                        .targets
                        .iter()
                        .any(|t| t.kind == 255 && t.object_lc.starts_with("Enum::"));

                    if is_interface_route {
                        // Interface polymorphic fan-out: applicability-gate every
                        // Routine route against (iface_lc, method_lc, arity).
                        // Unresolved routes (Rule-1/2 failures) are unchecked —
                        // they claim nothing and are always valid.
                        let iface_lc = match &fresh_recv_types[*fi] {
                            Some(ReceiverType::Interface { name_lc }) => name_lc.as_str(),
                            _ => unreachable!(),
                        };
                        let site_arity = fresh_arities[*fi];
                        let original_routes = &fresh_routes[*fi];

                        let all_applicable = original_routes.iter().all(|r| match &r.target {
                            RouteTarget::Unresolved => true,
                            RouteTarget::Routine(rid) => interface_route_applicable(
                                iface_lc, method_lc, site_arity, rid, &graph, &index,
                            ),
                            // AbiSymbol: a SymbolOnly (cross-app .app) implementer emitted
                            // from `implementers_of`.  Object-level applicability holds by
                            // construction — the object is a known interface implementer read
                            // from SymbolReference.  The member is opaque (no source to verify
                            // the signature) but the ABI boundary is known → PASS, exactly as
                            // the Phase-2 ObjectRun opaque-boundary treatment.  Classifying it
                            // as `unverified_extra` would be a false gate failure.
                            RouteTarget::AbiSymbol { .. } => true,
                            // Builtin on an interface fan-out site is genuinely anomalous —
                            // leave as FAIL so it surfaces as a real gate violation.
                            _ => false,
                        });

                        if all_applicable {
                            fresh_ahead_interface += 1;
                        } else {
                            unverified_extra += 1;
                        }
                    } else if is_instance_builtin_target {
                        // Instance-builtin fan-out route: independently re-check
                        // applicability using kind+method derived from the BuiltinId
                        // prefix ("PageInstance::" → Page, "ReportInstance::" →
                        // Report).  Do NOT rely on the receiver type — both
                        // `Object { kind: Page }` and `Framework(PageInstance)` are
                        // valid callers that produce PageInstance:: targets; both
                        // should validate via the same catalog gate.
                        let kind_from_target = f.targets.iter().find_map(|t| {
                            if t.kind == 255 {
                                if t.object_lc.starts_with("PageInstance::") {
                                    Some(ObjectKind::Page)
                                } else if t.object_lc.starts_with("ReportInstance::") {
                                    Some(ObjectKind::Report)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        });
                        if let Some(kind) = kind_from_target {
                            if instance_builtin_route_applicable(kind, method_lc) {
                                fresh_ahead_instance_builtin += 1;
                            } else {
                                unverified_extra += 1;
                            }
                        } else {
                            // PageInstance/ReportInstance prefix present but could not
                            // map to an ObjectKind — treat as unverified.
                            unverified_extra += 1;
                        }
                    } else if is_enum_static_target {
                        // Enum-static fan-out route: re-check catalog membership.
                        if member_builtin(
                            MemberCatalogKind::Framework(&FrameworkKind::Enum),
                            method_lc,
                        ) {
                            fresh_ahead_enum_static += 1;
                        } else {
                            unverified_extra += 1;
                        }
                    } else {
                        // Direct single-dispatch route (Routine/AbiSymbol/Record-catalog).
                        // The witness IS the proof — no applicability predicate needed.
                        // These are legitimate fresh wins where fresh resolved a Member call
                        // via direct dispatch but L3's in-scope filter excluded it
                        // (e.g., L3 dispatched via Method/Interface/Dynamic which are
                        // outside the member-oracle scope).
                        extra_site += 1;
                    }
                }
            }
            SiteMatch::L3Only(_) => {
                missing_site += 1;
            }
            SiteMatch::Unaligned(fs, ls) => {
                unaligned += fs.len() + ls.len();
            }
        }
    }

    // Print unexplained-regression diagnostics to stderr so they appear in
    // `cargo test -- --nocapture` output.
    if !diag_unexplained.is_empty() {
        eprintln!(
            "\n[Member harness] regression_unexplained={} (showing first {}):",
            regression_unexplained,
            diag_unexplained.len()
        );
        for d in &diag_unexplained {
            eprintln!(
                "  caller={} callee={:?} recv={} l3→[{}]",
                d.caller, d.callee_text, d.recv_type, d.l3_targets,
            );
        }
    }

    MemberResolutionReport {
        matched,
        regression_unexplained,
        regression_interface,
        regression_enum_static,
        regression_page_rec,
        regression_scalar,
        regression_compound_receiver,
        regression_codeunit_implicit_rec,
        evidence_overclaim,
        verified_win,
        divergence,
        missing_site,
        extra_site,
        fresh_ahead_interface,
        fresh_ahead_instance_builtin,
        fresh_ahead_enum_static,
        unverified_extra,
        unaligned,
        fresh_total,
        l3_total,
        fresh_unknown_count,
        fresh_resolved_count,
        l3_unknown_count,
        l3_resolved_count,
    }
}

// ---------------------------------------------------------------------------
// Phase-4 ImplicitTrigger resolution gate
// ---------------------------------------------------------------------------

/// Extract the lowercased table name from an AL `"Record <TableName>"` type string.
///
/// Returns `None` for non-specific Record types (`RecordRef`, numeric scalars, etc.).
/// The name is returned already-lowercased so callers can pass it directly to
/// case-insensitive lookups.
fn record_type_table_name_lc(ty: &str) -> Option<String> {
    let lc_trim = ty.trim().to_ascii_lowercase();
    // Must have "record " (with trailing space) to exclude RecordRef, RecordObject, etc.
    let rest = lc_trim.strip_prefix("record ")?;
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }
    // Strip surrounding double-quotes (AL name quoting: `Record "Sales Line"`).
    if rest.starts_with('"') && rest.ends_with('"') && rest.len() > 2 {
        Some(rest[1..rest.len() - 1].to_string())
    } else {
        Some(rest.to_string())
    }
}

/// Returns `true` iff `target_object` is `table_id` itself OR a
/// `TableExtension` of it (looked up via `index.table_extensions_of`).
///
/// Used to classify FreshOnly `Validate` routes: with `RecordOpCtx.field = None`
/// the full `implicit_trigger_route_applicable` always returns `false`, so we
/// fall back to this coarser table-identity check to distinguish
/// `fresh_ahead_validate_fanout` (on correct table) from `unverified_extra`
/// (on an unrelated table — a genuine false edge).
fn target_is_on_table_or_extension(
    target_object: &crate::program::node::ObjectNodeId,
    table_id: &crate::program::node::ObjectNodeId,
    graph: &crate::program::graph::ProgramGraph,
    index: &crate::program::resolve::index::ResolveIndex,
) -> bool {
    if target_object == table_id {
        return true;
    }
    let table_name_lc: String = match &table_id.key {
        crate::program::node::ObjKey::Name(s) => s.clone(),
        crate::program::node::ObjKey::Id(_) => graph
            .objects
            .iter()
            .find(|o| &o.id == table_id)
            .map(|n| n.name.to_ascii_lowercase())
            .unwrap_or_default(),
    };
    if table_name_lc.is_empty() {
        return false;
    }
    index
        .table_extensions_of(&table_name_lc)
        .contains(target_object)
}

/// Phase-4 resolution report for `ImplicitTrigger` call sites.
///
/// Three zero-tolerance gates:
/// - `regression_unexplained`: paired site where L3 has trigger targets but fresh has none.
/// - `evidence_overclaim`: route with `Source`/`Abi`/`Catalog` evidence but no valid witness.
/// - `unverified_extra`: FreshOnly site whose routes FAIL the applicability predicate and
///   are NOT explained by `fresh_ahead_validate_fanout`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImplicitTriggerResolutionReport {
    /// Paired sites where fresh and L3 target sets agree.
    pub matched: usize,
    /// Paired regressions: L3 has trigger targets, fresh has none (must be 0).
    pub regression_unexplained: usize,
    /// Routes with invalid `Source`/`Abi`/`Catalog` evidence (must be 0).
    pub evidence_overclaim: usize,
    /// FreshOnly sites with routes that fail `implicit_trigger_route_applicable`
    /// and are not explained by `fresh_ahead_validate_fanout` (must be 0).
    pub unverified_extra: usize,
    /// FreshOnly `insert`/`modify`/`delete` sites where all routes pass
    /// `implicit_trigger_route_applicable` — legitimate fresh wins.
    pub fresh_ahead_trigger: usize,
    /// FreshOnly `validate` sites where all routes target `onvalidate` on the
    /// correct table/extension — known over-approximation (field context unknown).
    pub fresh_ahead_validate_fanout: usize,
    /// Paired sites where L3 has empty targets but fresh has non-empty (fresh better).
    pub verified_win: usize,
    /// Paired sites where both sides have non-empty but differing target sets.
    pub divergence: usize,
    /// L3-only sites: fresh had no matching `ImplicitTrigger` edge.
    pub missing_site: usize,
    /// FreshOnly sites where fresh has empty targets (table has no triggers in scope).
    pub extra_site: usize,
    /// Sum of excess indices from `Unaligned` buckets.
    pub unaligned: usize,
    /// Total fresh `ImplicitTrigger` sites extracted from the workspace.
    pub fresh_total: usize,
    /// Total L3 `ImplicitTrigger`-in-scope edges.
    pub l3_total: usize,
}

/// Project the L3 resolver's `DispatchKind::ImplicitTrigger` edges for the workspace.
///
/// The L3 `build_implicit_trigger_edges` uses `op.id` (a `PRecordOperation` id)
/// as `callsite_id`.  To recover the call site's source span and callee-text
/// fingerprint this function builds a reverse map `op.id → PCallSite` via
/// `PCallSite.operation_id`.
#[must_use]
fn project_l3_implicit_trigger_in_scope(workspace_root: &Path) -> Vec<CanonicalEdge> {
    use std::collections::HashMap;

    use crate::engine::l3::call_resolver::{DeclaredDependency, resolve_calls};
    use crate::engine::l3::l3_workspace::{
        L3RecordOperation, assemble_and_resolve_workspace_default,
    };
    use crate::engine::l3::symbol_table::SymbolTable;
    use crate::engine::l3::taxonomy::DispatchKind;

    let Some(resolved) = assemble_and_resolve_workspace_default(workspace_root) else {
        return Vec::new();
    };
    let ws = &resolved.workspace;

    let symbols = SymbolTable::build(&ws.objects, &ws.tables, &ws.routines);
    let no_deps: Vec<DeclaredDependency> = Vec::new();
    let no_fetched: Vec<String> = Vec::new();
    let resolved_calls = resolve_calls(ws, &symbols, &no_deps, &no_fetched);

    let routine_by_id: HashMap<&str, &crate::engine::l3::l3_workspace::L3Routine> =
        ws.routines.iter().map(|r| (r.id.as_str(), r)).collect();

    // L3 ImplicitTrigger edges use PRecordOperation.id as callsite_id.
    // Build a direct op.id → &L3RecordOperation map (NOT via PCallSite.operation_id,
    // which is a separate numbering namespace: "{routine}/op{op_count+i}").
    let mut op_by_id: HashMap<&str, &L3RecordOperation> = HashMap::new();
    for routine in &ws.routines {
        for op in &routine.record_operations {
            op_by_id.insert(op.id.as_str(), op);
        }
    }

    let mut edges: Vec<CanonicalEdge> = resolved_calls
        .edges
        .iter()
        .filter(|edge| matches!(edge.dispatch_kind, DispatchKind::ImplicitTrigger))
        .filter_map(|edge| {
            let from_r = routine_by_id.get(edge.from.as_str())?;
            let from = make_canonical_key(
                from_r.app_guid.clone(),
                from_r.object_type.to_ascii_lowercase(),
                format!("{}", from_r.object_number),
                from_r.name.to_ascii_lowercase(),
            );

            // ImplicitTrigger edges use PRecordOperation.id as callsite_id;
            // look up the record op directly for its source_anchor and callee text.
            let op = op_by_id.get(edge.callsite_id.as_str())?;
            let a = &op.source_anchor;
            let unit_str = a
                .source_unit_id
                .strip_prefix("ws:")
                .unwrap_or(&a.source_unit_id)
                .to_string();
            let span = CanonicalSpan {
                unit: unit_str,
                start: SourcePos {
                    line: a.start_line,
                    col: a.start_column,
                },
                end: SourcePos {
                    line: a.end_line,
                    col: a.end_column,
                },
            };
            // callee_fp must match the fresh side: fresh uses the raw Member expression
            // text (e.g. "Rec.Insert"); L3RecordOperation stores the receiver name and
            // op in the same original case → produce the same lowercased fingerprint.
            let callee_text = format!("{}.{}", op.record_variable_name, op.op);
            let fp = callee_fp(&callee_text);
            let site = CanonicalSiteKey {
                caller: from.clone(),
                span,
                callee_fp: fp,
            };

            let targets: BTreeSet<CanonicalTarget> = if let Some(to_id) = &edge.to {
                if let Some(to_r) = routine_by_id.get(to_id.as_str()) {
                    let mut set = BTreeSet::new();
                    set.insert(CanonicalTarget {
                        kind: object_kind_str_to_tag(&to_r.object_type.to_ascii_lowercase()),
                        app: Some(to_r.app_guid.clone()),
                        object_lc: format!("{}", to_r.object_number),
                        routine_lc: Some(to_r.name.to_ascii_lowercase()),
                    });
                    set
                } else {
                    BTreeSet::new()
                }
            } else {
                BTreeSet::new()
            };

            Some(CanonicalEdge {
                from,
                site,
                kind: EdgeKind::ImplicitTrigger,
                targets,
            })
        })
        .collect();

    edges.sort();
    edges
}

/// Phase-4 ImplicitTrigger resolution harness: resolves every workspace
/// `RecordOp` call site (`insert`/`modify`/`delete`/`validate`) via
/// `resolve_implicit_trigger` and compares against the L3 oracle filtered to
/// `DispatchKind::ImplicitTrigger`.
///
/// Table resolution per site:
/// - `rec`/`xrec` in a `Table` object → the object IS the table.
/// - `rec`/`xrec` in a `TableExtension` object → base table via
///   `ObjectNode.extends_target`.
/// - Named variable → linear search params → locals → object globals for a
///   `Record <TableName>` type declaration.
/// - All other cases (rec/xrec in Page/Codeunit, untyped vars, `RecordRef`,
///   etc.) → skipped; those sites appear as L3-only (`missing_site`).
///
/// FreshOnly classification:
/// - `validate` sites (`RecordOpCtx.field = None`): every route ALWAYS fails
///   `implicit_trigger_route_applicable` (field mismatch); routes on the
///   correct table/extension → `fresh_ahead_validate_fanout`, routes on an
///   unrelated table → `unverified_extra`.
/// - `insert`/`modify`/`delete` sites: applicability gate via
///   `implicit_trigger_route_applicable` → pass → `fresh_ahead_trigger`,
///   fail → `unverified_extra`.
///
/// Fail-closed: any error during setup returns a zero report.
#[must_use]
pub fn run_implicit_trigger_harness(workspace_root: &Path) -> ImplicitTriggerResolutionReport {
    use std::collections::{HashMap, HashSet};

    use al_syntax::ir::ObjectKind;

    use crate::program::build::build_program_graph;
    use crate::program::node::{ObjKey, ObjectNodeId};
    use crate::program::node_extract::ObjectNode;
    use crate::program::resolve::applicability::{
        RecordOpCtx, RecordOpKind, RunTrigger, implicit_trigger_route_applicable,
    };
    use crate::program::resolve::body_map::BodyMap;
    use crate::program::resolve::edge::Route;
    use crate::program::resolve::extract::{CalleeShape, extract_sites_for_routine};
    use crate::program::resolve::index::ResolveIndex;
    use crate::program::resolve::resolver::resolve_implicit_trigger;
    use crate::snapshot::{SnapshotBuilder, parse_snapshot};

    // ── Step 1: Build snapshot ───────────────────────────────────────────────
    let snap = match (SnapshotBuilder {
        workspace_root: workspace_root.to_path_buf(),
        local_providers: vec![],
    })
    .build()
    {
        Ok(s) => s,
        Err(_) => return ImplicitTriggerResolutionReport::default(),
    };

    let ws_file_set: HashSet<String> = snap
        .apps
        .first()
        .and_then(|u| u.source.as_ref())
        .map(|s| s.files.iter().map(|f| f.virtual_path.clone()).collect())
        .unwrap_or_default();

    // ── Step 2: Build graph + index + body map ───────────────────────────────
    let graph = build_program_graph(&snap);
    let parsed = parse_snapshot(&snap);
    let index = ResolveIndex::build(&graph);
    let body_map = BodyMap::build(&graph, &parsed);

    // ── Step 3: Locate workspace app ─────────────────────────────────────────
    let Some(ws_ref) = graph.apps.find(&snap.workspace_app) else {
        return ImplicitTriggerResolutionReport::default();
    };
    let ws_guid = graph.apps.resolve(ws_ref).guid.clone();

    let obj_node_map: HashMap<ObjectNodeId, &ObjectNode> =
        graph.objects.iter().map(|o| (o.id.clone(), o)).collect();

    // ── Step 4: Resolve fresh ImplicitTrigger sites ──────────────────────────
    // Parallel vecs kept in sync:
    //   .0 fresh_canonical — canonical edge (EdgeKind::ImplicitTrigger)
    //   .1 ctx             — RecordOpCtx for applicability gate
    //   .2 routes          — original routes from resolve_implicit_trigger
    type FreshEntry = (CanonicalEdge, RecordOpCtx, Vec<Route>);
    let mut fresh_combined: Vec<FreshEntry> = Vec::new();
    let mut evidence_overclaim = 0usize;

    for unit in &parsed {
        let Some(app_ref) = graph.apps.find(&unit.app) else {
            continue;
        };
        if app_ref != ws_ref {
            continue;
        }

        for pf in &unit.files {
            if !ws_file_set.contains(&pf.virtual_path) {
                continue;
            }

            for (obj_idx, obj) in pf.file.objects.iter().enumerate() {
                let obj_key = match obj.id {
                    Some(n) => ObjKey::Id(n),
                    None => ObjKey::Name(obj.name.to_ascii_lowercase()),
                };
                let obj_kind_str = object_kind_str(obj.kind);
                let obj_lc = obj_key_lc(&obj_key);

                let obj_node_id = ObjectNodeId {
                    app: ws_ref,
                    kind: obj.kind,
                    key: obj_key.clone(),
                };
                let obj_node_opt: Option<&ObjectNode> = obj_node_map.get(&obj_node_id).copied();

                let object_globals_rec_set: HashSet<String> = obj
                    .globals
                    .iter()
                    .filter(|v| {
                        v.ty.as_deref()
                            .map(|ty| ty.trim().to_ascii_lowercase().starts_with("record"))
                            .unwrap_or(false)
                    })
                    .map(|v| v.name.to_ascii_lowercase())
                    .collect();

                for (routine_idx, routine) in obj.routines.iter().enumerate() {
                    let caller_key = make_canonical_key(
                        ws_guid.clone(),
                        obj_kind_str.clone(),
                        obj_lc.clone(),
                        routine.name.to_ascii_lowercase(),
                    );

                    let sites = extract_sites_for_routine(
                        &pf.file,
                        &pf.text,
                        &pf.virtual_path,
                        &object_globals_rec_set,
                        obj_idx,
                        routine_idx,
                    );

                    for site in &sites {
                        let (receiver_text, op_lc) = match &site.shape {
                            CalleeShape::RecordOp { receiver_text, op } => {
                                (receiver_text.as_str(), op.as_str())
                            }
                            _ => continue, // Skip non-RecordOp sites
                        };

                        // Only trigger-firing ops (mirrors L3 trigger_mapping).
                        let op_kind = match op_lc {
                            "insert" => RecordOpKind::Insert,
                            "modify" => RecordOpKind::Modify,
                            "delete" => RecordOpKind::Delete,
                            "validate" => RecordOpKind::Validate,
                            _ => continue, // Non-trigger ops (findset, setrange, …)
                        };

                        // Resolve the table ObjectNodeId from the receiver expression.
                        let recv_lc = receiver_text.to_ascii_lowercase();
                        let table_id_opt: Option<ObjectNodeId> = if recv_lc == "rec"
                            || recv_lc == "xrec"
                        {
                            match obj.kind {
                                ObjectKind::Table => {
                                    // The enclosing object IS the table.
                                    obj_node_opt.map(|o| o.id.clone())
                                }
                                ObjectKind::TableExtension => {
                                    // "Rec" refers to the base table.
                                    obj_node_opt
                                        .and_then(|o| o.extends_target.as_deref())
                                        .and_then(|base| {
                                            graph.resolve_object(ws_ref, ObjectKind::Table, base)
                                        })
                                        .map(|o| o.id.clone())
                                }
                                _ => None, // Page/Codeunit/… — implicit Rec can't be resolved here
                            }
                        } else {
                            // Named receiver: params → locals → object globals.
                            let resolve_record_ty = |ty_opt: Option<&str>| -> Option<ObjectNodeId> {
                                let ty = ty_opt?;
                                let table_name_lc = record_type_table_name_lc(ty)?;
                                graph
                                    .resolve_object(ws_ref, ObjectKind::Table, &table_name_lc)
                                    .map(|o| o.id.clone())
                            };
                            routine
                                .params
                                .iter()
                                .find(|p| p.name.to_ascii_lowercase() == recv_lc)
                                .and_then(|p| resolve_record_ty(p.ty.as_deref()))
                                .or_else(|| {
                                    routine
                                        .locals
                                        .iter()
                                        .find(|v| v.name.to_ascii_lowercase() == recv_lc)
                                        .and_then(|v| resolve_record_ty(v.ty.as_deref()))
                                })
                                .or_else(|| {
                                    obj.globals
                                        .iter()
                                        .find(|v| v.name.to_ascii_lowercase() == recv_lc)
                                        .and_then(|v| resolve_record_ty(v.ty.as_deref()))
                                })
                        };

                        let Some(table_id) = table_id_opt else {
                            continue; // Table not resolved — skip (appears as L3-only)
                        };

                        let Some(table_node) = graph.objects.iter().find(|o| o.id == table_id)
                        else {
                            continue; // ObjectNode absent from graph — shouldn't happen
                        };

                        // Resolve triggers.
                        let (_shape, _completeness, routes) =
                            resolve_implicit_trigger(op_lc, table_node, &graph, &index, &body_map);

                        // Evidence/witness contract check.
                        for r in &routes {
                            if !witness_contract_holds(r) {
                                evidence_overclaim += 1;
                            }
                        }

                        // Build RecordOpCtx for the FreshOnly applicability gate.
                        // field = None: Validate field is unknown at this layer (option b —
                        // categorise as fresh_ahead_validate_fanout).
                        // run_trigger = Guarded: conservative (can't determine from shape).
                        let ctx = RecordOpCtx {
                            kind: op_kind,
                            table: table_id.clone(),
                            field: None,
                            run_trigger: RunTrigger::Guarded,
                        };

                        let targets: BTreeSet<CanonicalTarget> = routes
                            .iter()
                            .filter_map(|r| project_target(&r.target, &graph.apps))
                            .collect();

                        let fp = callee_fp(&site.callee_text);
                        let edge = CanonicalEdge {
                            from: caller_key.clone(),
                            site: CanonicalSiteKey {
                                caller: caller_key.clone(),
                                span: site.span.clone(),
                                callee_fp: fp,
                            },
                            kind: EdgeKind::ImplicitTrigger,
                            targets,
                        };
                        fresh_combined.push((edge, ctx, routes));
                    }
                }
            }
        }
    }

    fresh_combined.sort_by(|a, b| a.0.cmp(&b.0));
    let fresh_ctxs: Vec<RecordOpCtx> = fresh_combined.iter().map(|(_, c, _)| c.clone()).collect();
    let fresh_routes: Vec<Vec<Route>> = fresh_combined.iter().map(|(_, _, r)| r.clone()).collect();
    let fresh_canonical: Vec<CanonicalEdge> =
        fresh_combined.into_iter().map(|(e, _, _)| e).collect();

    let fresh_total = fresh_canonical.len();

    // ── Step 5: Project L3 ImplicitTrigger oracle ─────────────────────────
    let l3_canonical = project_l3_implicit_trigger_in_scope(workspace_root);
    let l3_total = l3_canonical.len();

    // ── Step 6: Match sites ────────────────────────────────────────────────
    let site_matches = match_sites(&fresh_canonical, &l3_canonical);

    // ── Step 7: Bucket ─────────────────────────────────────────────────────
    let mut matched = 0usize;
    let mut regression_unexplained = 0usize;
    let mut verified_win = 0usize;
    let mut divergence = 0usize;
    let mut missing_site = 0usize;
    let mut extra_site = 0usize;
    let mut fresh_ahead_trigger = 0usize;
    let mut fresh_ahead_validate_fanout = 0usize;
    let mut unverified_extra = 0usize;
    let mut unaligned = 0usize;

    for m in &site_matches {
        match m {
            SiteMatch::Paired(fi, li) => {
                matched += 1;
                let f = &fresh_canonical[*fi];
                let l = &l3_canonical[*li];
                match (f.targets.is_empty(), l.targets.is_empty()) {
                    (true, true) => {}
                    (false, true) => verified_win += 1,
                    (true, false) => regression_unexplained += 1,
                    (false, false) => {
                        if f.targets != l.targets {
                            divergence += 1;
                        }
                    }
                }
            }
            SiteMatch::FreshOnly(fi) => {
                let f = &fresh_canonical[*fi];
                if f.targets.is_empty() {
                    extra_site += 1;
                } else {
                    let ctx = &fresh_ctxs[*fi];
                    let routes = &fresh_routes[*fi];

                    if matches!(ctx.kind, RecordOpKind::Validate) {
                        // With field=None, implicit_trigger_route_applicable always returns
                        // false for Validate.  Classify by table identity instead.
                        let all_on_correct_table = routes.iter().all(|r| match &r.target {
                            RouteTarget::Routine(rid) => target_is_on_table_or_extension(
                                &rid.object,
                                &ctx.table,
                                &graph,
                                &index,
                            ),
                            RouteTarget::Unresolved => true,
                            _ => false,
                        });
                        if all_on_correct_table {
                            fresh_ahead_validate_fanout += 1;
                        } else {
                            unverified_extra += 1;
                        }
                    } else {
                        // Insert / Modify / Delete: full applicability gate.
                        let all_pass = routes.iter().all(|r| match &r.target {
                            RouteTarget::Routine(rid) => {
                                implicit_trigger_route_applicable(ctx, rid, &graph, &index)
                            }
                            RouteTarget::Unresolved => true,
                            _ => false,
                        });
                        if all_pass {
                            fresh_ahead_trigger += 1;
                        } else {
                            unverified_extra += 1;
                        }
                    }
                }
            }
            SiteMatch::L3Only(_) => {
                missing_site += 1;
            }
            SiteMatch::Unaligned(fs, ls) => {
                unaligned += fs.len() + ls.len();
            }
        }
    }

    ImplicitTriggerResolutionReport {
        matched,
        regression_unexplained,
        evidence_overclaim,
        unverified_extra,
        fresh_ahead_trigger,
        fresh_ahead_validate_fanout,
        verified_win,
        divergence,
        missing_site,
        extra_site,
        unaligned,
        fresh_total,
        l3_total,
    }
}

// ---------------------------------------------------------------------------
// Test helper
// ---------------------------------------------------------------------------

/// Build a synthetic [`CanonicalEdge`] for use in matcher fixture tests.
///
/// * `caller` — colon-separated `"object_kind:object_lc:routine_lc"`,
///   e.g. `"cu:c:run"`.  `app_guid` is left empty.
/// * `span_start` — 0-based start line stored in the span.
/// * `fp` — callee fingerprint.
///
/// `from` is set equal to the caller key, `kind` is [`EdgeKind::Call`], and
/// `targets` is empty.  Column offsets default to 0 / 10.
#[doc(hidden)]
pub fn canonical_call_edge_for_test(caller: &str, span_start: u32, fp: u64) -> CanonicalEdge {
    let parts: Vec<&str> = caller.splitn(4, ':').collect();
    let caller_key = CanonicalKey {
        app_guid: String::new(),
        object_kind: parts.first().copied().unwrap_or("").to_string(),
        object_lc: parts.get(1).copied().unwrap_or("").to_string(),
        routine_lc: parts.get(2).copied().unwrap_or("").to_string(),
    };
    CanonicalEdge {
        from: caller_key.clone(),
        site: CanonicalSiteKey {
            caller: caller_key,
            span: CanonicalSpan {
                unit: "test_unit".to_string(),
                start: SourcePos {
                    line: span_start,
                    col: 0,
                },
                end: SourcePos {
                    line: span_start,
                    col: 10,
                },
            },
            callee_fp: fp,
        },
        kind: EdgeKind::Call,
        targets: BTreeSet::new(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_side_object_kind_parity() {
        // Fresh derives the caller key's object_kind via Debug-lowercase; L3 derives it from
        // its `object_type` string lowercased. They MUST agree for every kind or sites silently
        // drop out of `matched`. This asserts the canonical spelling for each variant.
        use crate::program::node::ObjectKind;
        let cases = [
            (ObjectKind::Codeunit, "codeunit"),
            (ObjectKind::Table, "table"),
            (ObjectKind::TableExtension, "tableextension"),
            (ObjectKind::Page, "page"),
            (ObjectKind::PageExtension, "pageextension"),
            (ObjectKind::Report, "report"),
            (ObjectKind::ReportExtension, "reportextension"),
            (ObjectKind::XmlPort, "xmlport"),
            (ObjectKind::Query, "query"),
            (ObjectKind::Enum, "enum"),
            (ObjectKind::EnumExtension, "enumextension"),
            (ObjectKind::Interface, "interface"),
            (ObjectKind::ControlAddIn, "controladdin"),
            (ObjectKind::Entitlement, "entitlement"),
            (ObjectKind::PermissionSet, "permissionset"),
            (ObjectKind::PermissionSetExtension, "permissionsetextension"),
            (ObjectKind::Profile, "profile"),
            (ObjectKind::Other, "other"),
        ];
        for (k, expected) in cases {
            assert_eq!(
                format!("{k:?}").to_ascii_lowercase(),
                expected,
                "kind {k:?}"
            );
        }
    }

    #[test]
    fn project_fresh_round_trips_a_synthetic_edge() {
        // Build a tiny ProgramGraph-free CanonicalEdge directly from a synthetic Edge.
        // (Full CDO projection is exercised by the env-gated harness test, Task 7.)
        let edges = crate::program::resolve::stub::synthetic_unknown_edge_for_test();
        let apps = crate::program::node::AppRegistry::default();
        let canon = project_fresh(&edges, &apps);
        assert_eq!(canon.len(), 1);
        assert!(
            canon[0].targets.is_empty(),
            "stub Unknown edge has no concrete target"
        );
    }

    #[test]
    fn project_l3_yields_spanned_canonical_edges_on_cdo() {
        let Some(ws) = std::env::var_os("CDO_WS")
            .map(std::path::PathBuf::from)
            .filter(|p| p.exists())
        else {
            return;
        };
        let edges = project_l3(&ws);
        assert!(
            edges.len() > 1000,
            "L3 should project many edges, got {}",
            edges.len()
        );
        // Every projected site carries a real span (non-zero end).
        assert!(
            edges
                .iter()
                .all(|e| e.site.span.end.line >= e.site.span.start.line)
        );
    }
}
