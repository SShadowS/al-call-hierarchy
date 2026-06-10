//! cli-c/c1 — Events fanout + chains differential tests.
//!
//! Each test:
//!   1. Runs the Rust `run_events_fanout` / `run_events_chains` pipeline with
//!      `deterministic = true` and `alsem_version = "cli-c-v1"`.
//!   2. Compares the output byte-for-byte to the committed golden files under
//!      `tests/cli-c-events-goldens/`.
//!
//! Golden files are committed from al-sem `scripts/cli-c-goldens/events/`.
//! The `--ignore` shells re-run `bun run scripts/dump-events.ts` to refresh them.
//!
//! ## Coverage
//!   - 12 fixtures × 4 files = 48 base goldens (fanout.human + fanout.json +
//!     chains.human + chains.json)
//!   - 1 depth-truncation fixture × 2 files (chains.human + chains.json) = 2
//!   - 1 scope-all fixture × 4 files = 4
//!   - Total: 54 golden files
//!
//! ## Cycle native oracle
//!   - A mocked `EventGraph` with a mutual-publish cycle exercises `cycleDetected: true`
//!     rendering — unreachable via real AL source (see manifest.json cycleNote).

use std::path::PathBuf;

use al_call_hierarchy::engine::gate::events::{
    format_chains_human, format_chains_json, format_fanout_human, format_fanout_json,
    run_events_chains, run_events_fanout, EventsChainsOptions, EventsFanoutOptions,
};
use al_call_hierarchy::engine::l5::event_flow::Scope;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("r0-corpus")
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("cli-c-events-goldens")
}

fn load_golden(name: &str) -> String {
    let path = golden_dir().join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read golden {}: {e}", path.display()))
}

const ALSEM_VERSION: &str = "cli-c-v1";

// ---------------------------------------------------------------------------
// Macro to generate fanout + chains differential tests for each fixture.
// ---------------------------------------------------------------------------

macro_rules! events_differential {
    ($test_name:ident, $fixture:literal, $scope:expr) => {
        #[test]
        fn $test_name() {
            let ws = corpus_dir().join($fixture);
            assert!(ws.is_dir(), "fixture missing: {}", ws.display());

            let fixture_stem = $fixture;

            // ── fanout human ─────────────────────────────────────────────
            {
                let opts = EventsFanoutOptions {
                    workspace: &ws,
                    format: "human",
                    scope: $scope,
                    alsem_version: ALSEM_VERSION,
                    deterministic: true,
                    strict: false,
                };
                let result = run_events_fanout(&opts);
                let golden = load_golden(&format!("{fixture_stem}.fanout.human.txt"));
                assert_eq!(result.text, golden, "{fixture_stem} fanout.human mismatch");
                assert_eq!(result.exit_code, 0, "{fixture_stem} fanout.human exit_code");
            }

            // ── fanout json ──────────────────────────────────────────────
            {
                let opts = EventsFanoutOptions {
                    workspace: &ws,
                    format: "json",
                    scope: $scope,
                    alsem_version: ALSEM_VERSION,
                    deterministic: true,
                    strict: false,
                };
                let result = run_events_fanout(&opts);
                let golden = load_golden(&format!("{fixture_stem}.fanout.json"));
                assert_eq!(result.text, golden, "{fixture_stem} fanout.json mismatch");
                assert_eq!(result.exit_code, 0, "{fixture_stem} fanout.json exit_code");
            }

            // ── chains human ─────────────────────────────────────────────
            {
                let opts = EventsChainsOptions {
                    workspace: &ws,
                    format: "human",
                    scope: $scope,
                    max_depth: None,
                    max_nodes: None,
                    alsem_version: ALSEM_VERSION,
                    deterministic: true,
                    strict: false,
                };
                let result = run_events_chains(&opts);
                let golden = load_golden(&format!("{fixture_stem}.chains.human.txt"));
                assert_eq!(result.text, golden, "{fixture_stem} chains.human mismatch");
                assert_eq!(result.exit_code, 0, "{fixture_stem} chains.human exit_code");
            }

            // ── chains json ──────────────────────────────────────────────
            {
                let opts = EventsChainsOptions {
                    workspace: &ws,
                    format: "json",
                    scope: $scope,
                    max_depth: None,
                    max_nodes: None,
                    alsem_version: ALSEM_VERSION,
                    deterministic: true,
                    strict: false,
                };
                let result = run_events_chains(&opts);
                let golden = load_golden(&format!("{fixture_stem}.chains.json"));
                assert_eq!(result.text, golden, "{fixture_stem} chains.json mismatch");
                assert_eq!(result.exit_code, 0, "{fixture_stem} chains.json exit_code");
            }
        }
    };
}

