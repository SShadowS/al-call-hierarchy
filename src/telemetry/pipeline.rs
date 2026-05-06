//! Producer-side mpsc channel and try_send wrapper.
//!
//! Hot path: one atomic load, one try_send. Drop-on-full is a counter bump.

use crate::telemetry::counters::Counters;
use crate::telemetry::events::EventEnvelope;
use std::sync::Arc;
use tokio::sync::mpsc::{self, error::TrySendError, Receiver, Sender};

pub struct Pipeline {
    tx: Sender<EventEnvelope>,
    counters: Arc<Counters>,
}

impl Pipeline {
    pub fn new(capacity: usize, counters: Arc<Counters>) -> (Self, Receiver<EventEnvelope>) {
        let (tx, rx) = mpsc::channel(capacity);
        (Self { tx, counters }, rx)
    }

    /// Non-blocking send. On full or closed channel, drops the event and bumps
    /// `queue_full_drops`. The hot path must call `counters.observe(...)` BEFORE
    /// calling this method so app-side observation is recorded regardless of fate.
    pub fn send(&self, env: EventEnvelope) {
        match self.tx.try_send(env) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) | Err(TrySendError::Closed(_)) => {
                self.counters.queue_full();
            }
        }
    }

    /// Cheap clone of the sender so multiple producer threads share the channel.
    pub fn clone_sender(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            counters: self.counters.clone(),
        }
    }

    /// Used at shutdown to close the producer side and let the background
    /// thread observe channel disconnect.
    pub fn close(self) {
        drop(self.tx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::events::{DefinitionKind, EventKind, HandlerEmpty, ObjectType};
    use std::time::SystemTime;

    fn dummy_event() -> EventEnvelope {
        EventEnvelope {
            schema_version: 1,
            timestamp: SystemTime::now(),
            install_id: "0000000000000000".into(),
            al_version: env!("CARGO_PKG_VERSION"),
            grammar_version: "v2",
            os: "test",
            session_id: 0,
            workspace_id: "0000000000000000".into(),
            event: EventKind::HandlerEmpty(HandlerEmpty {
                method: "incomingCalls",
                target_object_type: ObjectType::Codeunit,
                target_kind: DefinitionKind::Procedure,
                object_hash: "x".into(),
                procedure_hash: "y".into(),
                repeat_count: 0,
            }),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn send_succeeds_when_capacity_available() {
        let counters = Arc::new(Counters::new());
        let (p, mut rx) = Pipeline::new(8, counters.clone());
        p.send(dummy_event());
        assert!(rx.try_recv().is_ok());
        assert_eq!(counters.snapshot().queue_full_drops, 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn send_drops_and_counts_when_full() {
        let counters = Arc::new(Counters::new());
        let (p, _rx) = Pipeline::new(2, counters.clone());
        // Don't read from rx; fill capacity.
        p.send(dummy_event());
        p.send(dummy_event());
        // Third send must drop.
        p.send(dummy_event());
        assert_eq!(counters.snapshot().queue_full_drops, 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn send_after_close_drops() {
        let counters = Arc::new(Counters::new());
        let (p, rx) = Pipeline::new(2, counters.clone());
        drop(rx);
        p.send(dummy_event());
        assert_eq!(counters.snapshot().queue_full_drops, 1);
    }
}
