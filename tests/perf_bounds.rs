//! CI perf-bounds gate (Task T0.5). Asserts the legacy LSP call-hierarchy
//! pipeline (`indexer.rs` / `graph.rs` / `handlers.rs` — the surface that
//! ships today) stays within generous (~3x) margins of the CLAUDE.md
//! Performance Targets table, so an order-of-magnitude regression fails CI
//! loudly. See `benches/lsp_pipeline.rs` for the finer-grained Criterion
//! measurements this gate is a coarse tripwire for.
//!
//! Compiled for real ONLY under `#[cfg(not(debug_assertions))]`: a
//! debug-build timing assert is meaningless (unoptimized code can run several
//! times slower than release, for reasons unrelated to any real regression),
//! so the actual bounds checks below only exist in a release build. This is
//! deliberate, NOT a silent-skip: CI explicitly invokes
//! `cargo test --release --test perf_bounds`, so it always runs in the
//! profile where the checks are compiled in. The always-present marker test
//! below has no `cfg` gate at all, so `cargo test --test perf_bounds` (any
//! profile) never silently reports zero tests — if this file's `mod` wiring
//! ever broke, this test failing to even show up would be caught immediately
//! by the "did the binary run any tests" question, not silently pass.
//!
//! Bounds are 3x each CLAUDE.md target (USER DECISION, binding — see
//! `.superpowers/sdd/t0-task-5-brief.md`): generous by design so occasional
//! flake on a loaded CI runner doesn't cause false failures, while a true
//! order-of-magnitude regression still trips the gate.

// Gated the same as `release_checks` below (not just its contents): in a
// debug build nothing would call into `perf_support`, and its `pub fn`s
// would report as dead code under `cargo clippy --all-targets -- -D
// warnings` (this test binary has no library surface of its own to export
// them through). `benches/lsp_pipeline.rs` still exercises `perf_support`
// unconditionally, so it stays fully linted either way.
#[cfg(not(debug_assertions))]
#[path = "perf_support/mod.rs"]
mod perf_support;

/// Always present regardless of build profile — guarantees `cargo test
/// --test perf_bounds` never silently reports 0 tests even if the
/// release-only module below fails to compile in.
#[test]
fn perf_bounds_binary_is_never_empty() {}

#[cfg(debug_assertions)]
#[allow(dead_code)]
/// Compile-time note (not a test): the real bounds checks live in
/// `release_checks` below and only compile under `#[cfg(not(debug_assertions))]`.
/// Run `cargo test --release --test perf_bounds` to exercise them.
const DEBUG_BUILD_SKIPS_REAL_PERF_BOUNDS: &str = "see module doc comment";

#[cfg(not(debug_assertions))]
mod release_checks {
    use super::perf_support;
    use al_call_hierarchy::handlers;
    use al_call_hierarchy::indexer::Indexer;
    use al_call_hierarchy::protocol::path_to_uri;
    use lsp_types::{
        CallHierarchyIncomingCallsParams, CallHierarchyItem, CallHierarchyOutgoingCallsParams,
        CallHierarchyPrepareParams, Position, SymbolKind, TextDocumentIdentifier,
        TextDocumentPositionParams,
    };
    use std::sync::{Arc, RwLock};
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    // 3x the CLAUDE.md target, per the binding T0.5 USER DECISION.
    const INDEX_100_BOUND: Duration = Duration::from_millis(1500); // target: 500ms
    const INDEX_1000_BOUND: Duration = Duration::from_millis(6000); // target: 2s
    const QUERY_BOUND: Duration = Duration::from_millis(3); // target: 1ms
    const FILE_CHANGE_BOUND: Duration = Duration::from_millis(150); // target: 50ms

    fn median(mut samples: Vec<Duration>) -> Duration {
        samples.sort();
        samples[samples.len() / 2]
    }