// ── 12 base fixtures (scope=primary) ────────────────────────────────────────

events_differential!(events_ws_events, "ws-events", Scope::Primary);
events_differential!(events_ws_event_fanout, "ws-event-fanout", Scope::Primary);
events_differential!(events_ws_event_chains, "ws-event-chains", Scope::Primary);
events_differential!(
    events_ws_event_pub_cycle,
    "ws-event-pub-cycle",
    Scope::Primary
);
events_differential!(events_ws_event_cycle, "ws-event-cycle", Scope::Primary);
events_differential!(
    events_ws_event_ishandled,
    "ws-event-ishandled",
    Scope::Primary
);
events_differential!(
    events_ws_event_multi_sub_overlap,
    "ws-event-multi-sub-overlap",
    Scope::Primary
);
events_differential!(
    events_ws_event_read_after_write,
    "ws-event-read-after-write",
    Scope::Primary
);
events_differential!(
    events_ws_event_d45_deep,
    "ws-event-d45-deep",
    Scope::Primary
);
events_differential!(
    events_ws_d8_commit_in_tx,
    "ws-d8-commit-in-tx",
    Scope::Primary
);
events_differential!(
    events_ws_txn_d47_event_pos,
    "ws-txn-d47-event-pos",
    Scope::Primary
);
events_differential!(
    events_ws_d12_dead_event,
    "ws-d12-dead-event",
    Scope::Primary
);

// ── scope=all variant (ws-event-fanout) ─────────────────────────────────────

#[test]
fn events_ws_event_fanout_scope_all() {
    let ws = corpus_dir().join("ws-event-fanout");
    assert!(ws.is_dir(), "fixture missing: {}", ws.display());

    // fanout human
    {
        let opts = EventsFanoutOptions {
            workspace: &ws,
            format: "human",
            scope: Scope::All,
            alsem_version: ALSEM_VERSION,
            deterministic: true,
            strict: false,
        };
        let result = run_events_fanout(&opts);
        let golden = load_golden("ws-event-fanout.scope-all.fanout.human.txt");
        assert_eq!(result.text, golden, "scope-all fanout.human mismatch");
    }

    // fanout json
    {
        let opts = EventsFanoutOptions {
            workspace: &ws,
            format: "json",
            scope: Scope::All,
            alsem_version: ALSEM_VERSION,
            deterministic: true,
            strict: false,
        };
        let result = run_events_fanout(&opts);
        let golden = load_golden("ws-event-fanout.scope-all.fanout.json");
        assert_eq!(result.text, golden, "scope-all fanout.json mismatch");
    }

    // chains human
    {
        let opts = EventsChainsOptions {
            workspace: &ws,
            format: "human",
            scope: Scope::All,
            max_depth: None,
            max_nodes: None,
            alsem_version: ALSEM_VERSION,
            deterministic: true,
            strict: false,
        };
        let result = run_events_chains(&opts);
        let golden = load_golden("ws-event-fanout.scope-all.chains.human.txt");
        assert_eq!(result.text, golden, "scope-all chains.human mismatch");
    }

    // chains json
    {
        let opts = EventsChainsOptions {
            workspace: &ws,
            format: "json",
            scope: Scope::All,
            max_depth: None,
            max_nodes: None,
            alsem_version: ALSEM_VERSION,
            deterministic: true,
            strict: false,
        };
        let result = run_events_chains(&opts);
        let golden = load_golden("ws-event-fanout.scope-all.chains.json");
        assert_eq!(result.text, golden, "scope-all chains.json mismatch");
    }
}

