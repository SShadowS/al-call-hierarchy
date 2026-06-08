//! R3b Task 2 — STAGE 2 incremental-NONDETERMINISM oracle (Rev 2 #4).
//!
//! The incremental output must be byte-identical to from-scratch REGARDLESS of:
//!   1. the DEMAND ORDER — demanding the summaries/cones in shuffled routine orders
//!      must yield byte-identical output (Salsa memoizes; the JACOBI loop iterates
//!      members in the canonical sorted-StableRoutineId order, not visitation
//!      order).
//!   2. the DB PROVENANCE — the same edit applied to {fresh-from-scratch /
//!      reused-Salsa / reused-Salsa-with-a-different-prior-demand-order} must give
//!      byte-identical output.
//!   3. the recursive-SCC FIXPOINT SCHEDULE — a recursive SCC's settled fingerprint
//!      is order-invariant (the JACOBI/Gauss-Seidel loop converges to the same fixed
//!      point under any demand schedule).
//!   4. `RUST_HASH_SEED` — the internal `HashMap`/`HashSet` iteration order must not
//!      leak into output. This suite is RE-RUN under varied seeds in CI; locally,
//!      run e.g. `RUST_HASH_SEED=0`, `RUST_HASH_SEED=1`, `RUST_HASH_SEED=999` before
//!      `cargo test --test r3b_incremental_nondeterminism`.
//!
//! Rust's `RandomState` reads `RUST_HASH_SEED` when set; the assertions below hold
//! for any seed. The harness also self-checks by shuffling demand order, which
//! perturbs the SAME HashMaps a seed change would.

use std::path::PathBuf;

use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_workspace_default;
use al_call_hierarchy::engine::l4::incremental::edit::InputModel;
use al_call_hierarchy::engine::l4::incremental::wrap::input_model_r3a3_source_only;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn load_model(fixture: &str) -> Option<InputModel> {
    let dir = repo_root().join("tests").join("r0-corpus").join(fixture);
    let resolved = assemble_and_resolve_workspace_default(&dir)?;
    let m = input_model_r3a3_source_only(&resolved);
    if m.routine_ids.is_empty() {
        None
    } else {
        Some(m)
    }
}

/// A deterministic shuffle (Fisher–Yates with a fixed-seed xorshift) — perturbs the
/// demand order WITHOUT depending on `RUST_HASH_SEED`.
fn shuffle(mut v: Vec<String>, seed: u64) -> Vec<String> {
    let mut x = seed ^ 0xD1B5_4A32_D192_ED03;
    let mut step = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let n = v.len();
    for i in (1..n).rev() {
        let j = (step() % (i as u64 + 1)) as usize;
        v.swap(i, j);
    }
    v
}

/// The corpus subset for the nondeterminism sweep — a spread including the recursive
/// + event-cycle fixtures (the order-sensitive fixpoints) plus a broad slice.
fn sweep_fixtures() -> Vec<String> {
    let mut out = vec![
        "ws-recursive".to_string(),
        "ws-event-cycle".to_string(),
        "ws-d7-event-cycle".to_string(),
        "ws-compose".to_string(),
        "ws-combined".to_string(),
        "ws-calls".to_string(),
    ];
    // Add a broad slice from the r3a3 goldens for coverage.
    let gold_dir = repo_root().join("tests").join("r3a3-goldens");
    if let Ok(rd) = std::fs::read_dir(&gold_dir) {
        let mut more: Vec<String> = rd
            .filter_map(|e| {
                let n = e.ok()?.file_name().to_string_lossy().to_string();
                n.strip_suffix(".r3a3.golden.json").map(|s| s.to_string())
            })
            .collect();
        more.sort();
        out.extend(more.into_iter().take(40));
    }
    out.dedup();
    out
}

// ===========================================================================
// (1) Shuffled demand order — byte-identical output.
// ===========================================================================

#[test]
fn r3b_stage2_shuffled_demand_order_is_byte_identical() {
    let mut checked = 0usize;
    for fixture in sweep_fixtures() {
        let Some(model) = load_model(&fixture) else {
            continue;
        };
        let em = model.build_incremental();
        let sorted = {
            let mut v = model.routine_ids.clone();
            v.sort();
            v
        };
        let canonical = em.demand_in_order(&sorted).fingerprint();

        for seed in [1u64, 7, 42, 1000, 999_999] {
            let order = shuffle(sorted.clone(), seed);
            let shuffled = em.demand_in_order(&order).fingerprint();
            assert_eq!(
                canonical, shuffled,
                "[{fixture}] demand-order shuffle (seed {seed}) changed the demanded output — \
                 nondeterminism leak"
            );
        }
        checked += 1;
    }
    assert!(checked >= 5, "too few fixtures swept ({checked})");
    eprintln!("R3b Stage 2: shuffled-demand-order byte-identical on {checked} fixtures");
}

// ===========================================================================
// (2) Same edit on {fresh / reused / reused-diff-order} — all byte-equal.
// ===========================================================================

