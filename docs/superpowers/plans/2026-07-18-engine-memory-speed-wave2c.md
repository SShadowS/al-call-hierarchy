# Engine Memory/Speed Wave 2c (d1 walk_evidence Memoization) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collapse d1-db-op-in-loop's O(in-loop-callsites × 500-node DFS) to O(distinct callees × one DFS) by memoizing `walk_evidence` per callee — the sole remaining full-default blocker at 8020 scale (~93 min alone). Byte-identical findings.

**Architecture:** One task in `src/engine/l5/detectors/d1.rs`: a per-run memo keyed by callee id storing the CALLER-INDEPENDENT walk (empty prefix, zero initial depth); each callsite applies the mechanical transform (prepend its 2 prefix steps; add its loop-depth offset). Purity proof (source-verified): `initial_steps` is a pure prefix cloned into every result path; `initial_loop_depth` is only ever ADDITIVE (`path_walker.rs:173/184/194/205/214/223/235` — never a branch/cut input); `routine_path`/`uncertainties` start caller-independent (`path_walker.rs:129-142`); the node budget starts fresh per call today, so a memoized canonical walk explores the identical tree. Design doc: `.superpowers/sdd/w2c-design.md` (§3 proof; Option B-memo).

**Tech Stack:** Rust, crate `al-call-hierarchy`. No new deps.

## Global Constraints

- Byte-stable: `bash scripts/check-goldens` clean; full `cargo test` green; clippy clean; DO analyze JSON byte-identical modulo the single `generatedAt` line (capture the baseline with the pre-change release-fast binary BEFORE editing).
- rustfmt per file, never `cargo fmt`. Explicit staging. No stash. No `| tail` on long runs. Foreground commands only for subagents (background watchers cannot resume you).
- DO workspace `U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud`; corpora per the Wave-1 plan script; release-fast builds; kill no measurement processes.
- Dispatch: T1 Opus implementer + Opus reviewer; T2 Sonnet runbook; T3 orchestrator.

---

### Task 1: Memoize walk_evidence per callee in d1

**Files:**
- Modify: `src/engine/l5/detectors/d1.rs` (the walk call at ~:994 inside the per-callsite loop; the memo lives beside the T2-era `touches_db_memo` on the same per-run policy/context struct — find it at ~:587-610)
- Test: d1's test module (same file or its test sibling — `grep -n "mod.*test\|#\[test\]" src/engine/l5/detectors/d1.rs` to locate)

**Interfaces:** none outside d1.

- [ ] **Step 1: Read the exact call shape.** The call at ~:994 passes `WalkOpts { initial_loop_depth: cs.loop_stack.len() as i64, initial_steps: vec![loop_step, call_step] }` and starts at `&edge.to`. Confirm (grep) there is exactly ONE `walk_evidence` call in d1; confirm what `results` is used for afterward (per-result: path, effective_loop_depth, uncertainties, stop) so the transform covers every consumed field.

- [ ] **Step 2: Failing test first — memoized ≡ fresh.**