// ── max-depth=1 variant (ws-event-d45-deep) ─────────────────────────────────

#[test]
fn events_ws_event_d45_deep_max_depth_1() {
    let ws = corpus_dir().join("ws-event-d45-deep");
    assert!(ws.is_dir(), "fixture missing: {}", ws.display());

    // chains human (max-depth=1)
    {
        let opts = EventsChainsOptions {
            workspace: &ws,
            format: "human",
            scope: Scope::Primary,
            max_depth: Some(1),
            max_nodes: None,
            alsem_version: ALSEM_VERSION,
            deterministic: true,
            strict: false,
        };
        let result = run_events_chains(&opts);
        let golden = load_golden("ws-event-d45-deep.max-depth-1.chains.human.txt");
        assert_eq!(result.text, golden, "max-depth-1 chains.human mismatch");
    }

    // chains json (max-depth=1)
    {
        let opts = EventsChainsOptions {
            workspace: &ws,
            format: "json",
            scope: Scope::Primary,
            max_depth: Some(1),
            max_nodes: None,
            alsem_version: ALSEM_VERSION,
            deterministic: true,
            strict: false,
        };
        let result = run_events_chains(&opts);
        let golden = load_golden("ws-event-d45-deep.max-depth-1.chains.json");
        assert_eq!(result.text, golden, "max-depth-1 chains.json mismatch");
    }
}

// ── Native oracle: cycle path ────────────────────────────────────────────────
//
// Exercises `cycleDetected: true` in the chain walk. Unreachable via real AL
// source because the routine-indexer assigns a single kind= (first-match wins),
// so a routine cannot be both [IntegrationEvent] publisher and EventSubscriber.
// Build the EventGraph directly (mirrors al-sem test/engine/event-flow.test.ts).

