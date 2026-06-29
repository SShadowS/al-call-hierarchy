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
            let span = CanonicalSpan {
                unit: a.source_unit_id.clone(),
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