Add a test constructing a small ctx (reuse d1's existing fixture helpers) with: one callee reachable from TWO different in-loop callsites at DIFFERENT loop depths (1 and 2), the callee reaching one db op. Assert the per-callsite outputs (paths incl. prefixes, effective_loop_depth, uncertainties) from the memoized path equal a fresh direct `walk_evidence` call with the caller-specific opts. RED: the helper `canonical_walk_then_transform` doesn't exist yet.

```rust
    #[test]
    fn memoized_walk_matches_fresh_walk_for_two_callsites() {
        // build ctx: routine A (loop depth 1) and routine B (loop depth 2) both
        // call C; C contains a Modify db op.
        // fresh_1 = walk_evidence(C, policy, BOUNDS, WalkOpts{1, prefix1}, unc);
        // fresh_2 = walk_evidence(C, policy, BOUNDS, WalkOpts{2, prefix2}, unc);
        // memo    = canonical walk of C (WalkOpts{0, vec![]}) reused twice + transform;
        // assert_eq!(transform(memo, 1, prefix1), fresh_1);
        // assert_eq!(transform(memo, 2, prefix2), fresh_2);
    }
```

Fill with the real fixture-builder calls (mirror the style of d1's existing memo_tests from the Wave-2a change).

- [ ] **Step 3: Implement.**

```rust
    /// Per-run memo of the CANONICAL walk from a callee (empty prefix, zero
    /// initial depth). Caller-specific results are derived by the mechanical
    /// transform below — sound because initial_steps is a pure prefix and
    /// initial_loop_depth is only ever additive in the walker (never a
    /// branch/cut input; see path_walker.rs visit()).
    walk_memo: RefCell<HashMap<String, Rc<Vec<WalkResult>>>>,
```

At the call site:

```rust
    let canonical = {
        let mut memo = self.walk_memo.borrow_mut();
        Rc::clone(memo.entry(edge.to.clone()).or_insert_with(|| {
            Rc::new(walk_evidence(
                &edge.to,
                &policy,
                BOUNDS,
                WalkOpts {
                    initial_loop_depth: 0,
                    initial_steps: Vec::new(),
                },
                uncertainties_by_node,
            ))
        }))
    };
    let cs_depth = cs.loop_stack.len() as i64;
    let results: Vec<WalkResult> = canonical
        .iter()
        .map(|r| WalkResult {
            path: {
                let mut p = Vec::with_capacity(2 + r.path.len());
                p.push(loop_step.clone());
                p.push(call_step.clone());
                p.extend(r.path.iter().cloned());
                p
            },
            effective_loop_depth: r.effective_loop_depth + cs_depth,
            uncertainties: r.uncertainties.clone(),
            stop: r.stop.clone(),
        })
        .collect();
```

Adapt to the real types (WalkResult field names/Clone derives — add `Clone` if missing, that's a path_walker.rs one-liner; `Rc` vs plain clone: Rc avoids re-cloning the canonical vec per lookup — fine either way, pick what borrows cleanly). CAUTION: if `policy` differs per candidate/callsite (check its construction — if policy captures per-CALLSITE state, the memo key must include it or the memo is WRONG; if it's per-run, key by callee alone). STOP and report if policy is callsite-dependent in any way that feeds the walk.

- [ ] **Step 4: GREEN + gates + DO byte-diff** (baseline pre-captured). Full `cargo test`, check-goldens, clippy, DO diff empty modulo generatedAt.

- [ ] **Step 5: rustfmt + commit**

```bash
git commit -m "perf(l5): memoize d1's walk_evidence per callee

The walk from a callee is caller-independent (initial_steps = pure prefix;
initial_loop_depth = additive only, proven in path_walker's visit); each
callsite derives its result by a mechanical prefix+offset transform.
O(in-loop-callsites x 500-node DFS) -> O(distinct callees). Byte-identical:
memoized==fresh unit test + goldens + DO diff."
```

---

### Task 2: Payoff measurement

Runbook (foreground short runs; the orchestrator handles the long detached 8020 full-default):
- [ ] slice-5400 full-default via peak_rss.py — baseline 292.6-304.2 s, d1 157.9 s of it. Expect d1 to collapse toward the distinct-callee count's cost.
- [ ] 8020 3-det (baseline 40.9-45.3 s) + DO (baseline 9.0-9.5 s) — regression guards.
- [ ] Report numbers to `.superpowers/sdd/w2c-payoff.md`; orchestrator launches the detached 8020 full-default (the FINISH bar — first time ever if it lands).

---

### Task 3: Capstone

- [ ] Measurements doc §8 (Wave-2c outcome), CHANGELOG, OUTSTANDING (d1 item resolution; re-rank what remains — B1/B2 and the Jacobi-block substrate as the next levers if the full-default now finishes and something else tops the profile).
- [ ] Commit docs; merge decision with the user.

## Self-review notes
- One semantic risk named and gated: policy callsite-dependence (Step 3 STOP condition). Everything else is the proven-additive transform.
- Bounds equivalence: today's walk from C already starts with a fresh 500-node budget per callsite; the canonical walk uses the identical budget — same tree, same cuts. Recorded in the design doc §3.