    #[test]
    fn initial_index_100_files_within_bound() {
        let dir = TempDir::new().unwrap();
        perf_support::generate_corpus(dir.path(), 100);

        // Warm-up: first pass pages the corpus into the OS file cache so the
        // timed runs measure indexing, not cold disk I/O.
        Indexer::new().index_directory(dir.path()).unwrap();

        let mut samples = Vec::with_capacity(3);
        for _ in 0..3 {
            let mut indexer = Indexer::new();
            let start = Instant::now();
            indexer.index_directory(dir.path()).unwrap();
            samples.push(start.elapsed());
        }
        let m = median(samples.clone());
        println!(
            "[perf_bounds] initial_index_100_files: median={m:?} bound={INDEX_100_BOUND:?} samples={samples:?}"
        );
        assert!(
            m <= INDEX_100_BOUND,
            "100-file initial index median {m:?} exceeds 3x-target bound {INDEX_100_BOUND:?} (samples: {samples:?})"
        );
    }

    #[test]
    fn initial_index_1000_files_within_bound() {
        let dir = TempDir::new().unwrap();
        perf_support::generate_corpus(dir.path(), 1000);

        Indexer::new().index_directory(dir.path()).unwrap();

        let mut samples = Vec::with_capacity(3);
        for _ in 0..3 {
            let mut indexer = Indexer::new();
            let start = Instant::now();
            indexer.index_directory(dir.path()).unwrap();
            samples.push(start.elapsed());
        }
        let m = median(samples.clone());
        println!(
            "[perf_bounds] initial_index_1000_files: median={m:?} bound={INDEX_1000_BOUND:?} samples={samples:?}"
        );
        assert!(
            m <= INDEX_1000_BOUND,
            "1000-file initial index median {m:?} exceeds 3x-target bound {INDEX_1000_BOUND:?} (samples: {samples:?})"
        );
    }

    /// Build a 1000-file indexed graph once, wrapped the way the real LSP
    /// server wraps it, for the 3 query-handler bounds checks below.
    fn build_indexed_1000() -> (TempDir, Arc<RwLock<Indexer>>) {
        let dir = TempDir::new().unwrap();
        perf_support::generate_corpus(dir.path(), 1000);
        let mut indexer = Indexer::new();
        indexer.index_directory(dir.path()).unwrap();
        (dir, Arc::new(RwLock::new(indexer)))
    }