#[test]
fn cycle_native_oracle() {
    use al_call_hierarchy::engine::l3::event_graph::{
        EventEdge, EventGraph, EventSymbol, Evidence,
    };
    use al_call_hierarchy::engine::l5::event_flow::{
        build_event_flow_indexes, compute_chain_report, walk_event_chain, ChainNode,
        ChainWalkOptions, Scope,
    };
    use std::collections::BTreeSet;

    // Graph: P publishes E1 → S1 subscribes. S1 publishes E2 → P subscribes.
    let event_graph = EventGraph {
        events: vec![
            EventSymbol {
                id: "E1".to_string(),
                publisher_object_id: "app/Codeunit/1".to_string(),
                publisher_routine_id: Some("P".to_string()),
                publisher_stable_routine_id: Some("P".to_string()),
                event_name: "Ev1".to_string(),
                event_kind: "integration".to_string(),
                element_name: None,
                signature_hash: String::new(),
                parameters: Vec::new(),
                isolated: None,
                provenance: vec![Evidence {
                    source: "test".to_string(),
                    note: None,
                }],
            },
            EventSymbol {
                id: "E2".to_string(),
                publisher_object_id: "app/Codeunit/1".to_string(),
                publisher_routine_id: Some("S1".to_string()),
                publisher_stable_routine_id: Some("S1".to_string()),
                event_name: "Ev2".to_string(),
                event_kind: "integration".to_string(),
                element_name: None,
                signature_hash: String::new(),
                parameters: Vec::new(),
                isolated: None,
                provenance: vec![Evidence {
                    source: "test".to_string(),
                    note: None,
                }],
            },
        ],
        edges: vec![
            EventEdge {
                event_id: "E1".to_string(),
                subscriber_routine_id: "S1".to_string(),
                subscriber_stable_routine_id: "S1".to_string(),
                subscriber_app_id: "app".to_string(),
                resolution: "resolved".to_string(),
                provenance: vec![],
            },
            EventEdge {
                event_id: "E2".to_string(),
                subscriber_routine_id: "P".to_string(),
                subscriber_stable_routine_id: "P".to_string(),
                subscriber_app_id: "app".to_string(),
                resolution: "resolved".to_string(),
                provenance: vec![],
            },
        ],
    };

    let dep_ids: BTreeSet<String> = BTreeSet::new();
    let routines = Vec::new();
    let ix = build_event_flow_indexes(&event_graph, &routines, &dep_ids);

    // walk_event_chain from P must produce a cycle node.
    let tree = walk_event_chain("P", &ix, &ChainWalkOptions::default());

    fn collect_all(node: &ChainNode, out: &mut Vec<ChainNode>) {
        out.push(node.clone());
        for c in &node.children {
            collect_all(c, out);
        }
    }
    let mut nodes = Vec::new();
    collect_all(&tree, &mut nodes);

    let has_cycle = nodes.iter().any(|n| n.cycle_detected);
    assert!(
        has_cycle,
        "expected cycleDetected in walk from P over mutual-publish graph"
    );

    // The cycle node must be a subscriber node.
    let cycle_nodes: Vec<_> = nodes.iter().filter(|n| n.cycle_detected).collect();
    for cn in &cycle_nodes {
        assert_eq!(
            cn.kind, "subscriber",
            "cycle node must have kind=subscriber"
        );
    }

    // Human: must contain "  (cycle)".
    let report = compute_chain_report(&ix, &ChainWalkOptions::default(), Scope::All);
    let human = format_chains_human(&report);
    assert!(
        human.contains("  (cycle)"),
        "human must contain '  (cycle)'; got:\n{human}"
    );

    // JSON: must contain `"cycleDetected": true`.
    let json = format_chains_json(&report, "test-v1", true);
    assert!(
        json.contains("\"cycleDetected\": true"),
        "json must contain '\"cycleDetected\": true'; got:\n{json}"
    );

    // Summary: cyclesDetected >= 1.
    assert!(
        report.cycles_detected >= 1,
        "report.cycles_detected must be >= 1"
    );

    // Summary JSON field: "cyclesDetected" in summary block.
    assert!(
        json.contains("\"cyclesDetected\":"),
        "json summary must contain 'cyclesDetected'"
    );
}

// ── Refresh shell ────────────────────────────────────────────────────────────
//
// To regenerate the goldens, set AL_SEM_DIR to the al-sem repo root and run:
//   cargo test --test cli_c_events_differential refresh_goldens -- --ignored
//
// The shell invokes `bun run scripts/dump-events.ts` in the al-sem repo and
// copies the output into `tests/cli-c-events-goldens/`.

#[test]
#[ignore]
fn refresh_goldens() {
    let al_sem_dir = std::env::var("AL_SEM_DIR").unwrap_or_else(|_| "U:/Git/al-sem".to_string());
    let status = std::process::Command::new("bun")
        .arg("run")
        .arg("scripts/dump-events.ts")
        .current_dir(&al_sem_dir)
        .env("AL_SEM_VERSION_OVERRIDE", "cli-c-v1")
        .status()
        .expect("bun run scripts/dump-events.ts failed to launch");
    assert!(status.success(), "dump-events.ts failed");

    // Copy goldens from al-sem to engine.
    let src = PathBuf::from(&al_sem_dir).join("scripts/cli-c-goldens/events");
    let dst = golden_dir();
    let entries = std::fs::read_dir(&src).expect("read src dir");
    for entry in entries.flatten() {
        let p = entry.path();
        if let Some(name) = p.file_name() {
            std::fs::copy(&p, dst.join(name)).expect("copy golden");
        }
    }
    println!("Goldens refreshed from {}", src.display());
}
