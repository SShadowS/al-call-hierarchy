//! R4 — L5 DETECTOR FINDINGS differential over the SOURCE-ONLY smoke corpus.
//!
//! For each committed al-sem golden under `tests/r4-goldens/<fixture>.r4.golden.json`,
//! run the Rust source-only L0→L3→L5 pass (`assemble_and_resolve_workspace_default(...)`
//! → `engine::l5::finding::project_r4_findings(...)` over the REGISTERED detectors)
//! over the matching `tests/r0-corpus/<fixture>` workspace.
//!
//! ## Wave gating (the ACCEPTANCE GATE)
//!
//! `ported: true` fixtures MUST byte-match their golden END-TO-END. `ported: false`
//! fixtures run cleanly and produce the registered-detector subset (empty for not-yet-
//! ported detectors). Each wave flips its fixture(s) to `ported: true` as they land.
//!
//! ## Anti-degenerate (fail-on-zero)
//!
//! Each ported detector's positive fixture MUST produce ≥1 finding AND byte-match.
//!
//! ## Negative assertions
//!
//! Each new R4-A detector (d5/d10/d11/d18/d21/d36, plus d19/d20/d29) MUST yield 0
//! findings over a neutral fixture that lacks its pattern. R4-B adds d22/d33.
//!
//! ## KNOWN_DIVERGENCES gating
//!
//! Reuses the repo-root `KNOWN_DIVERGENCES.json` with exact `(test, fixture, path)`
//! matching, scoped to `test == R4_TEST_NAME`. Target: empty for the ported subset.

use std::collections::BTreeSet;
use std::path::PathBuf;

use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_workspace_default;
use al_call_hierarchy::engine::l5::detectors::registered_detectors;
use al_call_hierarchy::engine::l5::finding::{project_r4_findings, R4FindingsProjection};
use serde::Deserialize;
use serde_json::Value;

const R4_TEST_NAME: &str = "differential_r4_findings_match_goldens";

/// A smoke entry — the wave representative fixtures (one per substrate wave).
/// These are kept from R4-0 for continuity; the per-detector fixtures below are the
/// actual byte-match entries for R4-A.
///
/// `corpus_dir` — when `Some`, the corpus directory to run against (may differ from
/// the golden name / `fixture`). Used when one workspace produces multiple per-detector
/// goldens (e.g. d9 reuses the `ws-d8-commit-in-tx` corpus but has its own golden).
/// When `None` the corpus directory equals `fixture`.
struct Smoke {
    fixture: &'static str,
    wave: &'static str,
    detectors: &'static [&'static str],
    ported: bool,
    /// Optional: the corpus dir to run. Defaults to `fixture` when `None`.
    corpus_dir: Option<&'static str>,
}

/// The wave-representative smoke set (one per substrate wave).
const SMOKE: &[Smoke] = &[
    Smoke {
        fixture: "ws-d4-repeated-get",
        wave: "R4-A",
        detectors: &["d4-repeated-lookup-in-loop"],
        ported: true,
        corpus_dir: None,
    },
    Smoke {
        fixture: "ws-d22",
        wave: "R4-B",
        detectors: &["d22-flowfield-without-calcfields"],
        ported: true,
        corpus_dir: None,
    },
    Smoke {
        fixture: "ws-d12-dead-event",
        wave: "R4-C",
        detectors: &["d12-dead-integration-event"],
        ported: true,
        corpus_dir: None,
    },
    Smoke {
        fixture: "ws-d8-commit-in-tx",
        wave: "R4-D",
        detectors: &["d8-commit-in-transaction"],
        ported: true,
        corpus_dir: None,
    },
    Smoke {
        fixture: "ws-d3",
        wave: "R4-E",
        detectors: &["d3-missing-setloadfields"],
        ported: true,
        corpus_dir: None,
    },
    Smoke {
        fixture: "ws-txn-d47-pos-http-nocommit",
        wave: "R4-F",
        detectors: &["d47-io-unsafe-txn"],
        ported: false,
        corpus_dir: None,
    },
    Smoke {
        fixture: "ws-d14-dead-routine",
        wave: "R4-G",
        detectors: &["d14-dead-routine"],
        ported: false,
        corpus_dir: None,
    },
];

/// R4-A per-detector positive fixtures (ported: true = byte-match required).
const WAVE_A: &[Smoke] = &[
    // d5: loop-and-Modify could be ModifyAll
    Smoke {
        fixture: "ws-d5-modifyall",
        wave: "R4-A",
        detectors: &["d5-set-based-opportunity"],
        ported: true,
        corpus_dir: None,
    },
    // d10: self-modifying loop
    Smoke {
        fixture: "ws-d10-self-mod",
        wave: "R4-A",
        detectors: &["d10-self-modifying-loop"],
        ported: true,
        corpus_dir: None,
    },
    // d11: modify without get (no-get positive)
    Smoke {
        fixture: "ws-d11-no-get",
        wave: "R4-A",
        detectors: &["d11-modify-without-get"],
        ported: true,
        corpus_dir: None,
    },
    // d11: modify without get (modifyall+init, second positive fixture)
    Smoke {
        fixture: "ws-d11-modifyall-init",
        wave: "R4-A",
        detectors: &["d11-modify-without-get"],
        ported: true,
        corpus_dir: None,
    },
    // d18: constant filter in loop
    Smoke {
        fixture: "ws-d18",
        wave: "R4-A",
        detectors: &["d18-constant-filter-in-loop"],
        ported: true,
        corpus_dir: None,
    },
    // d21: read without load
    Smoke {
        fixture: "ws-d21",
        wave: "R4-A",
        detectors: &["d21-read-without-load"],
        ported: true,
        corpus_dir: None,
    },
    // d36: SetLoadFields placed after the load
    Smoke {
        fixture: "ws-d36",
        wave: "R4-A",
        detectors: &["d36-late-setloadfields"],
        ported: true,
        corpus_dir: None,
    },
    // d19: unused procedure parameter
    Smoke {
        fixture: "ws-d19",
        wave: "R4-A",
        detectors: &["d19-unused-parameter"],
        ported: true,
        corpus_dir: None,
    },
    // d20: unreachable statement after unconditional exit
    Smoke {
        fixture: "ws-d20",
        wave: "R4-A",
        detectors: &["d20-unreachable-after-exit"],
        ported: true,
        corpus_dir: None,
    },
    // d29: event subscriber mutates the inbound record
    Smoke {
        fixture: "ws-d29",
        wave: "R4-A",
        detectors: &["d29-subscriber-modify-on-event-record"],
        ported: true,
        corpus_dir: None,
    },
];

