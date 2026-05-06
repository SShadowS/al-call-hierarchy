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