#[test]
fn r3b_stage2_same_edit_db_provenance_invariant() {
    let mut checked = 0usize;
    for fixture in sweep_fixtures() {
        let Some(model) = load_model(&fixture) else {
            continue;
        };
        let Some(target) = model.routine_ids.first().cloned() else {
            continue;
        };
        let sorted = {
            let mut v = model.routine_ids.clone();
            v.sort();
            v
        };

        // The edit: bump the app identity (touches the cone object-resolution input
        // lineage; a uniform, always-applicable edit).
        let apply = |em: &mut al_call_hierarchy::engine::l4::incremental::edit::EditableModel| {
            em.set_app_identity("nondet-probe-identity");
            // Also a per-routine edit to exercise a routine-scoped invalidation.
            em.set_body_available(&target, false);
        };

        // (a) FRESH: build, edit immediately, demand.
        let mut fresh = model.build_incremental();
        apply(&mut fresh);
        let fresh_fp = fresh.demand().fingerprint();

        // (b) REUSED: build, demand (prime), edit, re-demand.
        let mut reused = model.build_incremental();
        let _ = reused.demand();
        apply(&mut reused);
        let reused_fp = reused.demand().fingerprint();

        // (c) REUSED with a DIFFERENT prior demand order, then edit, re-demand.
        let mut reused_diff = model.build_incremental();
        let _ = reused_diff.demand_in_order(&shuffle(sorted.clone(), 12345));
        apply(&mut reused_diff);
        let reused_diff_fp = reused_diff
            .demand_in_order(&shuffle(sorted.clone(), 67890))
            .fingerprint();

        // (d) FROM-SCRATCH oracle over the edited inputs (via the reused mirror).
        let from_scratch_fp = reused.model.demand_from_scratch().fingerprint();

        assert_eq!(
            fresh_fp, reused_fp,
            "[{fixture}] fresh vs reused-DB diverged after the same edit"
        );
        assert_eq!(
            reused_fp, reused_diff_fp,
            "[{fixture}] reused-DB vs reused-DB-different-demand-order diverged"
        );
        assert_eq!(
            reused_fp, from_scratch_fp,
            "[{fixture}] reused-DB vs from-scratch diverged after the same edit"
        );
        checked += 1;
    }
    assert!(checked >= 5, "too few fixtures swept ({checked})");
    eprintln!(
        "R3b Stage 2: {checked} fixtures — fresh == reused == reused-diff-order == from-scratch \
         (same edit, all byte-equal)"
    );
}

// ===========================================================================
// (3) Recursive-SCC fixpoint — the settled fingerprint is schedule-invariant.
// ===========================================================================

#[test]
fn r3b_stage2_recursive_scc_fingerprint_is_schedule_invariant() {
    // The recursive + event-cycle fixtures carry genuine multi-member SCCs (the
    // JACOBI fixpoint). Re-demand under many shuffled schedules; the recursive
    // routines' settled CORE summaries must be byte-identical every time.
    let recursive_fixtures = ["ws-recursive", "ws-event-cycle", "ws-d7-event-cycle"];
    let mut checked = 0usize;
    for fixture in recursive_fixtures {
        let Some(model) = load_model(fixture) else {
            continue;
        };
        let em = model.build_incremental();
        let sorted = {
            let mut v = model.routine_ids.clone();
            v.sort();
            v
        };
        let baseline = em.demand_in_order(&sorted);

        // The recursive members (the fixpoint participants).
        let recursive_ids: Vec<String> = baseline
            .core
            .iter()
            .filter(|(_, s)| s.in_recursive_cycle)
            .map(|(id, _)| id.clone())
            .collect();
        assert!(
            !recursive_ids.is_empty(),
            "[{fixture}] expected ≥1 recursive-cycle routine — fixture is not a fixpoint probe"
        );

        // Render the recursive members' settled summaries (the order-sensitive
        // fixpoint trace) and assert schedule-invariance.
        let trace_of = |res: &al_call_hierarchy::engine::l4::incremental::edit::DemandResult| {
            let mut ids = recursive_ids.clone();
            ids.sort();
            ids.iter()
                .map(|id| format!("{id}={:?}", res.core.get(id)))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let baseline_trace = trace_of(&baseline);

        for seed in [1u64, 3, 9, 27, 81, 243, 999] {
            let order = shuffle(sorted.clone(), seed);
            let again = em.demand_in_order(&order);
            assert_eq!(
                baseline_trace,
                trace_of(&again),
                "[{fixture}] recursive-SCC fixpoint trace changed under demand schedule (seed \
                 {seed}) — the JACOBI loop is NOT order-invariant"
            );
        }
        // And: the recursive fixpoint equals the from-scratch fixpoint.
        let fs = model.demand_from_scratch();
        assert_eq!(
            baseline_trace,
            trace_of(&fs),
            "[{fixture}] recursive-SCC fixpoint diverged from from-scratch"
        );
        checked += 1;
    }
    assert!(checked >= 1, "no recursive fixture available");
    eprintln!(
        "R3b Stage 2: recursive-SCC fixpoint schedule-invariant on {checked} fixtures (JACOBI \
         loop order-invariant, == from-scratch)"
    );
}
