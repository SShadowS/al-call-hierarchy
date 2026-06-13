//! R2b EXIT GATE — native L3-DIRECT call-graph resolution oracle.
//!
//! Ground-truth-free, STRUCTURAL oracles run NATIVELY against the Rust L3 call
//! resolver (`src/engine/l3/call_resolver.rs` + `call_graph_projection.rs`). Each
//! invariant drives an inline single-app workspace through the real
//! `assemble_and_resolve_default → project_call_graph` path and asserts a call-
//! resolution PROPERTY DIRECTLY on the resolved/projected graph — NOT a golden diff
//! against al-sem expected strings (that is `l3cg_resolution_vectors.rs` + the
//! differential's `*.l3cg.golden.json`).
//!
//! ## Why an L3-DIRECT oracle (not just the byte-parity differential)
//!
//! The corpus differential (`tests/differential.rs`,
//! `differential_l3_call_graph_match_goldens`) is BYTE-PARITY with al-sem: if BOTH
//! engines made the same resolution mistake, a pure equality diff would still pass.
//! These oracles assert the call-graph CONTRACT in absolute terms — a callsite is
//! NEVER collapsed to one edge (interface dispatch is multi-edge), a resolved edge's
//! `to` actually resolves in the symbol table, an ambiguous overload yields ≥2
//! candidates, the opaque-vs-external-target distinction holds, `upgrade_bindings`
//! runs exactly once (no double-upgrade diagnostic), and the edge byte-order sort is
//! stable. A FAILURE here that the differential misses means BOTH engines are wrong —
//! flag it loudly (it is NOT "fix the golden").
//!
//! ## Covered (source-only intra-workspace call resolution — R2b's guard)
//!   - interface dispatch emits ≥1 edge PER resolved impl and the projection NEVER
//!     collapses by callsiteId (a single callsite carries >1 edge), all "maybe";
//!   - a resolved edge's `to` StableRoutineId resolves to a real workspace routine;
//!   - an ambiguous overload → resolution "ambiguous" with ≥2 candidates;
//!   - member-not-found (wrong arity) → resolution "member-not-found";
//!   - object-run (`Codeunit.Run`) resolves to the target's `OnRun`;
//!   - opaque (object-run, target NOT in source) vs external-target (member call,
//!     target NOT in source, no unfetched dep) — the two distinct classifications;
//!   - `upgrade_bindings` runs EXACTLY once (no double-upgrade diagnostic ever);
//!   - the within-group edge sort is deterministic / byte-order stable.
//!
//! ## Deferred (NOT source-only intra-workspace; later gates — where they land)
//!   - CROSS-APP opaque/external — a member/object-run target that lives in a `.app`
//!     symbol package (so `has_unfetched_declared_dependency` is TRUE and member
//!     misses become `opaque`). The R2b corpus + this oracle are SOURCE-ONLY (no
//!     `.app` ingestion, empty `primary_dependencies`), so member-opaque is
//!     structurally unreachable here → R2.5 (`.app` projection). We DO assert the
//!     external-target branch (reachable source-only) and object-run opaque.
//!   - EVENT-graph resolution (publisher↔subscriber edges, `parseSubscriberAttribute`,
//!     open-world synthetic ids) → R2c.
//!   - REACHABILITY-crosscheck + inter-never-overclaim soundness oracles — these
//!     need the COMBINED graph (call + event + implicit) and a reachability walk that
//!     R2b does not compute (R2b stops at the resolved call edges). They land where
//!     reachability is computed (the L4/combined-graph gate), NOT here.

use al_call_hierarchy::engine::l3::call_graph_projection::{L3CallGraphProjection, PCallEdge};
use al_call_hierarchy::engine::l3::call_resolver::{resolve_calls, DeclaredDependency};
use al_call_hierarchy::engine::l3::l3_workspace::{assemble_and_resolve_default, L3Resolved};
use al_call_hierarchy::engine::l3::symbol_table::SymbolTable;

const APP_GUID: &str = "2b000000-0000-0000-0000-0000000002bb";

/// Assemble + resolve an inline multi-file workspace.
fn resolve_ws(files: &[(&str, &str)]) -> L3Resolved {
    let owned: Vec<(String, String)> = files
        .iter()
        .map(|(n, s)| (n.to_string(), s.to_string()))
        .collect();
    assemble_and_resolve_default(&owned, APP_GUID)
}

/// Project the call graph for an inline workspace.
fn project_ws(files: &[(&str, &str)]) -> L3CallGraphProjection {
    resolve_ws(files).project_call_graph()
}