/// R4-B per-detector positive fixtures (metadata — tableById field metadata).
const WAVE_B: &[Smoke] = &[
    // d33: unfiltered DeleteAll / ModifyAll
    Smoke {
        fixture: "ws-d33",
        wave: "R4-B",
        detectors: &["d33-unfiltered-bulk-write"],
        ported: true,
        corpus_dir: None,
    },
];

/// R4-C per-detector positive fixtures (event/call-graph — eventGraph edges +
/// combined-graph SCC). d12's smoke entry already byte-matches above; d7/d38 are the
/// new per-detector entries.
const WAVE_C: &[Smoke] = &[
    // d7: event-subscriber chain forms a cycle (combined-graph SCC + event-dispatch).
    Smoke {
        fixture: "ws-d7-event-cycle",
        wave: "R4-C",
        detectors: &["d7-recursive-event-expansion"],
        ported: true,
        corpus_dir: None,
    },
    // d38: primary subscriber bound to an [Obsolete] publisher event (2 findings).
    Smoke {
        fixture: "ws-d38",
        wave: "R4-C",
        detectors: &["d38-subscriber-to-obsolete-event"],
        ported: true,
        corpus_dir: None,
    },
];

/// R4-D per-detector positive fixtures (transaction-span / commit-reachability).
/// d8's smoke entry already byte-matches above; d9/d34/d35 are the new per-detector
/// entries. d9 reuses the ws-d8-commit-in-tx corpus (same workspace, different golden).
const WAVE_D: &[Smoke] = &[
    // d9: transaction span summary (info-level; reuses d8's corpus dir).
    Smoke {
        fixture: "ws-d8-commit-in-tx-d9",
        wave: "R4-D",
        detectors: &["d9-transaction-span-summary"],
        ported: true,
        corpus_dir: Some("ws-d8-commit-in-tx"),
    },
    // d34: Commit inside a loop (direct + transitive; 3 findings).
    Smoke {
        fixture: "ws-d34",
        wave: "R4-D",
        detectors: &["d34-commit-in-loop"],
        ported: true,
        corpus_dir: None,
    },
    // d35: Commit reachable from event subscriber (direct + transitive; 2 findings).
    Smoke {
        fixture: "ws-d35",
        wave: "R4-D",
        detectors: &["d35-commit-in-event-subscriber"],
        ported: true,
        corpus_dir: None,
    },
    // d32: Boolean parameter always passed the same literal (1 finding).
    Smoke {
        fixture: "ws-d32",
        wave: "R4-D",
        detectors: &["d32-constant-boolean-parameter"],
        ported: true,
        corpus_dir: None,
    },
];

/// R4-E per-detector positive fixtures (record-flow — parameterRoles). ws-d3's smoke
/// entry already byte-matches above; d37/d39 are the new per-detector entries.
/// d37: ws-d37 (3 findings — Validate without persist). d39: ws-d39 (1 finding —
/// record left dirty across a call chain via the reverse call graph).
const WAVE_E: &[Smoke] = &[
    Smoke {
        fixture: "ws-d37",
        wave: "R4-E",
        detectors: &["d37-validate-without-persist"],
        ported: true,
        corpus_dir: None,
    },
    Smoke {
        fixture: "ws-d39",
        wave: "R4-E",
        detectors: &["d39-record-left-dirty-across-chain"],
        ported: true,
        corpus_dir: None,
    },
];

/// R4-E2 per-detector positive fixtures (transitive record-flow — parameterRoles +
/// resolved call edge + the source-ordered load/filter scan). d40: ws-d40 (2 findings —
/// medium read + high mutate, OPT-IN detector). d41: ws-d41 (1 finding — filter lost
/// across a Reset helper). d42: ws-d42 (1 finding — narrowed load misses the field the
/// callee reads). Each byte-matches END-TO-END.
const WAVE_E2: &[Smoke] = &[
    Smoke {
        fixture: "ws-d40",
        wave: "R4-E2",
        detectors: &["d40-transitive-load-missing"],
        ported: true,
        corpus_dir: None,
    },
    Smoke {
        fixture: "ws-d41",
        wave: "R4-E2",
        detectors: &["d41-transitive-filter-loss"],
        ported: true,
        corpus_dir: None,
    },
    Smoke {
        fixture: "ws-d42",
        wave: "R4-E2",
        detectors: &["d42-cross-call-wrong-setloadfields"],
        ported: true,
        corpus_dir: None,
    },
];

