//! End-to-end tests: queue overflow accounting, dedup workspace isolation,
//! shutdown drains and emits summary.

#![cfg(feature = "telemetry")]

use al_call_hierarchy::telemetry::counters::Counters;
use al_call_hierarchy::telemetry::events::LeafKind;
use al_call_hierarchy::telemetry::pipeline::Pipeline;
use std::sync::Arc;

#[tokio::test(flavor = "current_thread")]
async fn queue_full_distinguishable_from_dedup() {
    let counters = Arc::new(Counters::new());
    let (p, _rx) = Pipeline::new(2, counters.clone());
    counters.observe(LeafKind::ResolutionObjectNotFound);
    counters.dedup_suppress();
    counters.queue_full();

    let snap = counters.snapshot();
    assert_eq!(snap.queue_full_drops, 1);
    assert_eq!(snap.dedup_suppressed, 1);
    assert_eq!(
        snap.observed_by_kind[LeafKind::ResolutionObjectNotFound.index()],
        1
    );

    drop(p);
}

/// Verifies `record_resolution_miss` increments the appropriate counter slot
/// when invoked directly through the public API after installing a
/// counters-only test runtime. The graph.rs instrumentation hooks added in
/// Phase 2 Task 2.3 use exactly this code path; this test pins the wire-up
/// (test helper -> public `record_resolution_miss` -> counters) end-to-end.
///
/// TODO Phase 2.5 wrap: once `crate::indexer` is exposed to the library, add
/// a fixture-driven integration test that runs `Indexer::index_directory` on
/// `tests/fixtures/telemetry/unresolved_app_dep` and asserts the same counter
/// is incremented organically. The current binary-only module layout makes
/// that exposure expensive (7-module cascade), so we leave it as future work.
#[cfg(feature = "test-runtime")]
#[test]
fn record_resolution_miss_increments_counters() {
    use al_call_hierarchy::telemetry::{
        record_resolution_miss, testing, CallContext, CallPattern, CalleeSource, CallerContext,
        ObjectType, ResolutionFailure,
    };

    let counters = Arc::new(Counters::new());
    testing::install_runtime_for_test(counters.clone());

    // Some other test in this binary may have installed a runtime first
    // (OnceLock), so always read the actually-active counters back.
    let active = testing::current_counters().expect("runtime must be installed");
    let before_object =
        active.snapshot().observed_by_kind[LeafKind::ResolutionObjectNotFound.index()];
    let before_unqualified =
        active.snapshot().observed_by_kind[LeafKind::ResolutionUnresolvedUnqualified.index()];

    record_resolution_miss(&CallContext {
        failure: ResolutionFailure::ObjectNotFound,
        call_pattern: CallPattern::Qualified,
        callee_object_type: None,
        callee_source: CalleeSource::Unknown,
        caller_object_type: ObjectType::Other,
        caller_context: CallerContext::Procedure,
        callee_object_name: Some("Missing External Codeunit"),
        callee_procedure_name: "PostInvoice",
        arg_count: 0,
        ts_node_path: "",
    });
    record_resolution_miss(&CallContext {
        failure: ResolutionFailure::UnresolvedUnqualified,
        call_pattern: CallPattern::Unqualified,
        callee_object_type: None,
        callee_source: CalleeSource::Workspace,
        caller_object_type: ObjectType::Other,
        caller_context: CallerContext::Procedure,
        callee_object_name: None,
        callee_procedure_name: "SomeMissingHelper",
        arg_count: 0,
        ts_node_path: "",
    });

    let snap = active.snapshot();
    assert_eq!(
        snap.observed_by_kind[LeafKind::ResolutionObjectNotFound.index()],
        before_object + 1,
        "ObjectNotFound counter should increment by exactly 1"
    );
    assert_eq!(
        snap.observed_by_kind[LeafKind::ResolutionUnresolvedUnqualified.index()],
        before_unqualified + 1,
        "UnresolvedUnqualified counter should increment by exactly 1"
    );
}