/// All edges across all groups (flattened — for assertions over the edge set).
fn all_edges(p: &L3CallGraphProjection) -> Vec<&PCallEdge> {
    p.groups.iter().flat_map(|g| g.edges.iter()).collect()
}

/// Every StableRoutineId in the workspace (the resolved-edge `to` symbol table).
fn all_stable_routine_ids(r: &L3Resolved) -> std::collections::HashSet<String> {
    r.workspace
        .routines
        .iter()
        .map(|rt| rt.stable_routine_id.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// Invariant 1: interface dispatch is MULTI-edge and is NEVER collapsed.
// ---------------------------------------------------------------------------

#[test]
fn interface_dispatch_emits_one_edge_per_resolved_impl_never_collapsed() {
    let files = &[
        (
            "src/iface.al",
            "interface IProc { procedure Process(); }",
        ),
        (
            "src/a.al",
            "codeunit 50100 \"Proc A\" implements IProc { procedure Process() begin end; }",
        ),
        (
            "src/b.al",
            "codeunit 50101 \"Proc B\" implements IProc { procedure Process() begin end; }",
        ),
        (
            "src/disp.al",
            "codeunit 50102 Disp { var P: Interface IProc; procedure Go() begin P.Process(); end; }",
        ),
    ];
    let p = project_ws(files);

    // Exactly ONE group is the interface callsite; it must carry 2 edges (one per
    // impl) — the projection MUST NOT collapse a callsite to a single CallEdge.
    let iface_groups: Vec<_> = p
        .groups
        .iter()
        .filter(|g| g.edges.iter().any(|e| e.dispatch_kind == "interface"))
        .collect();
    assert_eq!(
        iface_groups.len(),
        1,
        "expected exactly one interface callsite group"
    );
    let g = iface_groups[0];
    assert_eq!(
        g.edges.len(),
        2,
        "interface dispatch over 2 impls MUST emit 2 edges (multi-edge, never collapsed); got {:#?}",
        g.edges
    );
    // Every interface edge is resolution "maybe" (NOT "resolved") and has a distinct
    // `to`.
    for e in &g.edges {
        assert_eq!(e.dispatch_kind, "interface");
        assert_eq!(e.resolution, "maybe", "interface impl edges are 'maybe'");
        assert!(e.to.is_some(), "a resolved impl edge has a `to`");
    }
    let tos: std::collections::HashSet<_> = g.edges.iter().filter_map(|e| e.to.clone()).collect();
    assert_eq!(
        tos.len(),
        2,
        "the two impl edges resolve to DISTINCT routines"
    );

    // Group-level dispatchMeta is present, names the interface, totalImpls == 2.
    let dm = g
        .dispatch_meta
        .as_ref()
        .expect("interface group carries group-level dispatchMeta");
    assert_eq!(dm.interface_name, "IProc");
    assert_eq!(dm.total_impls, 2);
    assert!(dm.unresolved_impls.is_empty(), "both impls resolved");
}

// ---------------------------------------------------------------------------
// Invariant 2: a resolved edge's `to` resolves in the symbol table.
// ---------------------------------------------------------------------------

#[test]
fn resolved_edge_to_resolves_in_symbol_table() {
    let files = &[(
        "src/main.al",
        "codeunit 50100 A { procedure Caller() begin Target(); end; procedure Target() begin end; }",
    )];
    let resolved = resolve_ws(files);
    let known = all_stable_routine_ids(&resolved);
    let p = resolved.project_call_graph();

    let resolved_edges: Vec<_> = all_edges(&p)
        .into_iter()
        .filter(|e| e.resolution == "resolved" || e.resolution == "maybe")
        .collect();
    assert!(
        !resolved_edges.is_empty(),
        "expected at least one resolved edge"
    );
    for e in resolved_edges {
        let to = e.to.as_ref().expect("a resolved edge has a `to`");
        assert!(
            known.contains(to),
            "resolved edge `to` {to:?} must be a real workspace routine; known = {known:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Invariant 3: ambiguous overload → "ambiguous" with ≥2 candidates.
// ---------------------------------------------------------------------------

#[test]
fn ambiguous_overload_yields_two_or_more_candidates() {
    // Two same-arity overloads, the arg type (Variant) eliminates neither →
    // ambiguous with the candidate set.
    let files = &[
        (
            "src/setup.al",
            "table 50000 Setup { fields { field(1; \"Code\"; Code[20]) { } } keys { key(PK; \"Code\") { } } }",
        ),
        (
            "src/imp.al",
            "codeunit 50100 Importer { procedure Import(FileName: Text; S: Record Setup) begin end; procedure Import(Strm: InStream; S: Record Setup) begin end; }",
        ),
        (
            "src/caller.al",
            "codeunit 50101 Caller { procedure Run(S: Record Setup) var Imp: Codeunit Importer; Anything: Variant; begin Imp.Import(Anything, S); end; }",
        ),
    ];
    let p = project_ws(files);
    let amb: Vec<_> = all_edges(&p)
        .into_iter()
        .filter(|e| e.resolution == "ambiguous")
        .collect();
    assert_eq!(
        amb.len(),
        1,
        "expected exactly one ambiguous edge; got {amb:#?}"
    );
    let cands = amb[0]
        .candidates
        .as_ref()
        .expect("an ambiguous edge carries candidates");
    assert!(cands.len() >= 2, "ambiguous → ≥2 candidates; got {cands:?}");
    // Candidates are sorted (byte-order) — the projection's contract.
    let mut sorted = cands.clone();
    sorted.sort();
    assert_eq!(cands, &sorted, "candidates must be byte-order sorted");
}

// ---------------------------------------------------------------------------
// Invariant 4: member-not-found (wrong arity) → "member-not-found".
// ---------------------------------------------------------------------------

#[test]
fn wrong_arity_member_is_member_not_found() {
    let files = &[(
        "src/main.al",
        "codeunit 50100 Helper { procedure DoIt() begin end; } codeunit 50101 Caller { var H: Codeunit Helper; procedure Run() begin H.DoIt(1, 2); end; }",
    )];
    let p = project_ws(files);
    let mnf: Vec<_> = all_edges(&p)
        .into_iter()
        .filter(|e| e.resolution == "member-not-found")
        .collect();
    assert_eq!(
        mnf.len(),
        1,
        "a wrong-arity member call → member-not-found; got {:#?}",
        all_edges(&p)
    );
    assert_eq!(mnf[0].dispatch_kind, "method");
    assert!(mnf[0].to.is_none(), "member-not-found has no `to`");
}

// ---------------------------------------------------------------------------
// Invariant 5: object-run resolves to the target's OnRun.
// ---------------------------------------------------------------------------

#[test]
fn object_run_resolves_to_on_run() {
    let files = &[(
        "src/main.al",
        "codeunit 50100 Worker { trigger OnRun() begin end; } codeunit 50101 Caller { procedure Go() begin Codeunit.Run(Codeunit::Worker); end; }",
    )];
    let resolved = resolve_ws(files);
    let known = all_stable_routine_ids(&resolved);
    let p = resolved.project_call_graph();

    let run_edges: Vec<_> = all_edges(&p)
        .into_iter()
        .filter(|e| e.dispatch_kind == "codeunit-run")
        .collect();
    assert_eq!(
        run_edges.len(),
        1,
        "expected one codeunit-run edge; got {:#?}",
        all_edges(&p)
    );
    let e = run_edges[0];
    assert_eq!(e.resolution, "resolved");
    let to = e.to.as_ref().expect("object-run resolved → `to`");
    assert!(
        known.contains(to),
        "object-run `to` resolves in the symbol table"
    );
}

// ---------------------------------------------------------------------------
// Invariant 6: opaque (object-run miss) vs external-target (member miss). The two
// distinct source-only classifications, exercised by the two real fixtures' shape.
// ---------------------------------------------------------------------------

#[test]
fn external_target_vs_object_run_opaque_distinction() {
    // (a) member call on a Codeunit-typed var whose object is NOT in source, no
    //     declared deps → external-target (member miss, fetched-complete).
    let external = &[(
        "src/main.al",
        "codeunit 50350 ExternalCaller { var Helper: Codeunit \"External Dep Helper\"; procedure Go() begin Helper.RunIt(); end; }",
    )];
    let pe = project_ws(external);
    let ext: Vec<_> = all_edges(&pe)
        .into_iter()
        .filter(|e| e.resolution == "external-target")
        .collect();
    assert_eq!(
        ext.len(),
        1,
        "member miss (no dep) → external-target; got {:#?}",
        all_edges(&pe)
    );
    assert_eq!(ext[0].dispatch_kind, "method");
    let xt = ext[0]
        .external_type_ref
        .as_ref()
        .expect("external-target carries externalTypeRef");
    assert_eq!(xt.kind, "Codeunit");
    assert_eq!(xt.name, "External Dep Helper");

    // (b) object-run to a Codeunit NOT in source → ALWAYS opaque (object-run misses
    //     are opaque regardless of dep state).
    let opaque = &[(
        "src/main.al",
        "codeunit 50360 RunCaller { procedure Go() begin Codeunit.Run(Codeunit::\"Missing Worker\"); end; }",
    )];
    let po = project_ws(opaque);
    let op: Vec<_> = all_edges(&po)
        .into_iter()
        .filter(|e| e.resolution == "opaque")
        .collect();
    assert_eq!(
        op.len(),
        1,
        "object-run miss → opaque; got {:#?}",
        all_edges(&po)
    );
    assert_eq!(op[0].dispatch_kind, "codeunit-run");
    assert!(op[0].to.is_none(), "opaque has no `to`");

    // The two classifications are DISTINCT (no external-target in the opaque case,
    // no opaque in the external case).
    assert!(
        all_edges(&po)
            .iter()
            .all(|e| e.resolution != "external-target"),
        "object-run miss must NOT be external-target"
    );
    assert!(
        all_edges(&pe).iter().all(|e| e.resolution != "opaque"),
        "member miss (no dep) must NOT be opaque"
    );
}

// ---------------------------------------------------------------------------
// Invariant 7: upgrade_bindings runs EXACTLY once — no double-upgrade diagnostic.
// ---------------------------------------------------------------------------

#[test]
fn upgrade_bindings_runs_once_no_double_upgrade() {
    // A resolved record-var binding (the case `upgrade_bindings` mutates). A second
    // upgrade pass would emit the double-upgrade diagnostic + skip; one resolve pass
    // must emit NONE.
    let files = &[(
        "src/main.al",
        "table 50000 Cust { fields { field(1; \"No.\"; Code[20]) { } } keys { key(PK; \"No.\") { } } } \
         codeunit 50100 A { procedure Caller() var C: Record Cust; begin Take(C); end; procedure Take(var C: Record Cust) begin end; }",
    )];
    let resolved = resolve_ws(files);
    let ws = &resolved.workspace;
    let symbols = SymbolTable::build(&ws.objects, &ws.tables, &ws.routines);
    let no_deps: Vec<DeclaredDependency> = Vec::new();
    let no_fetched: Vec<String> = Vec::new();
    let result = resolve_calls(ws, &symbols, &no_deps, &no_fetched);

    assert!(
        result.diagnostics.is_empty(),
        "a single resolve pass must NOT emit a double-upgrade diagnostic; got {:?}",
        result.diagnostics
    );

    // The binding was actually upgraded (calleeParameterIsVar true, resolution
    // resolved) — proving the upgrade DID fire (so "no diagnostic" isn't vacuous).
    let p = resolved.project_call_graph();
    let upgraded = p
        .bindings
        .iter()
        .flat_map(|b| b.bindings.iter())
        .any(|b| b.callee_parameter_is_var && b.binding_resolution == "resolved");
    assert!(
        upgraded,
        "the var-record binding must be upgraded to resolved (calleeParameterIsVar=true)"
    );
}

// ---------------------------------------------------------------------------
// Invariant 8: the within-group edge sort is deterministic / byte-order stable.
// ---------------------------------------------------------------------------

#[test]
fn edge_sort_is_deterministic_byte_order_stable() {
    let files = &[
        (
            "src/iface.al",
            "interface IProc { procedure Process(); }",
        ),
        (
            "src/a.al",
            "codeunit 50100 \"Proc A\" implements IProc { procedure Process() begin end; }",
        ),
        (
            "src/b.al",
            "codeunit 50101 \"Proc B\" implements IProc { procedure Process() begin end; }",
        ),
        (
            "src/c.al",
            "codeunit 50102 \"Proc C\" implements IProc { procedure Process() begin end; }",
        ),
        (
            "src/disp.al",
            "codeunit 50103 Disp { var P: Interface IProc; procedure Go() begin P.Process(); end; }",
        ),
    ];
    // Projecting the SAME workspace twice yields byte-identical JSON (no Map/Set
    // iteration leaks; the 6 sorts are byte-order deterministic).
    let j1 = serde_json::to_string(&project_ws(files)).unwrap();
    let j2 = serde_json::to_string(&project_ws(files)).unwrap();
    assert_eq!(j1, j2, "projection must be deterministic across runs");

    // Within the (3-impl) interface group, edges are sorted by their byte-order key
    // → the `to` ids are in non-decreasing order (the dominant key component once
    // resolution/dispatchKind are equal across impl edges).
    let p = project_ws(files);
    let g = p
        .groups
        .iter()
        .find(|g| g.edges.len() == 3 && g.edges.iter().all(|e| e.dispatch_kind == "interface"))
        .expect("the 3-impl interface group");
    let tos: Vec<String> = g.edges.iter().filter_map(|e| e.to.clone()).collect();
    let mut sorted = tos.clone();
    sorted.sort();
    assert_eq!(
        tos, sorted,
        "interface impl edges sorted byte-order by `to`"
    );
}

// ---------------------------------------------------------------------------
// Invariant 9 (Phase 2): every `builtin` member edge's method is in the catalog,
// and no edge is BOTH builtin and resolved (mutually exclusive resolutions).
// ---------------------------------------------------------------------------

#[test]
fn every_builtin_member_edge_method_is_in_the_catalog() {
    use al_call_hierarchy::engine::l3::member_builtins::{
        classify_receiver, member_builtin_disposition,
    };

    // A workspace exercising Record + framework + RecordRef intrinsics. (FieldNo /
    // GetFilter are Record call-site intrinsics; FindSet/SetRange are L2
    // record_operations and never reach the resolver, so we use call-site methods.)
    let files = &[(
        "src/main.al",
        "table 50000 Cust { fields { field(1; \"No.\"; Code[20]) { } } keys { key(PK; \"No.\") { } } } \
         codeunit 50100 A { procedure Go() var C: Record Cust; J: JsonObject; R: RecordRef; begin \
         C.FieldNo(\"No.\"); C.GetFilter(\"No.\"); J.Add('k', 1); R.Open(18); end; }",
    )];
    let resolved = resolve_ws(files);
    let p = resolved.project_call_graph();

    let builtin_edges: Vec<_> = all_edges(&p)
        .into_iter()
        .filter(|e| e.resolution == "builtin")
        .collect();
    assert!(!builtin_edges.is_empty(), "expected builtin member edges");

    // CONTRACT 1: a builtin edge is NEVER also a resolved-to-routine edge.
    for e in all_edges(&p) {
        if e.resolution == "builtin" {
            assert!(
                e.to.is_none(),
                "a builtin edge has no resolved `to`; edge: {e:#?}"
            );
        }
    }

    // CONTRACT 2: every builtin method seen is a real catalog entry for some
    // catalog-eligible receiver kind declared in the fixture's routine variables.
    let ws = &resolved.workspace;
    let declared_types: Vec<String> = ws
        .routines
        .iter()
        .flat_map(|r| r.variables.iter().map(|v| v.declared_type.clone()))
        .collect();
    let catalog_kinds: Vec<_> = declared_types
        .iter()
        .filter_map(|d| classify_receiver(d))
        .collect();
    assert!(
        !catalog_kinds.is_empty(),
        "fixture has catalog-eligible receivers"
    );
    for method in ["fieldno", "getfilter", "add", "open"] {
        let hit = catalog_kinds
            .iter()
            .any(|k| member_builtin_disposition(*k, method).is_some());
        assert!(
            hit,
            "method {method} must be a catalog builtin for some fixture receiver kind"
        );
    }
}

// ---------------------------------------------------------------------------
// Invariant 10 (Phase 2): a Record-receiver method that is a real built-in
// classifies `builtin`, not `unknown` (the core reclassification contract).
// ---------------------------------------------------------------------------

#[test]
fn record_builtin_classifies_builtin_not_unknown() {
    let files = &[(
        "src/main.al",
        "table 50000 Cust { fields { field(1; \"No.\"; Code[20]) { } } keys { key(PK; \"No.\") { } } } \
         codeunit 50100 A { procedure Go() var C: Record Cust; begin C.FieldNo(\"No.\"); end; }",
    )];
    let p = project_ws(files);
    let builtin: Vec<_> = all_edges(&p)
        .into_iter()
        .filter(|e| e.resolution == "builtin")
        .collect();
    assert_eq!(
        builtin.len(),
        1,
        "Record.FieldNo -> exactly one builtin edge; got {:#?}",
        all_edges(&p)
    );
    assert_eq!(builtin[0].dispatch_kind, "builtin");
    assert!(
        all_edges(&p)
            .iter()
            .all(|e| !(e.dispatch_kind == "method" && e.resolution == "unknown")),
        "no Record intrinsic remains a method/unknown hole"
    );
}
