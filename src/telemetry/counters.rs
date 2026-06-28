//! Atomic counters shared between producer threads (LSP request handlers)
//! and the background telemetry thread. See spec §6 "Counter dimensions".

use crate::telemetry::events::LeafKind;
use std::sync::atomic::{AtomicU32, Ordering};

pub struct Counters {
    pub observed: [AtomicU32; 14],
    pub exported: [AtomicU32; 14],
    pub dedup_suppressed: AtomicU32,
    pub queue_full_drops: AtomicU32,
    pub export_attempts: AtomicU32,
    pub export_failures: AtomicU32,
}

impl Default for Counters {
    fn default() -> Self {
        Self::new()
    }
}

impl Counters {
    pub const fn new() -> Self {
        // const-init is verbose pre-Rust-1.79; use a helper.
        const fn zero_array() -> [AtomicU32; 14] {
            [
                AtomicU32::new(0),
                AtomicU32::new(0),
                AtomicU32::new(0),
                AtomicU32::new(0),
                AtomicU32::new(0),
                AtomicU32::new(0),
                AtomicU32::new(0),
                AtomicU32::new(0),
                AtomicU32::new(0),
                AtomicU32::new(0),
                AtomicU32::new(0),
                AtomicU32::new(0),
                AtomicU32::new(0),
                AtomicU32::new(0),
            ]
        }
        Self {
            observed: zero_array(),
            exported: zero_array(),
            dedup_suppressed: AtomicU32::new(0),
            queue_full_drops: AtomicU32::new(0),
            export_attempts: AtomicU32::new(0),
            export_failures: AtomicU32::new(0),
        }
    }

    pub fn observe(&self, kind: LeafKind) {
        self.observed[kind.index()].fetch_add(1, Ordering::Relaxed);
    }

    pub fn export_succeeded(&self, kind: LeafKind) {
        self.exported[kind.index()].fetch_add(1, Ordering::Relaxed);
    }

    pub fn dedup_suppress(&self) {
        self.dedup_suppressed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn queue_full(&self) {
        self.queue_full_drops.fetch_add(1, Ordering::Relaxed);
    }

    pub fn export_attempted(&self) {
        self.export_attempts.fetch_add(1, Ordering::Relaxed);
    }

    pub fn export_failed(&self) {
        self.export_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> Snapshot {
        let read = |a: &[AtomicU32; 14]| -> [u32; 14] {
            let mut out = [0u32; 14];
            for (i, v) in a.iter().enumerate() {
                out[i] = v.load(Ordering::Relaxed);
            }
            out
        };
        Snapshot {
            observed_by_kind: read(&self.observed),
            exported_by_kind: read(&self.exported),
            dedup_suppressed: self.dedup_suppressed.load(Ordering::Relaxed),
            queue_full_drops: self.queue_full_drops.load(Ordering::Relaxed),
            export_attempts: self.export_attempts.load(Ordering::Relaxed),
            export_failures: self.export_failures.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub observed_by_kind: [u32; 14],
    pub exported_by_kind: [u32; 14],
    pub dedup_suppressed: u32,
    pub queue_full_drops: u32,
    pub export_attempts: u32,
    pub export_failures: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observe_increments_correct_slot() {
        let c = Counters::new();
        c.observe(LeafKind::ResolutionObjectNotFound);
        c.observe(LeafKind::ResolutionObjectNotFound);
        c.observe(LeafKind::ParserTreeError);
        let snap = c.snapshot();
        assert_eq!(
            snap.observed_by_kind[LeafKind::ResolutionObjectNotFound.index()],
            2
        );
        assert_eq!(snap.observed_by_kind[LeafKind::ParserTreeError.index()], 1);
        assert_eq!(snap.observed_by_kind[LeafKind::HandlerEmpty.index()], 0);
    }

    #[test]
    fn pipeline_counters_independent_of_observed() {
        let c = Counters::new();
        c.queue_full();
        c.queue_full();
        c.dedup_suppress();
        let snap = c.snapshot();
        assert_eq!(snap.queue_full_drops, 2);
        assert_eq!(snap.dedup_suppressed, 1);
        assert_eq!(snap.observed_by_kind, [0u32; 14]);
    }

    #[test]
    fn snapshot_is_consistent_under_concurrent_writes() {
        use std::sync::Arc;
        use std::thread;

        let c = Arc::new(Counters::new());
        let mut handles = vec![];
        for _ in 0..16 {
            let c2 = c.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..1000 {
                    c2.observe(LeafKind::ResolutionProcedureNotFound);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let snap = c.snapshot();
        assert_eq!(
            snap.observed_by_kind[LeafKind::ResolutionProcedureNotFound.index()],
            16_000
        );
    }
}