/// D1 per-detector positive fixtures (path-walker substrate end-to-end). d1 is the
/// MOST COMPLEX detector — its byte-match validates `walk_evidence` +
/// `merge_by_terminal` + `describe_table` + `pick_actionable_anchor` together.
/// ws-d1 (3 findings, one WITH additionalPaths), ws-d1-multi-caller (1 finding, 2
/// additionalPaths — validates merge_by_terminal), ws-d1-setup-singleton (3).
/// ws-d1-dep-terminal (0 findings — the explicit negative) is byte-matched
/// separately below (its 0-count golden is exempt from the anti-degenerate ≥1
/// check).
const WAVE_D1: &[Smoke] = &[
    Smoke {
        fixture: "ws-d1",
        wave: "R4-D1",
        detectors: &["d1-db-op-in-loop"],
        ported: true,
        corpus_dir: None,
    },
    Smoke {
        fixture: "ws-d1-multi-caller",
        wave: "R4-D1",
        detectors: &["d1-db-op-in-loop"],
        ported: true,
        corpus_dir: None,
    },
    Smoke {
        fixture: "ws-d1-setup-singleton",
        wave: "R4-D1",
        detectors: &["d1-db-op-in-loop"],
        ported: true,
        corpus_dir: None,
    },
];

/// D2 per-detector positive fixture (event-fanout-in-loop — the complement of d1).
/// ws-d2 (1 finding, WITH additionalPaths — two loops publish the same event, folded
/// by merge_by_terminal). Validates the event-dispatch-following WalkPolicy + the
/// subscriber DB-effect terminal + merge_by_terminal on rootCauseKey `d2/{eventId}`.
const WAVE_D2: &[Smoke] = &[Smoke {
    fixture: "ws-d2",
    wave: "R4-D2",
    detectors: &["d2-event-fanout-in-loop"],
    ported: true,
    corpus_dir: None,
}];

/// D48 per-detector positive fixture (http/file IO inside a loop). ws-txn-d48-pos
/// (2 findings: one transitive HTTP Send via a call chain, one direct FILE IO).
/// Validates the capability-fact IoTerminal (resource_kind http/file, witness
/// callsite, HttpExtra method) + the in-loop call-chain walk.
const WAVE_D48: &[Smoke] = &[Smoke {
    fixture: "ws-txn-d48-pos",
    wave: "R4-D48",
    detectors: &["d48-io-in-loop"],
    ported: true,
    corpus_dir: None,
}];

/// ws-txn-d48-neg: the explicit D48 negative — a 0-finding workspace (http/file IO
/// present but NOT inside any loop) with a committed 0-count golden. Byte-matched
/// END-TO-END but EXEMPT from the anti-degenerate ≥1 check.
const WAVE_D48_NEGATIVE: Smoke = Smoke {
    fixture: "ws-txn-d48-neg",
    wave: "R4-D48",
    detectors: &["d48-io-in-loop"],
    ported: true,
    corpus_dir: None,
};

/// R4-EVENT per-detector positive fixtures (event-flow substrate: d43/d44/d45).
/// d43: 6 fixtures totaling 7 findings (ws-event-ishandled = 1, -conditional-set = ?,
/// -exit, -helper, -multi-dispatch, -nested-guard). d44: 2 fixtures totaling 2
/// (ws-event-multi-sub-overlap = 1, ws-event-read-after-write = 1). d45: 2 fixtures
/// totaling 3 (ws-event-d45-deep = 2, ws-event-transitive-exposure = 1). Each
/// byte-matches END-TO-END.
const WAVE_R4_EVENT: &[Smoke] = &[
    // --- d43: event-ishandled-skip (6 fixtures) ---
    Smoke {
        fixture: "ws-event-ishandled",
        wave: "R4-EVENT",
        detectors: &["d43-event-ishandled-skip"],
        ported: true,
        corpus_dir: None,
    },
    Smoke {
        fixture: "ws-event-ishandled-conditional-set",
        wave: "R4-EVENT",
        detectors: &["d43-event-ishandled-skip"],
        ported: true,
        corpus_dir: None,
    },
    Smoke {
        fixture: "ws-event-ishandled-exit",
        wave: "R4-EVENT",
        detectors: &["d43-event-ishandled-skip"],
        ported: true,
        corpus_dir: None,
    },
    Smoke {
        fixture: "ws-event-ishandled-helper",
        wave: "R4-EVENT",
        detectors: &["d43-event-ishandled-skip"],
        ported: true,
        corpus_dir: None,
    },
    Smoke {
        fixture: "ws-event-ishandled-multi-dispatch",
        wave: "R4-EVENT",
        detectors: &["d43-event-ishandled-skip"],
        ported: true,
        corpus_dir: None,
    },
    Smoke {
        fixture: "ws-event-ishandled-nested-guard",
        wave: "R4-EVENT",
        detectors: &["d43-event-ishandled-skip"],
        ported: true,
        corpus_dir: None,
    },
    // --- d44: event-multi-subscriber-overlap (2 fixtures) ---
    Smoke {
        fixture: "ws-event-multi-sub-overlap",
        wave: "R4-EVENT",
        detectors: &["d44-event-multi-subscriber-overlap"],
        ported: true,
        corpus_dir: None,
    },
    Smoke {
        fixture: "ws-event-read-after-write",
        wave: "R4-EVENT",
        detectors: &["d44-event-multi-subscriber-overlap"],
        ported: true,
        corpus_dir: None,
    },
    // --- d45: event-transitive-table-exposure (2 fixtures, 3 findings) ---
    Smoke {
        fixture: "ws-event-d45-deep",
        wave: "R4-EVENT",
        detectors: &["d45-event-transitive-table-exposure"],
        ported: true,
        corpus_dir: None,
    },
    Smoke {
        fixture: "ws-event-transitive-exposure",
        wave: "R4-EVENT",
        detectors: &["d45-event-transitive-table-exposure"],
        ported: true,
        corpus_dir: None,
    },
];

/// ws-d1-dep-terminal: the explicit D1 negative — a 0-finding workspace with a
/// committed 0-count golden. Byte-matched END-TO-END (so the projection envelope's
/// `findingCount: 0` shape is asserted) but EXEMPT from the anti-degenerate ≥1
/// check that the positive fixtures must satisfy.
const WAVE_D1_NEGATIVE: Smoke = Smoke {
    fixture: "ws-d1-dep-terminal",
    wave: "R4-D1",
    detectors: &["d1-db-op-in-loop"],
    ported: true,
    corpus_dir: None,
};

