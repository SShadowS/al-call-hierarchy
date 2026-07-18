//! THROWAWAY instrumentation for the 2026-07-17 memory/speed design review.
//! Env-gated: set ALSEM_STAGE_TIMING=1 to emit stderr stage marks with wall
//! time + current/peak RSS. Never committed to master.

use std::sync::OnceLock;
use std::time::Instant;

static T0: OnceLock<Instant> = OnceLock::new();
static ENABLED: OnceLock<bool> = OnceLock::new();

fn enabled() -> bool {
    *ENABLED.get_or_init(|| std::env::var("ALSEM_STAGE_TIMING").as_deref() == Ok("1"))
}

#[cfg(windows)]
fn rss_mb() -> (u64, u64) {
    #[repr(C)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(
            process: isize,
            counters: *mut ProcessMemoryCounters,
            cb: u32,
        ) -> i32;
    }
    unsafe {
        let mut c: ProcessMemoryCounters = std::mem::zeroed();
        c.cb = std::mem::size_of::<ProcessMemoryCounters>() as u32;
        if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) != 0 {
            (
                (c.working_set_size / 1_048_576) as u64,
                (c.peak_working_set_size / 1_048_576) as u64,
            )
        } else {
            (0, 0)
        }
    }
}

#[cfg(not(windows))]
fn rss_mb() -> (u64, u64) {
    (0, 0)
}

use std::sync::atomic::{AtomicU64, Ordering};

pub const ACC_PARSE: usize = 0;
pub const ACC_PROJECT: usize = 1;
pub const ACC_UTF16: usize = 2;
pub const ACC_JACOBI_CLONE: usize = 3;
pub const ACC_JACOBI_COMPOSE: usize = 4;
pub const ACC_JACOBI_FP: usize = 5;
pub const ACC_SPAN_BFS: usize = 6;
static ACCUMS: [AtomicU64; 7] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static ACC_NAMES: [&str; 7] = [
    "al_syntax_parse",
    "l3_projection",
    "utf16_cols",
    "jacobi_snapshot_clone",
    "jacobi_compose",
    "jacobi_project_fingerprint",
    "span_backward_bfs",
];

/// Accumulate a duration into a named slot (cheap; safe cross-thread).
pub fn accum(slot: usize, d: std::time::Duration) {
    if !enabled() {
        return;
    }
    ACCUMS[slot].fetch_add(d.as_nanos() as u64, Ordering::Relaxed);
}

/// Print all nonzero accumulators as `ACCUM <name> total_s=<s>`.
pub fn dump_accums() {
    if !enabled() {
        return;
    }
    for (i, a) in ACCUMS.iter().enumerate() {
        let ns = a.load(Ordering::Relaxed);
        if ns > 0 {
            eprintln!("ACCUM {} total_s={:.2}", ACC_NAMES[i], ns as f64 / 1e9);
        }
    }
}

/// Emit a stage mark: `STAGE <name> t=<s> rss_mb=<cur> peak_mb=<peak>`.
pub fn stage(name: &str) {
    if !enabled() {
        return;
    }
    let t0 = T0.get_or_init(Instant::now);
    let (cur, peak) = rss_mb();
    eprintln!(
        "STAGE {name} t={:.2}s rss_mb={cur} peak_mb={peak}",
        t0.elapsed().as_secs_f64()
    );
}
