//! Correctness checks for the synthetic AL corpus generator (Task T0.5).
//! Kept as its own always-compiled integration test target (rather than a
//! `#[cfg(test)]` block inside `perf_support/mod.rs`) because that file is
//! also `#[path]`-included by `benches/lsp_pipeline.rs`, a `harness = false`
//! bench where `#[test]` functions would compile as unreachable dead code.

#[path = "perf_support/mod.rs"]
mod perf_support;

use al_call_hierarchy::graph::QualifiedName;
use al_call_hierarchy::indexer::Indexer;
use perf_support::{HUB_INDEX, PROCS_PER_FILE, file_name, generate_corpus, object_name};
use std::fs;
use tempfile::TempDir;

#[test]
fn generates_expected_file_count() {
    let dir = TempDir::new().unwrap();
    let n = generate_corpus(dir.path(), 10);
    assert_eq!(n, 10);
    let al_files: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "al"))
        .collect();
    assert_eq!(al_files.len(), 10);
}

#[test]
fn corpus_is_deterministic_across_calls() {
    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();
    generate_corpus(dir_a.path(), 25);
    generate_corpus(dir_b.path(), 25);
    for i in 0..25 {
        let a = fs::read_to_string(dir_a.path().join(file_name(i))).unwrap();
        let b = fs::read_to_string(dir_b.path().join(file_name(i))).unwrap();
        assert_eq!(a, b, "file {i} content diverged between two generations");
    }
}

#[test]
fn indexes_with_expected_definitions_and_hub_fan_in() {
    let dir = TempDir::new().unwrap();
    let file_count = 20;
    generate_corpus(dir.path(), file_count);

    let mut indexer = Indexer::new();
    indexer.index_directory(dir.path()).unwrap();
    let graph = indexer.graph();

    assert_eq!(graph.definition_count(), file_count * PROCS_PER_FILE);

    // The hub's Proc0 must have exactly `file_count - 1` incoming calls (one
    // qualified call from every other file's Proc0).
    let hub_obj = graph.get_symbol(&object_name(HUB_INDEX)).unwrap();
    let proc0 = graph.get_symbol("Proc0").unwrap();
    let hub_qname = QualifiedName {
        object: hub_obj,
        procedure: proc0,
    };
    assert_eq!(graph.get_incoming_calls(&hub_qname).len(), file_count - 1);

    // A non-hub file's Proc0 must have exactly 3 outgoing calls.
    let f1_obj = graph.get_symbol(&object_name(1)).unwrap();
    let f1_qname = QualifiedName {
        object: f1_obj,
        procedure: proc0,
    };
    assert_eq!(graph.get_outgoing_calls(&f1_qname).len(), 3);
}

#[test]
fn rewrite_with_extra_procedure_adds_one_definition() {
    let dir = TempDir::new().unwrap();
    let file_count = 5;
    generate_corpus(dir.path(), file_count);

    let mut indexer = Indexer::new();
    indexer.index_directory(dir.path()).unwrap();
    assert_eq!(
        indexer.graph().definition_count(),
        file_count * PROCS_PER_FILE
    );

    perf_support::rewrite_with_extra_procedure(dir.path(), file_count, 1);
    indexer
        .reindex_file(&dir.path().join(file_name(1)))
        .unwrap();

    assert_eq!(
        indexer.graph().definition_count(),
        file_count * PROCS_PER_FILE + 1,
        "rewritten file must contribute one extra definition (ProcExtra)"
    );
}