/// A negative assertion: the given detector must produce 0 findings over the
/// neutral fixture (which lacks the detector's triggering pattern).
struct NegativeAssertion {
    detector: &'static str,
    neutral_fixture: &'static str,
}

/// One neutral per detector — pick a corpus fixture that CONTAINS the detector's
/// op-family but in a safe arrangement, so the detector genuinely runs its logic and
/// emits 0. The per-detector rationale is given below.
const NEGATIVES: &[NegativeAssertion] = &[
    // d5: ws-d18 has for-loops (no repeat..until / Next()) and SetRange/SetFilter —
    // but no Modify inside any loop, so the loop-and-Modify pattern never fires.
    NegativeAssertion {
        detector: "d5-set-based-opportunity",
        neutral_fixture: "ws-d18",
    },
    // d10: ws-d36 has Get/SetLoadFields ops but no loops at all — the self-modifying
    // loop detector requires Next() inside a repeat..until body and finds none.
    NegativeAssertion {
        detector: "d10-self-modifying-loop",
        neutral_fixture: "ws-d36",
    },
    // d11: ws-d10-self-mod has Modify on Customer but FindSet() (a LOAD_OP) precedes
    // every Modify — the "loaded before" check suppresses all instances.
    NegativeAssertion {
        detector: "d11-modify-without-get",
        neutral_fixture: "ws-d10-self-mod",
    },
    // d18: ws-d5-modifyall has SetRange outside the loop only; no SetRange/SetFilter
    // appears inside any loop body, so the constant-filter pattern never triggers.
    NegativeAssertion {
        detector: "d18-constant-filter-in-loop",
        neutral_fixture: "ws-d5-modifyall",
    },
    // d21: ws-d36 contains Get/SetLoadFields but no TestField/CalcFields/CalcSums —
    // the reading ops that d21 checks for are entirely absent.
    NegativeAssertion {
        detector: "d21-read-without-load",
        neutral_fixture: "ws-d36",
    },
    // d36: ws-d18 has SetRange/SetFilter inside loops and FindFirst — but no
    // SetLoadFields or AddLoadFields appear anywhere, so the late-setloadfields
    // pattern is never exercised.
    NegativeAssertion {
        detector: "d36-late-setloadfields",
        neutral_fixture: "ws-d18",
    },
    // d19: ws-d11-no-get has a procedure `FromParam(var Customer: Record Customer)`
    // whose record parameter IS referenced (Customer.Modify()) — the detector runs
    // its identifier-reference check on a parameterized procedure and finds every
    // param used, so it emits 0.
    NegativeAssertion {
        detector: "d19-unused-parameter",
        neutral_fixture: "ws-d11-no-get",
    },
    // d20: ws-d36 has Get/SetLoadFields procedures but no Exit/Error/CurrReport.Quit
    // followed by a statement — the body DFS records no unreachable pair, so 0.
    NegativeAssertion {
        detector: "d20-unreachable-after-exit",
        neutral_fixture: "ws-d36",
    },
    // d29: ws-d11-no-get contains Modify on a record (the MUTATING_OPS family) but
    // has NO [EventSubscriber] routine at all — the subscriber-kind gate suppresses
    // every candidate, so 0.
    NegativeAssertion {
        detector: "d29-subscriber-modify-on-event-record",
        neutral_fixture: "ws-d11-no-get",
    },
    // d22: ws-d11-no-get has field accesses on "No." and "Last Date Modified" (Normal
    // fields), but neither field is declared FlowField — the fieldClass gate drops all
    // accesses and the detector emits 0.
    NegativeAssertion {
        detector: "d22-flowfield-without-calcfields",
        neutral_fixture: "ws-d11-no-get",
    },
    // d33: ws-d22 contains Get/CalcFields ops but NO DeleteAll or ModifyAll
    // calls — the bulk-op gate never fires, so the detector emits 0.
    NegativeAssertion {
        detector: "d33-unfiltered-bulk-write",
        neutral_fixture: "ws-d22",
    },
    // d7: ws-d38 has event-dispatch edges (publisher → subscriber) but the
    // subscribers don't re-publish, so the combined graph has NO SCC of size >= 2 —
    // Tarjan finds only singletons and the detector emits 0.
    NegativeAssertion {
        detector: "d7-recursive-event-expansion",
        neutral_fixture: "ws-d38",
    },
    // d12: ws-d38 declares three [IntegrationEvent] publishers, but EACH has at least
    // one [EventSubscriber] in the workspace — subsByEvent > 0 for every event, so the
    // dead-event gate never fires and the detector emits 0.
    NegativeAssertion {
        detector: "d12-dead-integration-event",
        neutral_fixture: "ws-d38",
    },
    // d38: ws-d12-dead-event has an [IntegrationEvent] publisher but NO subscriber at
    // all — there are no resolved edges to walk, so the obsolete-publisher check is
    // never reached and the detector emits 0.
    NegativeAssertion {
        detector: "d38-subscriber-to-obsolete-event",
        neutral_fixture: "ws-d12-dead-event",
    },
    // d8: ws-d4-repeated-get has no Commit operations at all — no transaction spans
    // are seeded, so the posting-span check never fires and the detector emits 0.
    NegativeAssertion {
        detector: "d8-commit-in-transaction",
        neutral_fixture: "ws-d4-repeated-get",
    },
    // d9: ws-d4-repeated-get has no transaction spans (no Commit) — the
    // span-summary check never fires and the detector emits 0.
    NegativeAssertion {
        detector: "d9-transaction-span-summary",
        neutral_fixture: "ws-d4-repeated-get",
    },
    // d34: ws-d35 has a Commit inside an event subscriber but that subscriber's body
    // has no loop at all — the in-loop gate suppresses every commit operation, so 0.
    NegativeAssertion {
        detector: "d34-commit-in-loop",
        neutral_fixture: "ws-d35",
    },
    // d35: ws-d34 has Commits inside loops and a Persist callee that commits, but NO
    // [EventSubscriber] routine — the subscriber-kind gate suppresses every
    // candidate, so 0.
    NegativeAssertion {
        detector: "d35-commit-in-event-subscriber",
        neutral_fixture: "ws-d34",
    },
    // d32: ws-d19 has a `local` event-subscriber (kind == "event-subscriber", not
    // "procedure") and public procedures with no Boolean params — the kind gate and
    // the boolean-param gate both suppress every candidate, so 0.
    NegativeAssertion {
        detector: "d32-constant-boolean-parameter",
        neutral_fixture: "ws-d19",
    },
    // d2: ws-d4-repeated-get has a repeat..until loop with in-loop Get ops but NO
    // event publish inside any loop — the publisher-routine gate on the in-loop
    // callsite never matches, so the event-fanout detector emits 0.
    NegativeAssertion {
        detector: "d2-event-fanout-in-loop",
        neutral_fixture: "ws-d4-repeated-get",
    },
    // d43: ws-event-multi-sub-overlap has event subscribers but NO IsHandled dispatch
    // guard anywhere (no conditionReferences) — the substrate guard bails (or, even
    // reaching the site loop, no dispatch site has a post-call IsHandled guard), so 0.
    NegativeAssertion {
        detector: "d43-event-ishandled-skip",
        neutral_fixture: "ws-event-multi-sub-overlap",
    },
    // d44: ws-event-transitive-exposure has a single subscriber per event (no two
    // subscribers writing the same table, no reader-after-writer) — the overlap and
    // read-after-write gates never fire, so 0.
    NegativeAssertion {
        detector: "d44-event-multi-subscriber-overlap",
        neutral_fixture: "ws-event-transitive-exposure",
    },
    // d45: ws-event-ishandled has an event publisher whose subscriber sets IsHandled
    // but writes NO table — the transitive-write aggregation finds no writer-subscriber
    // table, so the per-(publisher,table) loop never fires and the detector emits 0.
    NegativeAssertion {
        detector: "d45-event-transitive-table-exposure",
        neutral_fixture: "ws-event-ishandled",
    },
    // d3: ws-d36 has Get/FindSet retrievals WITH SetLoadFields ops — deriveLoadStates
    // runs the full load-state machine, but every retrieval's loaded set covers its
    // accessed fields (no uncovered same-routine field access in any window), so the
    // missing/incomplete determination is silent and the detector emits 0.
    NegativeAssertion {
        detector: "d3-missing-setloadfields",
        neutral_fixture: "ws-d36",
    },
    // d37: ws-d39 contains a Validate op, but it is on a BY-VAR PARAMETER record
    // (ValidatesAndExits validates its `var Customer` parameter) — the parameter-record
    // suppression fires (the caller may persist after returning), so the detector
    // exercises its gate and emits 0.
    NegativeAssertion {
        detector: "d37-validate-without-persist",
        neutral_fixture: "ws-d39",
    },
    // d39: ws-d37 has Validate-without-persist routines (so parameterRoles are
    // computed), but NONE forward a Validate-dirty record by-var across a call chain
    // to a primary callee with dirtyAtExit === "yes" — the reverse-call-graph walk
    // finds no qualifying caller/callee pair, so the detector emits 0.
    NegativeAssertion {
        detector: "d39-record-left-dirty-across-chain",
        neutral_fixture: "ws-d37",
    },
    // d40: ws-d41 forwards records by-var to ResettingHelper, but that callee resets
    // filters — it does NOT require its parameter loaded at entry
    // (requiresLoadedAtEntry !== "yes") — so the transitive-load gate never fires and
    // the detector emits 0.
    NegativeAssertion {
        detector: "d40-transitive-load-missing",
        neutral_fixture: "ws-d41",
    },
    // d41: ws-d40 forwards records by-var to MutatingHelper / ReadingHelper, but
    // neither callee calls Reset (resetsFiltersOnParam !== "yes") — the filter-loss
    // gate never fires, so the detector emits 0.
    NegativeAssertion {
        detector: "d41-transitive-filter-loss",
        neutral_fixture: "ws-d40",
    },
    // d42: ws-d40 forwards records that were never narrowed (no SetLoadFields /
    // AddLoadFields) — the caller-side load is "full", so the callerFull skip fires
    // for every binding and the detector emits 0.
    NegativeAssertion {
        detector: "d42-cross-call-wrong-setloadfields",
        neutral_fixture: "ws-d40",
    },
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn goldens_dir() -> PathBuf {
    repo_root().join("tests").join("r4-goldens")
}

fn corpus_dir() -> PathBuf {
    repo_root().join("tests").join("r0-corpus")
}

#[derive(Debug, Clone, Deserialize)]
struct AllowEntry {
    #[serde(default = "default_allow_test")]
    test: String,
    fixture: String,
    path: String,
    #[serde(default)]
    #[allow(dead_code)]
    reason: String,
    #[serde(default)]
    #[allow(dead_code)]
    expires: String,
}

fn default_allow_test() -> String {
    "differential_identity_subset_matches_goldens".to_string()
}

fn load_allowlist() -> Vec<AllowEntry> {
    let path = repo_root().join("KNOWN_DIVERGENCES.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("failed to parse {} as a JSON array: {e}", path.display()))
}

#[derive(Debug, Clone)]
struct Divergence {
    fixture: String,
    path: String,
    golden_value: String,
    rust_value: String,
}

fn compact(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| format!("{v:?}"))
}

/// Recursively diff two values POSITIONALLY (both sides already canonically sorted).
fn diff_value(fixture: &str, path: &str, golden: &Value, rust: &Value, out: &mut Vec<Divergence>) {
    match (golden, rust) {
        (Value::Object(g), Value::Object(r)) => {
            for (k, gv) in g {
                let child = format!("{path}.{k}");
                match r.get(k) {
                    Some(rv) => diff_value(fixture, &child, gv, rv, out),
                    None => out.push(Divergence {
                        fixture: fixture.to_string(),
                        path: format!("{child}:MISSING_IN_RUST"),
                        golden_value: compact(gv),
                        rust_value: "<absent>".to_string(),
                    }),
                }
            }
            for (k, rv) in r {
                if !g.contains_key(k) {
                    out.push(Divergence {
                        fixture: fixture.to_string(),
                        path: format!("{path}.{k}:EXTRA_IN_RUST"),
                        golden_value: "<absent>".to_string(),
                        rust_value: compact(rv),
                    });
                }
            }
        }
        (Value::Array(g), Value::Array(r)) => {
            if g.len() != r.len() {
                out.push(Divergence {
                    fixture: fixture.to_string(),
                    path: format!("{path}:LENGTH"),
                    golden_value: g.len().to_string(),
                    rust_value: r.len().to_string(),
                });
            }
            let n = g.len().min(r.len());
            for i in 0..n {
                diff_value(fixture, &format!("{path}[{i}]"), &g[i], &r[i], out);
            }
            for (i, gv) in g.iter().enumerate().skip(n) {
                out.push(Divergence {
                    fixture: fixture.to_string(),
                    path: format!("{path}[{i}]:MISSING_IN_RUST"),
                    golden_value: compact(gv),
                    rust_value: "<absent>".to_string(),
                });
            }
            for (i, rv) in r.iter().enumerate().skip(n) {
                out.push(Divergence {
                    fixture: fixture.to_string(),
                    path: format!("{path}[{i}]:EXTRA_IN_RUST"),
                    golden_value: "<absent>".to_string(),
                    rust_value: compact(rv),
                });
            }
        }
        _ => {
            if golden != rust {
                out.push(Divergence {
                    fixture: fixture.to_string(),
                    path: path.to_string(),
                    golden_value: compact(golden),
                    rust_value: compact(rust),
                });
            }
        }
    }
}

/// Run the Rust source-only L5 pass for one fixture over the REGISTERED detectors,
/// projecting the envelope with the given detector list (findings filtered to that set).
///
/// `golden_name` is the `fixtureName` stamped into the projection envelope (must match
/// the golden file's `fixtureName` field for byte parity). `source_dir` is the corpus
/// directory to actually parse — when a golden covers a subset of a larger workspace
/// (e.g. d9 reuses `ws-d8-commit-in-tx`), these two differ.
fn run_rust(golden_name: &str, source_dir: &str, detector_names: &[&str]) -> R4FindingsProjection {
    let fixture_dir = corpus_dir().join(source_dir);
    assert!(
        fixture_dir.is_dir(),
        "R4 golden for {golden_name} has no matching in-repo fixture at {} (offline corpus incomplete)",
        fixture_dir.display()
    );
    let names: Vec<String> = detector_names.iter().map(|s| s.to_string()).collect();
    let detectors = registered_detectors();
    match assemble_and_resolve_workspace_default(&fixture_dir) {
        Some(resolved) => project_r4_findings(&resolved, &detectors, golden_name, &names),
        None => R4FindingsProjection {
            fixture_name: golden_name.to_string(),
            detectors: names,
            finding_count: 0,
            findings: vec![],
        },
    }
}

/// Pretty-serialize + trailing newline — the exact on-disk golden form.
fn pretty_with_newline(proj: &R4FindingsProjection) -> String {
    let mut s = serde_json::to_string_pretty(proj).expect("serialize R4 projection");
    s.push('\n');
    s
}

/// The subset of a golden `Value`'s findings whose `detector` is in `names`.
fn finding_subset(golden: &Value, names: &BTreeSet<String>) -> Vec<Value> {
    golden
        .get("findings")
        .and_then(|f| f.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|f| {
                    f.get("detector")
                        .and_then(|d| d.as_str())
                        .map(|d| names.contains(d))
                        .unwrap_or(false)
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Run a single smoke entry through the acceptance gate or deferred path.
fn run_smoke_entry(
    smoke: &Smoke,
    registered_names: &BTreeSet<String>,
    all_divergences: &mut Vec<Divergence>,
) -> Option<(bool, usize)> {
    let golden_path = goldens_dir().join(format!("{}.r4.golden.json", smoke.fixture));
    assert!(
        golden_path.is_file(),
        "missing R4 golden: {}",
        golden_path.display()
    );
    let golden_text = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|e| panic!("read R4 golden {}: {e}", golden_path.display()));
    let golden_json: Value = serde_json::from_str(&golden_text)
        .unwrap_or_else(|e| panic!("R4 golden {} not valid JSON: {e}", golden_path.display()));
    // Shape guard.
    let _: R4FindingsProjection = serde_json::from_value(golden_json.clone())
        .unwrap_or_else(|e| panic!("R4 golden {} not R4FindingsProjection: {e}", smoke.fixture));

    let source_dir = smoke.corpus_dir.unwrap_or(smoke.fixture);
    let rust = run_rust(smoke.fixture, source_dir, smoke.detectors);

    if smoke.ported {
        let rust_text = pretty_with_newline(&rust);
        let byte_matched = rust_text == golden_text;
        if !byte_matched {
            let rust_json = serde_json::to_value(&rust).expect("rust → value");
            diff_value(smoke.fixture, "", &golden_json, &rust_json, all_divergences);
        }
        let count = rust.finding_count;
        assert_eq!(
            rust_text, golden_text,
            "R4 ACCEPTANCE GATE: {} ({}) did NOT byte-match its golden",
            smoke.fixture, smoke.wave
        );
        Some((byte_matched, count))
    } else {
        let golden_subset = finding_subset(&golden_json, registered_names);
        let rust_json = serde_json::to_value(&rust).expect("rust → value");
        let rust_subset = finding_subset(&rust_json, registered_names);
        diff_value(
            smoke.fixture,
            ".findings(registered-subset)",
            &Value::Array(golden_subset.clone()),
            &Value::Array(rust_subset.clone()),
            all_divergences,
        );
        eprintln!(
            "R4 {} ({}): deferred to wave {} — registered-subset findings: {} (golden subset: {})",
            smoke.fixture,
            smoke.wave,
            smoke.wave,
            rust_subset.len(),
            golden_subset.len(),
        );
        None
    }
}

#[test]
fn differential_r4_findings_match_goldens() {
    let allowlist: Vec<AllowEntry> = load_allowlist()
        .into_iter()
        .filter(|e| e.test == R4_TEST_NAME)
        .collect();

    let registered_names: BTreeSet<String> = registered_detectors()
        .iter()
        .map(|d| d.name.clone())
        .collect();

    let mut all_divergences: Vec<Divergence> = Vec::new();

    // Track per-fixture byte-match results for anti-degenerate checks.
    let mut ported_results: Vec<(&'static str, bool, usize)> = Vec::new();

    // --- Smoke entries (wave representatives) ----------------------------------
    for smoke in SMOKE {
        if let Some((matched, count)) =
            run_smoke_entry(smoke, &registered_names, &mut all_divergences)
        {
            ported_results.push((smoke.fixture, matched, count));
        }
    }

    // --- R4-A per-detector fixtures -------------------------------------------
    for smoke in WAVE_A {
        if let Some((matched, count)) =
            run_smoke_entry(smoke, &registered_names, &mut all_divergences)
        {
            ported_results.push((smoke.fixture, matched, count));
        }
    }

    // --- R4-B per-detector fixtures -------------------------------------------
    for smoke in WAVE_B {
        if let Some((matched, count)) =
            run_smoke_entry(smoke, &registered_names, &mut all_divergences)
        {
            ported_results.push((smoke.fixture, matched, count));
        }
    }

    // --- R4-C per-detector fixtures -------------------------------------------
    for smoke in WAVE_C {
        if let Some((matched, count)) =
            run_smoke_entry(smoke, &registered_names, &mut all_divergences)
        {
            ported_results.push((smoke.fixture, matched, count));
        }
    }

    // --- R4-D per-detector fixtures -------------------------------------------
    for smoke in WAVE_D {
        if let Some((matched, count)) =
            run_smoke_entry(smoke, &registered_names, &mut all_divergences)
        {
            ported_results.push((smoke.fixture, matched, count));
        }
    }

    // --- R4-E per-detector fixtures (record-flow — parameterRoles) ------------
    for smoke in WAVE_E {
        if let Some((matched, count)) =
            run_smoke_entry(smoke, &registered_names, &mut all_divergences)
        {
            ported_results.push((smoke.fixture, matched, count));
        }
    }

    // --- R4-E2 per-detector fixtures (transitive record-flow: d40/d41/d42) -----
    for smoke in WAVE_E2 {
        if let Some((matched, count)) =
            run_smoke_entry(smoke, &registered_names, &mut all_divergences)
        {
            ported_results.push((smoke.fixture, matched, count));
        }
    }

    // --- D1 per-detector fixtures (path-walker substrate) ---------------------
    for smoke in WAVE_D1 {
        if let Some((matched, count)) =
            run_smoke_entry(smoke, &registered_names, &mut all_divergences)
        {
            ported_results.push((smoke.fixture, matched, count));
        }
    }
    // The D1 explicit negative: byte-match its 0-count golden END-TO-END, but do
    // NOT push it into ported_results (it is exempt from the anti-degenerate ≥1).
    run_smoke_entry(&WAVE_D1_NEGATIVE, &registered_names, &mut all_divergences);

    // --- D2 per-detector fixture (event-fanout-in-loop) -----------------------
    for smoke in WAVE_D2 {
        if let Some((matched, count)) =
            run_smoke_entry(smoke, &registered_names, &mut all_divergences)
        {
            ported_results.push((smoke.fixture, matched, count));
        }
    }

    // --- D48 per-detector fixture (http/file IO in loop) ----------------------
    for smoke in WAVE_D48 {
        if let Some((matched, count)) =
            run_smoke_entry(smoke, &registered_names, &mut all_divergences)
        {
            ported_results.push((smoke.fixture, matched, count));
        }
    }
    // The D48 explicit negative: byte-match its 0-count golden END-TO-END, but do
    // NOT push it into ported_results (exempt from the anti-degenerate ≥1).
    run_smoke_entry(&WAVE_D48_NEGATIVE, &registered_names, &mut all_divergences);

    // --- R4-EVENT per-detector fixtures (event-flow substrate: d43/d44/d45) ----
    for smoke in WAVE_R4_EVENT {
        if let Some((matched, count)) =
            run_smoke_entry(smoke, &registered_names, &mut all_divergences)
        {
            ported_results.push((smoke.fixture, matched, count));
        }
    }

    // --- Anti-degenerate: all ported fixtures byte-matched AND had ≥1 finding -
    for (fixture, byte_matched, count) in &ported_results {
        assert!(
            *byte_matched,
            "R4 anti-degenerate: {fixture} did NOT byte-match (acceptance gate failed)"
        );
        assert!(
            *count >= 1,
            "R4 anti-degenerate: {fixture} produced {count} findings — expected ≥1"
        );
    }

    // --- Negative assertions: each new R4-A detector → 0 findings on neutral --
    for neg in NEGATIVES {
        let neutral_dir = corpus_dir().join(neg.neutral_fixture);
        assert!(
            neutral_dir.is_dir(),
            "R4 negative: neutral fixture {} not found",
            neg.neutral_fixture
        );
        let detectors = registered_detectors();
        let names = vec![neg.detector.to_string()];
        let rust = match assemble_and_resolve_workspace_default(&neutral_dir) {
            Some(resolved) => {
                project_r4_findings(&resolved, &detectors, neg.neutral_fixture, &names)
            }
            None => R4FindingsProjection {
                fixture_name: neg.neutral_fixture.to_string(),
                detectors: names.clone(),
                finding_count: 0,
                findings: vec![],
            },
        };
        assert_eq!(
            rust.finding_count, 0,
            "R4 negative assertion FAILED: detector {} produced {} finding(s) on neutral fixture {} \
             (expected 0)",
            neg.detector, rust.finding_count, neg.neutral_fixture
        );
        eprintln!(
            "R4 negative: {} on {} → {} findings (expected 0) ✓",
            neg.detector, neg.neutral_fixture, rust.finding_count
        );
    }

    // --- Allowlist gating ---------------------------------------------------
    all_divergences
        .sort_by(|a, b| (a.fixture.as_str(), &a.path).cmp(&(b.fixture.as_str(), &b.path)));
    let mut entry_used = vec![false; allowlist.len()];
    let mut undocumented: Vec<&Divergence> = Vec::new();
    for div in &all_divergences {
        let mut covered = false;
        for (i, entry) in allowlist.iter().enumerate() {
            if entry.fixture == div.fixture && entry.path == div.path {
                entry_used[i] = true;
                covered = true;
            }
        }
        if !covered {
            undocumented.push(div);
        }
    }
    let unused: Vec<&AllowEntry> = allowlist
        .iter()
        .enumerate()
        .filter(|(i, _)| !entry_used[*i])
        .map(|(_, e)| e)
        .collect();

    let mut failure = String::new();
    if !undocumented.is_empty() {
        failure.push_str(&format!(
            "\n{} UNDOCUMENTED R4 divergence(s) (not in KNOWN_DIVERGENCES.json, test={R4_TEST_NAME}):\n",
            undocumented.len()
        ));
        for d in &undocumented {
            failure.push_str(&format!(
                "  [{}] {}\n      golden = {}\n      rust   = {}\n",
                d.fixture, d.path, d.golden_value, d.rust_value
            ));
        }
    }
    if !unused.is_empty() {
        failure.push_str(&format!(
            "\n{} UNUSED R4 allowlist entr(y/ies) (no matching divergence this run):\n",
            unused.len()
        ));
        for e in &unused {
            failure.push_str(&format!(
                "  [{}] {}  (reason: {:?}, expires: {:?})\n",
                e.fixture, e.path, e.reason, e.expires
            ));
        }
    }

    assert!(
        failure.is_empty(),
        "R4 findings differential FAILED:{failure}"
    );

    eprintln!(
        "R4 differential: {} smoke + {} R4-A + {} R4-B + {} R4-C + {} R4-D + {} D1 wave fixture(s) \
         (+1 D1 negative); {} ported (all byte-matched); {} negatives passed; {} deferred; \
         allowlist consumed ({} entr(y/ies)).",
        SMOKE.len(),
        WAVE_A.len(),
        WAVE_B.len(),
        WAVE_C.len(),
        WAVE_D.len(),
        WAVE_D1.len(),
        ported_results.len(),
        NEGATIVES.len(),
        SMOKE.iter().filter(|s| !s.ported).count(),
        allowlist.len(),
    );
}

/// Refresh the R4 goldens + manifest from a local al-sem checkout. Gated on
/// `AL_SEM_DIR`; does NOT auto-commit. Mirrors `refresh_r3a5_goldens_from_al_sem`.
#[test]
#[ignore]
fn refresh_r4_goldens_from_al_sem() {
    let al_sem = match std::env::var("AL_SEM_DIR") {
        Ok(d) => PathBuf::from(d),
        Err(_) => {
            eprintln!("AL_SEM_DIR not set — skipping R4 refresh");
            return;
        }
    };
    let src = al_sem.join("scripts").join("r4-goldens");
    let dst = goldens_dir();
    std::fs::create_dir_all(&dst).expect("mk r4 goldens dir");
    let mut copied = 0usize;
    // Smoke + wave-A + wave-B + wave-C + wave-D fixtures
    let all_fixtures: Vec<&str> = SMOKE
        .iter()
        .chain(WAVE_A.iter())
        .chain(WAVE_B.iter())
        .chain(WAVE_C.iter())
        .chain(WAVE_D.iter())
        .chain(WAVE_E.iter())
        .chain(WAVE_E2.iter())
        .chain(WAVE_D1.iter())
        .chain(std::iter::once(&WAVE_D1_NEGATIVE))
        .chain(WAVE_D2.iter())
        .chain(WAVE_D48.iter())
        .chain(std::iter::once(&WAVE_D48_NEGATIVE))
        .map(|s| s.fixture)
        .collect();
    for fixture in all_fixtures {
        let name = format!("{fixture}.r4.golden.json");
        let s = src.join(&name);
        if s.exists() {
            std::fs::copy(&s, dst.join(&name)).unwrap_or_else(|e| panic!("copy {name}: {e}"));
            copied += 1;
        }
    }
    let manifest = src.join("manifest.json");
    if manifest.exists() {
        std::fs::copy(&manifest, dst.join("manifest.json")).expect("copy manifest");
    }
    eprintln!(
        "R4: refreshed {copied} golden(s) + manifest from {} → {}",
        src.display(),
        dst.display()
    );
}
