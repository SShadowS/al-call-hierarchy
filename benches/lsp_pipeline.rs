//! Criterion benches for the legacy LSP call-hierarchy pipeline (Task T0.5):
//! `indexer.rs` / `graph.rs` / `handlers.rs` — the surface that ships today
//! and the one CLAUDE.md's Performance Targets table describes. Measures:
//! initial index of 100/1000-file synthetic corpora, the 3 call-hierarchy
//! query handlers (`prepareCallHierarchy` / `incomingCalls` / `outgoingCalls`)
//! against a 1000-file indexed graph, and a single-file reindex — all
//! in-process, no LSP stdio loop.
//!
//! Run: `cargo bench --bench lsp_pipeline` (or `cargo bench` for every bench
//! target, including `telemetry_hot_path`). See `tests/perf_bounds.rs` for
//! the release-only CI gate these measurements feed into, and
//! `tests/perf_support/mod.rs` for the corpus shape/rationale (deterministic,
//! real cross-file fan-in/fan-out).

#[path = "../tests/perf_support/mod.rs"]
mod perf_support;

use al_call_hierarchy::handlers;
use al_call_hierarchy::indexer::Indexer;
use al_call_hierarchy::protocol::path_to_uri;
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use lsp_types::{
    CallHierarchyIncomingCallsParams, CallHierarchyItem, CallHierarchyOutgoingCallsParams,
    CallHierarchyPrepareParams, Position, SymbolKind, TextDocumentIdentifier,
    TextDocumentPositionParams,
};
use std::sync::{Arc, RwLock};
use tempfile::TempDir;

/// Initial index of the 100- and 1000-file synthetic corpora (CLAUDE.md
/// targets: <500ms / <2s). Corpus generation happens once, outside the timed
/// closure — only `Indexer::index_directory` is measured per iteration.
fn bench_initial_index(c: &mut Criterion) {
    let mut group = c.benchmark_group("initial_index");

    for &file_count in &[100usize, 1000] {
        let dir = TempDir::new().unwrap();
        perf_support::generate_corpus(dir.path(), file_count);
        group.bench_function(format!("{file_count}_files"), |b| {
            b.iter(|| {
                let mut indexer = Indexer::new();
                indexer.index_directory(black_box(dir.path())).unwrap();
                black_box(indexer.graph().definition_count());
            });
        });
    }
    group.finish();
}

/// The 3 call-hierarchy query handlers against a 1000-file indexed graph
/// (CLAUDE.md targets: all <1ms). Indexing happens once, outside the timed
/// closures — each iteration measures only the handler call (symbol lookup +
/// graph traversal + LSP-shape construction), matching how the real LSP
/// server serves a steady-state request.
fn bench_queries(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    perf_support::generate_corpus(dir.path(), 1000);
    let mut indexer = Indexer::new();
    indexer.index_directory(dir.path()).unwrap();
    let indexer = Arc::new(RwLock::new(indexer));

    let mut group = c.benchmark_group("query_handlers_1000_files");

    // prepareCallHierarchy: resolve a source position to a definition. Line 2
    // is `    procedure Proc0()` in generated file content (see
    // perf_support::codeunit_source); character 15 lands inside its range.
    let prepare_uri = path_to_uri(&dir.path().join(perf_support::file_name(1)));
    group.bench_function("prepareCallHierarchy", |b| {
        b.iter(|| {
            let params = CallHierarchyPrepareParams {
                text_document_position_params: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier {
                        uri: prepare_uri.clone(),
                    },
                    position: Position {
                        line: 2,
                        character: 15,
                    },
                },
                work_done_progress_params: Default::default(),
            };
            black_box(handlers::prepare_call_hierarchy(&indexer, params).unwrap());
        });
    });

    // incomingCalls: the hub's Proc0 has real fan-in (999 callers, one per
    // other generated file) — see perf_support module doc.
    group.bench_function("incomingCalls", |b| {
        b.iter(|| {
            let params = CallHierarchyIncomingCallsParams {
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
            black_box(handlers::incoming_calls(&indexer, params).unwrap());
        });
    });

    // outgoingCalls: any non-hub file's Proc0 has real fan-out (3 callees).
    group.bench_function("outgoingCalls", |b| {
        b.iter(|| {
            let params = CallHierarchyOutgoingCallsParams {
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
            black_box(handlers::outgoing_calls(&indexer, params).unwrap());
        });
    });

    group.finish();
}

/// Single-file reindex against a 1000-file graph (CLAUDE.md target: <50ms).
/// The target file's on-disk content is fixed before timing starts; each
/// iteration re-runs the exact `remove_file` + reparse + `add_to_graph` path
/// the real file-watcher/didSave flow triggers on every save. The on-disk
/// content is changed once, before timing starts (see
/// `perf_support::rewrite_with_extra_procedure`) — the disk write itself
/// isn't part of what the LSP measures, only the reindex that follows it.
fn bench_single_file_reindex(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    perf_support::generate_corpus(dir.path(), 1000);
    let mut indexer = Indexer::new();
    indexer.index_directory(dir.path()).unwrap();
    let target = dir.path().join(perf_support::file_name(1));
    perf_support::rewrite_with_extra_procedure(dir.path(), 1000, 1);

    c.bench_function("single_file_reindex_1000_files", |b| {
        b.iter(|| {
            indexer.reindex_file(black_box(&target)).unwrap();
        });
    });
}

criterion_group!(
    benches,
    bench_initial_index,
    bench_queries,
    bench_single_file_reindex
);
criterion_main!(benches);
