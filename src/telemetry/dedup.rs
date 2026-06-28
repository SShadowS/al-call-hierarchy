//! Workspace-scoped LRU dedup with TTL.
//!
//! Same call shape repeating within a session is suppressed and counted.
//! Different workspaces never cross-suppress (key includes workspace_id).
//!
//! NOTE: complete but not yet wired into the telemetry pipeline (future design);
//! module-level `allow(dead_code)` until a caller consumes it.
#![allow(dead_code)]

use crate::telemetry::events::LeafKind;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct DedupKey {
    pub kind: LeafKind,
    pub workspace_id: String,
    pub object_hash: Option<String>,
    pub procedure_hash: Option<String>,
    pub callee_object_type: Option<u8>, // discriminant of ObjectType
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub first_seen: Instant,
    pub last_seen: Instant,
    pub repeat_count: u32,
}

pub enum Decision {
    First,
    Repeat,
}

pub struct Dedup {
    cache: Mutex<LruCache<DedupKey, Entry>>,
    ttl: Duration,
}

impl Dedup {
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        let cap = NonZeroUsize::new(capacity.max(1)).unwrap();
        Self {
            cache: Mutex::new(LruCache::new(cap)),
            ttl,
        }
    }

    pub fn check(&self, key: &DedupKey, now: Instant) -> Decision {
        let mut cache = self.cache.lock().expect("dedup mutex poisoned");
        if let Some(entry) = cache.get_mut(key) {
            if now.saturating_duration_since(entry.last_seen) > self.ttl {
                // TTL expired — treat as new occurrence; reset the entry.
                *entry = Entry {
                    first_seen: now,
                    last_seen: now,
                    repeat_count: 0,
                };
                Decision::First
            } else {
                entry.last_seen = now;
                entry.repeat_count = entry.repeat_count.saturating_add(1);
                Decision::Repeat
            }
        } else {
            cache.put(
                key.clone(),
                Entry {
                    first_seen: now,
                    last_seen: now,
                    repeat_count: 0,
                },
            );
            Decision::First
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::events::LeafKind;

    fn key(workspace: &str, proc_hash: &str) -> DedupKey {
        DedupKey {
            kind: LeafKind::ResolutionProcedureNotFound,
            workspace_id: workspace.into(),
            object_hash: Some("obj".into()),
            procedure_hash: Some(proc_hash.into()),
            callee_object_type: Some(0),
        }
    }

    #[test]
    fn first_call_returns_first() {
        let d = Dedup::new(16, Duration::from_secs(60));
        match d.check(&key("ws", "p1"), Instant::now()) {
            Decision::First => {}
            _ => panic!(),
        }
    }

    #[test]
    fn repeat_within_ttl_returns_repeat() {
        let d = Dedup::new(16, Duration::from_secs(60));
        let now = Instant::now();
        let _ = d.check(&key("ws", "p1"), now);
        match d.check(&key("ws", "p1"), now) {
            Decision::Repeat => {}
            _ => panic!(),
        }
    }

    #[test]
    fn different_workspace_ids_do_not_cross_suppress() {
        let d = Dedup::new(16, Duration::from_secs(60));
        let now = Instant::now();
        match d.check(&key("ws_a", "p1"), now) {
            Decision::First => {}
            _ => panic!(),
        }
        match d.check(&key("ws_b", "p1"), now) {
            Decision::First => {}
            _ => panic!("ws_b should be First, not suppressed by ws_a"),
        }
    }

    #[test]
    fn ttl_expiry_resets_to_first() {
        let d = Dedup::new(16, Duration::from_millis(50));
        let t0 = Instant::now();
        let _ = d.check(&key("ws", "p1"), t0);
        let t1 = t0 + Duration::from_millis(100);
        match d.check(&key("ws", "p1"), t1) {
            Decision::First => {}
            _ => panic!("expired entry should be First again"),
        }
    }
}