    #[test]
    fn prepare_call_hierarchy_within_bound() {
        let (dir, indexer) = build_indexed_1000();
        let uri = path_to_uri(&dir.path().join(perf_support::file_name(1)));
        let make_params = || CallHierarchyPrepareParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                // Line 2 is `    procedure Proc0()` in generated file content;
                // character 15 lands inside the range covering that definition.
                position: Position {
                    line: 2,
                    character: 15,
                },
            },
            work_done_progress_params: Default::default(),
        };

        // Warm-up.
        let warm = handlers::prepare_call_hierarchy(&indexer, make_params()).unwrap();
        assert!(warm.is_some(), "sanity: warm-up must find a definition");

        let mut samples = Vec::with_capacity(5);
        for _ in 0..5 {
            let start = Instant::now();
            let result = handlers::prepare_call_hierarchy(&indexer, make_params()).unwrap();
            samples.push(start.elapsed());
            assert!(result.is_some(), "sanity: must find a definition");
        }
        let m = median(samples.clone());
        println!(
            "[perf_bounds] prepareCallHierarchy: median={m:?} bound={QUERY_BOUND:?} samples={samples:?}"
        );
        assert!(
            m <= QUERY_BOUND,
            "prepareCallHierarchy median {m:?} exceeds 3x-target bound {QUERY_BOUND:?} (samples: {samples:?})"
        );
    }

    #[test]
    fn incoming_calls_within_bound() {
        let (_dir, indexer) = build_indexed_1000();
        let make_params = || CallHierarchyIncomingCallsParams {
            item: CallHierarchyItem {
                name: "Proc0".to_string(),
                kind: SymbolKind::FUNCTION,
                tags: None,
                detail: None,
                uri: path_to_uri(std::path::Path::new("unused.al")),
                range: Default::default(),
                selection_range: Default::default(),
                data: Some(serde_json::json!({
                    "object": perf_support::object_name(perf_support::HUB_INDEX),
                    "procedure": "Proc0",
                })),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        handlers::incoming_calls(&indexer, make_params()).unwrap();

        let mut samples = Vec::with_capacity(5);
        for _ in 0..5 {
            let start = Instant::now();
            let result = handlers::incoming_calls(&indexer, make_params()).unwrap();
            samples.push(start.elapsed());
            assert_eq!(
                result.unwrap().len(),
                999,
                "sanity: hub Proc0 must show real fan-in (999 = 1000 files - 1 hub)"
            );
        }
        let m = median(samples.clone());
        println!(
            "[perf_bounds] incomingCalls: median={m:?} bound={QUERY_BOUND:?} samples={samples:?}"
        );
        assert!(
            m <= QUERY_BOUND,
            "incomingCalls median {m:?} exceeds 3x-target bound {QUERY_BOUND:?} (samples: {samples:?})"
        );
    }

    #[test]
    fn outgoing_calls_within_bound() {
        let (_dir, indexer) = build_indexed_1000();
        let make_params = || CallHierarchyOutgoingCallsParams {
            item: CallHierarchyItem {
                name: "Proc0".to_string(),
                kind: SymbolKind::FUNCTION,
                tags: None,
                detail: None,
                uri: path_to_uri(std::path::Path::new("unused.al")),
                range: Default::default(),
                selection_range: Default::default(),
                data: Some(serde_json::json!({
                    "object": perf_support::object_name(1),
                    "procedure": "Proc0",
                })),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        handlers::outgoing_calls(&indexer, make_params()).unwrap();

        let mut samples = Vec::with_capacity(5);
        for _ in 0..5 {
            let start = Instant::now();
            let result = handlers::outgoing_calls(&indexer, make_params()).unwrap();
            samples.push(start.elapsed());
            assert_eq!(
                result.unwrap().len(),
                3,
                "sanity: file-1 Proc0 must show real fan-out"
            );
        }
        let m = median(samples.clone());
        println!(
            "[perf_bounds] outgoingCalls: median={m:?} bound={QUERY_BOUND:?} samples={samples:?}"
        );
        assert!(
            m <= QUERY_BOUND,
            "outgoingCalls median {m:?} exceeds 3x-target bound {QUERY_BOUND:?} (samples: {samples:?})"
        );
    }

    #[test]
    fn single_file_reindex_within_bound() {
        let (dir, indexer_arc) = build_indexed_1000();
        let target = dir.path().join(perf_support::file_name(1));

        // Put "changed" content on disk once, outside the timed region — the
        // real didSave flow re-parses whatever is already on disk; the write
        // itself isn't part of what the LSP measures. Each timed call below
        // re-runs the same remove+reparse+re-add path on this changed file.
        perf_support::rewrite_with_extra_procedure(dir.path(), 1000, 1);

        // Warm-up (also proves the reindex path works before timing it).
        indexer_arc.write().unwrap().reindex_file(&target).unwrap();

        let mut samples = Vec::with_capacity(3);
        for _ in 0..3 {
            let start = Instant::now();
            indexer_arc.write().unwrap().reindex_file(&target).unwrap();
            samples.push(start.elapsed());
        }
        let m = median(samples.clone());
        println!(
            "[perf_bounds] single_file_reindex: median={m:?} bound={FILE_CHANGE_BOUND:?} samples={samples:?}"
        );
        assert!(
            m <= FILE_CHANGE_BOUND,
            "single-file reindex median {m:?} exceeds 3x-target bound {FILE_CHANGE_BOUND:?} (samples: {samples:?})"
        );
    }
}
