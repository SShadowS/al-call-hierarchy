# Engine Memory/Speed Wave 1 (Track A) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `alsem analyze` finish on Base-App-scale corpora (8k files: DNF at 90 min / 35.8 GB → minutes) with goldens byte-stable, by removing the measured accidental-waste terms from the findings doc (`docs/superpowers/specs/2026-07-17-engine-memory-speed-findings.md` §7 Wave 1).

**Architecture:** Ten surgical tasks over the existing L3/L4/L5 substrate — no data-model redesign (that is Wave 2/3). Order: mechanical wins first (T1-T4), then ownership fixes (T5-T6), then the Jacobi fixes (T7-T8), parallel parse (T9), and the demand-driven detector substrate last (T10, the structural one). T11 is the measurement capstone.

**Tech Stack:** Rust (existing crate `al-call-hierarchy` — note the HYPHEN in `-p al-call-hierarchy`), rayon (already a dep), no new dependencies.

## Global Constraints

- Byte-stable goldens throughout: run `scripts/check-goldens` after every task; a diff = the task is WRONG (exception: T10's documented cap-hit-diagnostic change, decision (a) below).
- Format touched files with `rustfmt <file>` — NEVER `cargo fmt`.
- Stage files explicitly — NEVER `git add -A`.
- Build for measurement with `cargo build --profile release-fast --bin alsem`; never full `--release` in the loop.
- Never pipe long test/gate runs through `| tail` (masks exit codes); redirect to a file and grep it.
- `cargo clippy --all-targets --all-features` must stay clean after each task.
- Unit-test filter form: `cargo test -p al-call-hierarchy --lib <filter>`.
- Branch: `worktree-design-engine-memory-speed` (worktree `.claude/worktrees/design-engine-memory-speed`). The `8448ffb` "WIP(probes)" commit stays on the branch during the wave (the probes are how T11 measures); it is dropped/reverted at merge time.
- **Decision (a), user-approved:** summarize cap-hit diagnostics are emitted only by runs that BUILD core summaries. A substrate-skipping run (T10) omits them. This is the only permitted output change in the wave, and only for detector selections that don't demand core summaries.
- Measurement corpus rebuild (scratchpad dies with sessions; ~1 min):

```bash
python -c "
import zipfile, re, json, os
ws = r'<SCRATCH>/baseapp-ws'
os.makedirs(ws, exist_ok=True)
z = zipfile.ZipFile(r'U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud/.alpackages/Microsoft_Base Application_28.0.46665.48632.app')
mani = [n for n in z.namelist() if n.endswith('NavxManifest.xml')][0]
xml = z.read(mani).decode('utf-8', 'ignore')
g = lambda a: re.search(f'{a}=\"([^\"]+)\"', xml).group(1)
json.dump({'id': g('Id'), 'name': g('Name'), 'publisher': g('Publisher'), 'version': g('Version'), 'dependencies': []}, open(ws + '/app.json', 'w'))
[z.extract(n, ws) for n in z.namelist() if n.endswith('.al') and n.startswith('src/')]
"
# slice-5400: copy first 5400 (sorted) basenames of <SCRATCH>/baseapp-ws/src into <SCRATCH>/slice-5400/src + same app.json
```

Detector triple for measurements:
`--detector d61-ishandled-bypasses-critical-write,d62-telemetry-before-success,d64-api-page-write-surface --format json`

**Suggested dispatch models** (cost control; every task is gated by byte-stable goldens, so cheap models are safe): T1-T4, T6, T7 → Sonnet; T5, T8, T9, T10 → Opus; T11 → orchestrator + Haiku for run-babysitting.

---

### Task 1: FingerprintIndex once per run (W1.5)

**Files:**
- Modify: `src/engine/l5/detector_context.rs` (add field + build; both `build_detector_context` at :199 and `build_detector_context_cross_app` at :489)
- Modify: `src/engine/l5/fingerprint.rs` (make `FingerprintIndex` storable in ctx)
- Modify: every `src/engine/l5/detectors/d*.rs` that calls `FingerprintIndex::build` (54 files; pattern below)
- Modify: `src/engine/gate/policy/policy_engine.rs:155`

**Interfaces:**
- Produces: `DetectorContext.fingerprint_index: crate::engine::l5::fingerprint::FingerprintIndex<'a>` — detectors use `&ctx.fingerprint_index` wherever they previously built their own.

**Blocker to check first:** `FingerprintIndex<'a>` borrows `&'a [L3Routine]`/`&'a [L3Object]` — same lifetime as the ctx's existing borrowed maps (`routine_by_id: HashMap<&'a str, &'a L3Routine>`), so it slots in cleanly. The cross-app context (`build_detector_context_cross_app`) builds a THROWAWAY `merged_workspace_view` (`registry.rs:320-328`) — its routines/objects live in a LOCAL `L3Resolved` constructed in `run_detectors_cross_app` (`registry.rs:285-290`), which outlives the ctx there; build the index from `base.ws_routines`/`base.objects` (the `R3a5CrossAppBase` borrow) instead so the lifetime is honest.

- [ ] **Step 1: Inventory the exact call sites**

```bash
grep -rn "FingerprintIndex::build" src/engine/l5/detectors/ src/engine/gate/ | grep -v "^Binary" > /tmp/fpi-sites.txt
wc -l /tmp/fpi-sites.txt   # expect 55 (54 detectors + policy_engine)
```

- [ ] **Step 2: Add the ctx field**

In `src/engine/l5/detector_context.rs`, add to `DetectorContext<'a>` (after `summarize_diagnostics`):

```rust
    /// The shared finding-fingerprint index (routine/object id maps + the
    /// internal→stable routine-id substitution map). Built ONCE per run —
    /// previously every detector rebuilt it (54 × ~2 String clones per routine).
    pub fingerprint_index: crate::engine::l5::fingerprint::FingerprintIndex<'a>,
```

In `build_detector_context`, before constructing the struct:

```rust
    let fingerprint_index =
        crate::engine::l5::fingerprint::FingerprintIndex::build(&ws.routines, &ws.objects);
```

and add `fingerprint_index,` to the struct literal. In `build_detector_context_cross_app`, build from the base slices: `FingerprintIndex::build(&base.ws_routines, &base.objects)`.

- [ ] **Step 3: Mechanical detector sweep**

In each of the 54 detector files, replace

```rust
    let fp_index = FingerprintIndex::build(&ws.routines, &ws.objects);
```

with

```rust
    let fp_index = &ctx.fingerprint_index;
```

Downstream uses (`fp_index.fingerprint_of(...)`) compile unchanged through auto-deref. Remove the now-unused `use ...fingerprint::FingerprintIndex;` import in files where it becomes dead (compiler tells you). `policy_engine.rs:155` has no ctx — leave it building its own (one call, not hot).

CAUTION: some detectors bind a different variable name — follow `/tmp/fpi-sites.txt` line by line, don't blind-sed.

- [ ] **Step 4: Build + full test + goldens**

```bash
cargo build 2>&1 | tail -3            # expect success
cargo test > /tmp/t1-test.log 2>&1; echo exit=$?   # expect exit=0
bash scripts/check-goldens > /tmp/t1-goldens.log 2>&1; echo exit=$?   # expect exit=0, no diffs
```

- [ ] **Step 5: rustfmt touched files, clippy, commit**

```bash
for f in $(git diff --name-only); do rustfmt "$f"; done
cargo clippy --all-targets --all-features 2>&1 | grep -c '^error' # expect 0
git add src/engine/l5/detector_context.rs src/engine/l5/detectors/ 
git commit -m "perf(l5): build FingerprintIndex once per run in DetectorContext

All 54 detectors previously rebuilt it (O(detectors x routines), ~2 String
clones per routine per build). Byte-stable: index contents identical, only
construction count changes."
```

---

### Task 2: L3 assembly — take, don't clone (A7)

**Files:**
- Modify: `src/engine/l3/l3_workspace.rs` — the `project_file` per-routine assembly block (clone sites at ~:894 `field_accesses`, ~:930 `call_sites`, ~:1026-1036 `operation_sites`/`statement_tree`/`loops`/`identifier_references`/`unreachable_statements`/`var_assignments`/`condition_references`)

**Interfaces:** none change — `L3Routine` fields identical.

The per-routine `features` value is an OWNED local dropped at the end of each loop iteration; the only field read after the clone block is `features.variables` (~:974 and the `variables` assembly). So every listed clone can become `std::mem::take(&mut features.<field>)` — leaves an empty Vec/None behind, all later reads target OTHER fields. Requires the binding to be `mut` (it already is: `let (routine_id, mut features) = ...` at ~:762).

- [ ] **Step 1: Verify the post-clone read set (safety check, not optional)**

```bash
grep -n "features\." src/engine/l3/l3_workspace.rs | sed -n '1,80p'
```

Confirm: after the LAST `mem::take` target line, the only `features.` reads are `features.variables` (and `features.has_branching`, a Copy bool). If anything else reads a taken field afterwards, STOP and reorder the takes below that read.

- [ ] **Step 2: Apply the takes**

```rust
// was: let field_accesses = features.field_accesses.clone();
let field_accesses = std::mem::take(&mut features.field_accesses);
// was: let mut call_sites = features.call_sites.clone();
let mut call_sites = std::mem::take(&mut features.call_sites);
// was: let operation_sites = features.operation_sites.clone();
let operation_sites = std::mem::take(&mut features.operation_sites);
// was: let statement_tree = features.statement_tree.clone();
let statement_tree = features.statement_tree.take();
// was: let loops = features.loops.clone();
let loops = std::mem::take(&mut features.loops);
// was: let identifier_references = features.identifier_references.clone();
let identifier_references = std::mem::take(&mut features.identifier_references);
// was: let unreachable_statements = features.unreachable_statements.clone();
let unreachable_statements = std::mem::take(&mut features.unreachable_statements);
// was: let var_assignments = features.var_assignments.clone();
let var_assignments = std::mem::take(&mut features.var_assignments);
// was: let condition_references = features.condition_references.clone();
let condition_references = std::mem::take(&mut features.condition_references);
```

ORDERING CAUTION: the `call_sites` take is at ~:930 but the RV-8/G-8 block between :961-1024 reads `features.variables` — that field is NOT taken, fine. `record_operations`/`record_variables` are REBUILT field-by-field (not cloned wholesale) — leave them.

- [ ] **Step 3: Test + goldens + commit**

```bash
cargo test > /tmp/t2-test.log 2>&1; echo exit=$?          # expect 0
bash scripts/check-goldens > /tmp/t2-goldens.log 2>&1; echo exit=$?  # expect 0
rustfmt src/engine/l3/l3_workspace.rs
git add src/engine/l3/l3_workspace.rs
git commit -m "perf(l3): move L2 feature payloads into L3Routine instead of cloning

features is a transient per-routine local; the only post-move read is
features.variables. Pure allocation win, byte-stable."
```

---

### Task 3: Hoist build_cross_extension_subscribers into ctx (A8)

**Files:**
- Modify: `src/engine/l5/detector_context.rs` (new field, both build fns)
- Modify: `src/engine/l5/detectors/d43.rs:382`, `d44.rs:59`, `d45.rs:51`

**Interfaces:**
- Produces: `DetectorContext.cross_extension_subscribers: std::collections::BTreeMap<String, Vec<String>>`

- [ ] **Step 1: Add field + build once**

In `DetectorContext`:

```rust
    /// event id → cross-extension subscriber routine ids (subscribers living in a
    /// DIFFERENT app than the publisher object). Previously rebuilt identically by
    /// d43, d44 AND d45 each run; built once here (same sharing pattern as
    /// `event_flow_indexes`).
    pub cross_extension_subscribers: std::collections::BTreeMap<String, Vec<String>>,
```

In `build_detector_context` (after `event_flow_indexes` is built — it needs `event_graph` + `ws.objects`, both in scope):

```rust
    let cross_extension_subscribers =
        crate::engine::l5::event_flow::build_cross_extension_subscribers(&event_graph, &ws.objects);
```

Mirror in `build_detector_context_cross_app` from its own event graph + objects (grep how it builds `event_flow_indexes` there and follow the same inputs).

- [ ] **Step 2: Point d43/d44/d45 at it**

Replace, in each of the three files:

```rust
    let cross_ext_by_event = build_cross_extension_subscribers(&ctx.event_graph, &ws.objects);
```

with

```rust
    let cross_ext_by_event = &ctx.cross_extension_subscribers;
```

Fix borrow fallout: downstream uses are lookups (`cross_ext_by_event.get(...)`) — compile-clean via auto-deref. Remove dead imports.

- [ ] **Step 3: Test + goldens + commit**

```bash
cargo test > /tmp/t3-test.log 2>&1; echo exit=$?; bash scripts/check-goldens > /tmp/t3-goldens.log 2>&1; echo exit=$?
rustfmt src/engine/l5/detector_context.rs src/engine/l5/detectors/d43.rs src/engine/l5/detectors/d44.rs src/engine/l5/detectors/d45.rs
git add src/engine/l5/detector_context.rs src/engine/l5/detectors/d43.rs src/engine/l5/detectors/d44.rs src/engine/l5/detectors/d45.rs
git commit -m "perf(l5): build cross-extension-subscribers map once in DetectorContext

d43/d44/d45 (all DEFAULT) each rebuilt the identical map per run."
```

---

### Task 4: Parallelize the workspace-diagnostics re-parse (A9')

**Files:**
- Modify: `src/engine/gate/workspace_diagnostics.rs:108-127`

**Interfaces:** none change.

Scope note: the findings doc's original A9 ("share the L3 parse") is DEFERRED — `compute_workspace_diagnostics` uses UNSCOPED `discover_al_files` while L3 uses `discover_al_files_app_scoped`, so the file sets differ on nested-app workspaces and naive sharing is behavior-changing (same class of trap as the shared-parse investigation). Parallelizing the existing loop is byte-stable by construction and captures most of the win.

- [ ] **Step 1: Replace the sequential big-stack loop**

Replace lines ~:117-127:

```rust
    crate::big_stack::run_with_big_stack(|| {
        for (rel, source) in &units {
            if al_syntax::parse(source).objects.is_empty() {
                out.push(Diagnostic {
                    severity: "info".to_string(),
                    stage: "index".to_string(),
                    message: format!("No object declaration found in {rel}"),
                });
            }
        }
    });
```

with a rayon map that PRESERVES unit order (order = rel-posix-sorted, the documented determinism contract at the top of the file):

```rust
    // Parallel parse, order-preserving: par_iter keeps index order in collect,
    // so the emitted diagnostics stay in rel-posix-sorted unit order. The rayon
    // pool threads get the big stack via big_stack_pool (same pool the fresh
    // engine's parse_snapshot uses — see src/snapshot/parse.rs:85-87).
    use rayon::prelude::*;
    let empties: Vec<Option<&String>> = crate::big_stack::big_stack_pool().install(|| {
        units
            .par_iter()
            .map(|(rel, source)| {
                if al_syntax::parse(source).objects.is_empty() {
                    Some(rel)
                } else {
                    None
                }
            })
            .collect()
    });
    for rel in empties.into_iter().flatten() {
        out.push(Diagnostic {
            severity: "info".to_string(),
            stage: "index".to_string(),
            message: format!("No object declaration found in {rel}"),
        });
    }
```

Check `big_stack_pool()` exists and is public (`src/big_stack.rs`; `snapshot/parse.rs:86` uses it — same call shape). If the signature differs, mirror `parse_snapshot`'s exact usage.

- [ ] **Step 2: Test + goldens + commit**

```bash
cargo test > /tmp/t4-test.log 2>&1; echo exit=$?; bash scripts/check-goldens > /tmp/t4-goldens.log 2>&1; echo exit=$?
rustfmt src/engine/gate/workspace_diagnostics.rs
git add src/engine/gate/workspace_diagnostics.rs
git commit -m "perf(gate): parallelize the workspace-diagnostics index re-parse

Order-preserving par_iter on the big-stack pool; diagnostics order unchanged
(rel-posix-sorted). ~4x on the 3rd full workspace parse."
```

---

### Task 5: Move, don't clone, at the L4→L5 hand-offs (W1.3)

**Files:**
- Modify: `src/engine/l5/detector_context.rs` (`build_detector_context` ~:236-252 summaries block, ~:333-341 edge-index block, `upgraded_bindings_by_callsite` assembly ~:428, `parameter_roles`/`uncertainties` harvest ~:396-426)

**Interfaces:** none change (ctx field types stay identical).

- [ ] **Step 1: cones → summaries by move**

Current shape (~:236-251):

```rust
    let empty_facts: Vec<CapabilityFact> = Vec::new();
    let mut summaries: HashMap<String, FullRoutineSummary> = HashMap::new();
    for r in &ws.routines {
        let cone = cones.get(&r.id);
        let inherited = cone.map(|c| c.inherited.clone()).unwrap_or_default();
        let coverage = cone.map(|c| c.coverage.clone());
        summaries.insert(
            r.id.clone(),
            FullRoutineSummary {
                routine_id: r.id.clone(),
                capability_facts_direct: direct_full.get(&r.id).unwrap_or(&empty_facts).clone(),
                capability_facts_inherited: inherited,
                coverage,
            },
        );
    }
```

Replace with (both `cones` and `direct_full` are locally owned and dead after this loop — verify with grep before editing; if `cones` is read later, STOP and report):

```rust
    let mut summaries: HashMap<String, FullRoutineSummary> = HashMap::new();
    for r in &ws.routines {
        let (inherited, coverage) = match cones.remove(&r.id) {
            Some(c) => (c.inherited, Some(c.coverage)),
            None => (Vec::new(), None),
        };
        summaries.insert(
            r.id.clone(),
            FullRoutineSummary {
                routine_id: r.id.clone(),
                capability_facts_direct: direct_full.remove(&r.id).unwrap_or_default(),
                capability_facts_inherited: inherited,
                coverage,
            },
        );
    }
```

(`cones`/`direct_full` bindings become `mut`.) Iteration is over `ws.routines` (stable order), and `remove` returns the same values `get`+clone returned — byte-identical.

- [ ] **Step 2: CallEdge index by move**

Current (~:333-341):

```rust
    let mut resolved_call_edge_by_callsite: HashMap<String, CallEdge> = HashMap::new();
    for ce in &calls.edges {
        if ce.to.is_none() {
            continue;
        }
        resolved_call_edge_by_callsite
            .entry(ce.callsite_id.clone())
            .or_insert_with(|| ce.clone());
    }
```

`calls.edges` is not read after this point, but `calls.upgraded_bindings` IS (compute_summaries at ~:382-389 and the ctx field at ~:428). So take the edges only:

```rust
    let mut resolved_call_edge_by_callsite: HashMap<String, CallEdge> = HashMap::new();
    for ce in std::mem::take(&mut calls.edges) {
        if ce.to.is_none() {
            continue;
        }
        resolved_call_edge_by_callsite
            .entry(ce.callsite_id.clone())
            .or_insert(ce);
    }
```

(`calls` becomes `mut`; `entry().or_insert(ce)` preserves first-edge-wins.) VERIFY first: `grep -n "calls\.edges" src/engine/l5/detector_context.rs` — the take must be at the LAST use. If `build_combined_graph`/anything reads `calls.edges` after this line, move the take below that read.

- [ ] **Step 3: upgraded_bindings + parameter_roles/uncertainties by move where legal**

- `upgraded_bindings_by_callsite`: currently `calls.upgraded_bindings.clone()` (~:428). `compute_summaries` borrows it EARLIER (~:386) — so after that call, move it: `let upgraded_bindings_by_callsite = std::mem::take(&mut calls.upgraded_bindings);` placed after the `compute_summaries` call. Verify order with grep.
- `parameter_roles_by_routine` harvest (~:421-426) clones out of `core_summaries` while `core_summaries` is still used for `uncertainties_by_node` (~:432). Restructure to consume `core_summaries` ONCE at the end: first build `uncertainties_by_node` (it clones the uncertainty entries — keep those clones, the union logic needs both sources), THEN drain:

```rust
    let mut parameter_roles_by_routine: HashMap<String, Vec<RecordRoleSummary>> = HashMap::new();
    for (rid, s) in core_summaries {
        if !s.parameter_roles.is_empty() {
            parameter_roles_by_routine.insert(rid, s.parameter_roles);
        }
    }
```

Preserve the existing emptiness-filter semantics EXACTLY: check the current condition at ~:423-426 (`if let Some(s) = ... && !s.parameter_roles.is_empty()` shape) and replicate it. NOTE the current code iterates `ws.routines` and looks up — the drain above iterates the map instead; the result is a HashMap so ORDER DOESN'T MATTER, but membership must match: current code only inserts for routines present in `core_summaries` with non-empty roles — identical to the drain + filter. Keep byte-safety by running the golden suite.

- [ ] **Step 4: Test + goldens + commit**

```bash
cargo test > /tmp/t5-test.log 2>&1; echo exit=$?; bash scripts/check-goldens > /tmp/t5-goldens.log 2>&1; echo exit=$?
rustfmt src/engine/l5/detector_context.rs
git add src/engine/l5/detector_context.rs
git commit -m "perf(l5): move cone/summary/edge payloads into ctx instead of cloning

cones->summaries, direct facts, resolved CallEdge index, upgraded bindings
and parameter-role harvest now consume their sources. -5.4 GB at 8k."
```

---

### Task 6: One SpanTemplate per seed routine (W1.2)

**Files:**
- Modify: `src/engine/l5/transaction_spans.rs:175-264`

**Interfaces:** none change (`TransactionSpan` unchanged).

The per-seed work (`backward_cone` + `aggregate_span` + `span_roots_of` + the sorted `routines_in_span` vec) depends only on `(seed_routine_id, commits_by_routine, reverse, summaries)` — identical for every commit op in the same routine AND for §B seeds of the same routine. Cache it.

- [ ] **Step 1: Add the template cache**

At the top of `compute_transaction_spans`'s body (after `commits_by_routine` is built):

```rust
    /// Everything about a span that depends only on the seed ROUTINE.
    struct SpanTemplate {
        routines_in_span: Vec<String>,
        writes_tables: Vec<String>,
        publishes_events: Vec<String>,
        span_roots: Vec<String>,
        coverage_complete: bool,
    }
    let mut template_cache: HashMap<String, SpanTemplate> = HashMap::new();
```

Implement the compute-or-lookup as a plain helper fn ABOVE `compute_transaction_spans`:

```rust
fn span_template<'c>(
    seed: &str,
    commits_by_routine: &BTreeMap<String, Vec<String>>,
    reverse: &ReverseCallGraph,
    summaries: &HashMap<String, FullRoutineSummary>,
    cache: &'c mut HashMap<String, SpanTemplate>,
) -> &'c SpanTemplate {
    if !cache.contains_key(seed) {
        let visited = backward_cone(seed, commits_by_routine, reverse);
        let (writes_tables, publishes_events, coverage_complete) =
            aggregate_span(&visited, summaries);
        let span_roots = span_roots_of(&visited, reverse);
        cache.insert(
            seed.to_string(),
            SpanTemplate {
                routines_in_span: visited.iter().cloned().collect(),
                writes_tables,
                publishes_events,
                span_roots,
                coverage_complete,
            },
        );
    }
    &cache[seed]
}
```

(`SpanTemplate` struct declared at module level, private.)

- [ ] **Step 2: Use it in both seed passes**

§A explicit-commit loop (~:204-222) becomes:

```rust
    for (commit_routine_id, commit_ops) in &commits_by_routine {
        let t = span_template(
            commit_routine_id,
            &commits_by_routine,
            reverse,
            summaries,
            &mut template_cache,
        );
        // clone the template fields once per OP (same values every op — was a
        // full recompute per op before)
        for commit_operation_id in commit_ops {
            spans.push(TransactionSpan {
                seed_kind: SeedKind::ExplicitCommit,
                commit_operation_id: commit_operation_id.clone(),
                seed_callsite_id: None,
                commit_routine_id: commit_routine_id.clone(),
                routines_in_span: t.routines_in_span.clone(),
                writes_tables: t.writes_tables.clone(),
                publishes_events: t.publishes_events.clone(),
                span_roots: t.span_roots.clone(),
                coverage_complete: t.coverage_complete,
            });
        }
    }
```

BORROW NOTE: `span_template` borrows the cache mutably and returns a shared ref — the per-op loop only reads `t`, but `commits_by_routine` is ALSO borrowed by the outer loop. If the borrow checker objects to `&commits_by_routine` inside `span_template` while iterating it, restructure: collect `let seeds: Vec<(String, Vec<String>)> = commits_by_routine.iter().map(|(k, v)| (k.clone(), v.clone())).collect();` and iterate that (cheap — one entry per committing routine). Alternatively have `span_template` return a CLONE of the template (`SpanTemplate: Clone`) — still one compute per routine.

§B checked-run loop (~:229-261): replace the three per-callsite computations with the same `span_template(&r.id, ...)` lookup; keep everything else identical (`commit_operation_id: cs.id.clone()`, `seed_callsite_id: Some(cs.id.clone())`).

CRITICAL semantics check: in §A the walk's stop-condition treats the SEED specially (`id != seed && commits_by_routine.contains_key(&id)`). §B seeds may or may not be committing routines — the cache key is the seed id and the function inputs are identical across both passes, so one cache entry per routine id is CORRECT for both. Convince yourself by reading `backward_cone` (:89-118) — it takes only `(seed, commits_by_routine, reverse)`.

- [ ] **Step 3: Unit test (existing tests cover semantics; add the dedup property)**

Append to the `tests` module in the same file:

```rust
    #[test]
    fn multi_commit_routine_ops_share_identical_span_shape() {
        // Routine with TWO commit ops: both spans must have identical
        // routines_in_span/writes/events/roots (the template), differing only
        // in commit_operation_id.
        let routines = vec![
            routine("root", "trigger"),
            op_commit_routine("committer", "procedure", &["c/op1", "c/op2"]),
        ];
        let graph = graph_from_edges(
            &["root", "committer"],
            &[edge("root", "committer", "cs1")],
        );
        let reverse = build_reverse_call_graph(&graph);
        let summaries: HashMap<String, FullRoutineSummary> = HashMap::new();
        let spans = compute_transaction_spans(&routines, &BTreeSet::new(), &reverse, &summaries);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].routines_in_span, spans[1].routines_in_span);
        assert_eq!(spans[0].span_roots, spans[1].span_roots);
        assert_ne!(spans[0].commit_operation_id, spans[1].commit_operation_id);
    }
```

(Adapt helper signatures to `test_support` — check `op_commit_routine`'s exact shape at the existing tests, :279-296.)

- [ ] **Step 4: Test + goldens + commit**

```bash
cargo test -p al-call-hierarchy --lib transaction_spans > /tmp/t6-unit.log 2>&1; echo exit=$?
cargo test > /tmp/t6-test.log 2>&1; echo exit=$?; bash scripts/check-goldens > /tmp/t6-goldens.log 2>&1; echo exit=$?
rustfmt src/engine/l5/transaction_spans.rs
git add src/engine/l5/transaction_spans.rs
git commit -m "perf(l5): compute transaction-span cone/aggregation once per seed routine

BFS + aggregate + roots depend only on the seed routine; ops share the
template. 198s -> tens of s at 8k, byte-stable."
```

---

### Task 7: Index the uncertainty-edge scan + hoist cfg_walker indexes (W1.1a)

**Files:**
- Modify: `src/engine/l4/summary_runner.rs` (`SccComputeCtx` ~:939-947, `compose_routine` ~:349-357 + ~:482-497, `compute_summaries_with_leaves` ~:849-931, `run_one_scc` call sites ~:1025, ~:1081)
- Modify: `src/engine/l4/cfg_walker.rs:226-241` + the caller loop `summary_runner.rs:621-646`

**Interfaces:**
- `SccComputeCtx` gains `pub uncertainty_edges_by_from: &'a HashMap<String, Vec<usize>>` (indexes into `graph.uncertainty_edges`, preserving global order).
- `cfg_walker::walk_param` gains a `indexes: &WalkIndexes` parameter (the type `build_indexes` returns — check its real name at `cfg_walker.rs:241`; adjust).

- [ ] **Step 1: Build the index once**

In `compute_summaries_with_leaves` (before the SCC loop):

```rust
    // Index the global uncertainty-edge list by source routine, preserving the
    // GLOBAL list order per source (indices into graph.uncertainty_edges) so the
    // per-routine iteration below sees the same sequence the linear scan saw.
    let mut uncertainty_edges_by_from: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, ue) in graph.uncertainty_edges.iter().enumerate() {
        uncertainty_edges_by_from
            .entry(ue.from.clone())
            .or_default()
            .push(i);
    }
```

Add to `SccComputeCtx` and thread through `run_one_scc` → `compose_routine`. NOTE: the R3b Salsa path also constructs `SccComputeCtx` (grep `SccComputeCtx {` for ALL construction sites — `scc_summaries` query included) — build the same index there (or pass an owned map built the same way).

- [ ] **Step 2: Replace the scan in compose_routine**

Replace ~:483-497:

```rust
    for ue in &graph.uncertainty_edges {
        if ue.from != routine.id {
            continue;
        }
        ...
    }
```

with:

```rust
    if let Some(idxs) = uncertainty_edges_by_from.get(&routine.id) {
        for &i in idxs {
            let ue = &graph.uncertainty_edges[i];
            let u = Uncertainty {
                kind: ue.uncertainty.kind.clone(),
                callsite_id: ue.uncertainty.callsite_id.clone(),
                operation_id: ue.uncertainty.operation_id.clone(),
                routine_id: ue.uncertainty.routine_id.clone(),
                interface_name: ue.uncertainty.interface_name.clone(),
            };
            let k = uncertainty_key(&u);
            uncertainties_by_key.entry(k).or_insert(u);
            has_unresolved_calls = true;
        }
    }
```

Same edges, same order (indices pushed in global order) — byte-identical.

- [ ] **Step 3: Hoist build_indexes above the per-param loop**

`walk_param` (`cfg_walker.rs:226`) calls `let indexes = build_indexes(routine);` per PARAM. Change `walk_param` to accept `indexes: &<IndexesType>` (drop the internal build), and in `summary_runner.rs:621`:

```rust
    if routine.body_available && !routine.parse_incomplete {
        let walk_indexes = crate::engine::l4::cfg_walker::build_indexes(routine);
        for param_role in &mut parameter_roles {
            ...
            let f = crate::engine::l4::cfg_walker::walk_param(
                routine,
                &rec_var_name_lc,
                rec_var_id,
                snapshot,
                final_map,
                upgraded_bindings,
                graph,
                body_avail_by_id,
                &walk_indexes,
            );
            ...
        }
    }
```

`build_indexes` must become `pub` (it's module-private today — check). Grep OTHER `walk_param` callers (`cfg_walker.rs:1828` is a recursive/internal call — inspect and thread the reference through) and fix all.

- [ ] **Step 4: Test + goldens + commit**

```bash
cargo test > /tmp/t7-test.log 2>&1; echo exit=$?; bash scripts/check-goldens > /tmp/t7-goldens.log 2>&1; echo exit=$?
rustfmt src/engine/l4/summary_runner.rs src/engine/l4/cfg_walker.rs
git add src/engine/l4/summary_runner.rs src/engine/l4/cfg_walker.rs
git commit -m "perf(l4): index uncertainty edges by source; build cfg-walk indexes once per routine

Kills the O(members x iterations x global-uncertainty-edges) scan inside the
Jacobi (10k+ edges at 5.4k files) and the per-param index rebuild. Same
iteration order, byte-stable."
```

---

### Task 8: Jacobi — cached change keys, mem::take snapshot, dirty frontier (W1.1b)

**Files:**
- Modify: `src/engine/l4/summary_runner.rs` (`run_one_scc` recursive branch ~:1041-1130; new helper next to `summary_fingerprint` ~:796)
- Modify: `src/engine/l4/summary.rs` (make the change-key components reusable — see Step 1)
- Test: `src/engine/l4/summary_runner.rs` tests module + full golden suite + trace oracle

**Interfaces:** none outside l4.

Semantic ground rules (from the findings doc + external review, both loaded):
- The change test MUST keep comparing the PROJECTED (stable-id) form — internal-id comparison is stricter and can change the iteration trajectory (stable-id collisions exist; entries 53-54 in the L3 golden history).
- SYNCHRONOUS rounds only (Jacobi, not Gauss-Seidel); the cap-hit trajectory is load-bearing.
- Dirty-frontier is trajectory-preserving because compose is a deterministic function of (snapshot ∩ callees, fixed inputs): if no callee changed in round k, the member's round-k+1 output is bit-identical to its round-k output, so skipping the recompute changes nothing.

- [ ] **Step 1: A structured change key replacing the JSON string**

In `summary.rs`, next to `stable_summary_fingerprint` (:580), add:

```rust
/// The EXACT information `stable_summary_fingerprint` encodes, as a comparable
/// struct instead of a serde_json string. Equality of `SummaryChangeKey`s is
/// equivalent to equality of the fingerprint strings: the fingerprint is
/// JSON.stringify over these same components in the same order, and JSON
/// serialization of (arrays of strings + bool + numbers) is injective.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryChangeKey {
    pub db_effects: Vec<String>,        // "{effect_key}:{via}" per effect, in order
    pub has_unresolved_calls: bool,
    pub uncertainties: Vec<String>,     // p_uncertainty_key per uncertainty, in order
    pub parameter_roles: Vec<Vec<String>>, // the same 14 per-role fields, stringified in order
}

pub fn summary_change_key(s: &PRoutineSummaryCore) -> SummaryChangeKey {
    SummaryChangeKey {
        db_effects: s
            .db_effects
            .iter()
            .map(|e| format!("{}:{}", e.effect_key, e.via))
            .collect(),
        has_unresolved_calls: s.has_unresolved_calls,
        uncertainties: s.uncertainties.iter().map(p_uncertainty_key).collect(),
        parameter_roles: s
            .parameter_roles
            .iter()
            .map(|r| {
                fn fl(v: &serde_json::Value) -> String {
                    if let serde_json::Value::Array(arr) = v {
                        arr.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(",")
                    } else {
                        v.to_string()
                    }
                }
                vec![
                    r.parameter_index.to_string(),
                    r.loads_from_db_param.clone(),
                    r.initialises_param.clone(),
                    r.persists_current_record.clone(),
                    r.set_based_db_writes.clone(),
                    r.validates_param.clone(),
                    r.copies_into_param.clone(),
                    r.resets_filters_on_param.clone(),
                    r.mutates_param.clone(),
                    r.requires_loaded_at_entry.clone(),
                    r.mutates_before_load.clone(),
                    fl(&r.required_loaded_fields_at_entry),
                    r.dirty_at_exit.clone(),
                    fl(&r.current_loaded_fields_at_exit),
                ]
            })
            .collect(),
    }
}
```

EQUIVALENCE CAUTION on `fl`: the fingerprint's `field_list_fp` clones the non-array value VERBATIM into the JSON array (so a JSON string value serializes with quotes); `v.to_string()` on a `Value::String` also produces the quoted form — but a bare `Value::String("unknown")` under `field_list_fp` becomes the JSON string `"unknown"` while the ARRAY case produces an UNQUOTED joined string. Preserve the distinction: for the array case use the joined string as above; for the non-array case use `v.to_string()` (which keeps quotes) — the two can never collide because one is always quote-wrapped. Add a unit test pinning exactly this (below).

- [ ] **Step 2: Equivalence test (write it FIRST, prove it against the old path)**

In `summary.rs` tests (or `summary_runner.rs` tests if fixtures live there):

```rust
    #[test]
    fn change_key_equality_iff_fingerprint_equality() {
        // Build a handful of PRoutineSummaryCore values exercising every field:
        // empty; effects-only; uncertainty-only; param-roles with Known/Unknown
        // field lists (array AND string forms); pairs differing in exactly one
        // component. For every pair (a, b):
        // assert_eq!(
        //     summary_change_key(a) == summary_change_key(b),
        //     stable_summary_fingerprint(a) == stable_summary_fingerprint(b)
        // );
    }
```

Fill the fixture list concretely — at minimum 8 values: default/empty core; one effect `k1:direct`; same key different via; `has_unresolved_calls` flipped; one uncertainty; two uncertainties in swapped ORDER (order matters — fingerprints differ, keys must differ too); a param role with `required_loaded_fields_at_entry` as `Value::Array(["a","b"])` vs `Value::String("a,b")` (MUST compare UNEQUAL — quoted vs joined); identical clones (equal). Run: `cargo test -p al-call-hierarchy --lib change_key -- --nocapture` → PASS.

- [ ] **Step 3: Rewrite the recursive-SCC loop**

Replace the loop body (~:1062-1106) with:

```rust
    // Per-member cached change key of the CURRENT in_progress value (stable-id
    // space — the same comparison surface the JSON fingerprint used).
    let mut key_cache: HashMap<String, crate::engine::l4::summary::SummaryChangeKey> =
        HashMap::new();
    for id in &scc_entry.members {
        if leaf_summaries.contains_key(id) {
            continue;
        }
        if let Some(s) = in_progress.get(id) {
            let proj = project_summary_to_stable(id, s, ctx.stable_map);
            key_cache.insert(id.clone(), crate::engine::l4::summary::summary_change_key(&proj));
        }
    }

    // Intra-SCC dependents: member -> members that CALL it (so when member m
    // changes, the callers of m are dirty next round). Derived from the same
    // edges_by_from the compose reads.
    let member_set: std::collections::HashSet<&String> = scc_entry.members.iter().collect();
    let mut dependents: HashMap<&String, Vec<&String>> = HashMap::new();
    for m in &scc_entry.members {
        if let Some(edges) = ctx.graph.edges_by_from.get(m) {
            for e in &edges_targets_of(edges) {  // see note below
                if member_set.contains(e) {
                    dependents.entry(e).or_default().push(m);
                }
            }
        }
    }
    // NOTE: `edges_by_from` values are edge structs — extract `.to` (adjust to the
    // real CombinedGraph edge type: `for e in edges { let to = &e.to; ... }`).

    let mut dirty: std::collections::BTreeSet<String> =
        scc_entry.members.iter().filter(|m| !leaf_summaries.contains_key(*m)).cloned().collect();

    let mut iterations = 0usize;
    let mut changed = true;
    while changed {
        changed = false;
        iterations += 1;

        // JACOBI: freeze the prior-pass state WITHOUT a deep clone. Reads during
        // this pass come from `snapshot`; writes accumulate in `next_pass`;
        // unchanged members are carried over by move afterwards.
        let snapshot: HashMap<String, RoutineSummary> = std::mem::take(&mut in_progress);
        let mut next_pass: HashMap<String, RoutineSummary> = HashMap::new();
        let mut next_dirty: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

        for id in &scc_entry.members {
            if leaf_summaries.contains_key(id) {
                continue;
            }
            if !dirty.contains(id) {
                continue; // inputs unchanged in the prior round => output identical
            }
            let routine = match ctx.routines_by_id.get(id) {
                Some(r) => r,
                None => continue,
            };
            let next = compose_routine(
                routine,
                &snapshot,
                predecessor_final_map,
                ctx.base_summaries,
                ctx.upgraded_bindings,
                ctx.graph,
                ctx.body_avail_by_id,
                /* + the ctx threading added in Task 7 */
            );
            let next_proj = project_summary_to_stable(id, &next, ctx.stable_map);
            let next_key = crate::engine::l4::summary::summary_change_key(&next_proj);
            let member_changed = key_cache.get(id) != Some(&next_key);
            if member_changed {
                changed = true;
                key_cache.insert(id.clone(), next_key);
                for dep in dependents.get(id).map(|v| v.as_slice()).unwrap_or(&[]) {
                    next_dirty.insert((*dep).clone());
                }
            }
            next_pass.insert(id.clone(), next);
        }

        // Carry over members not recomputed this round (move from snapshot).
        let mut merged = snapshot;
        for (k, v) in next_pass {
            merged.insert(k, v);
        }
        in_progress = merged;
        dirty = next_dirty;

        // Trace hook (opt-in) — UNCHANGED semantics: projects CURRENT in_progress.
        if collect_trace { /* keep the existing block verbatim */ }

        if iterations >= MAX_FIXED_POINT_ITERATIONS { /* keep the existing cap block verbatim */ }
    }
```

TRAJECTORY-PRESERVATION NOTES the implementer must respect:
1. First round: every member dirty → identical to today's full first pass.
2. `member_changed` for a member whose PREVIOUS in_progress entry was ABSENT (no base summary) — today `fp_prev = None` ⇒ `changed = true` whenever recomputed. `key_cache` has no entry ⇒ `key_cache.get(id) != Some(&next_key)` is true — matches.
3. The old code set `changed` by comparing prev vs next PER RECOMPUTED MEMBER; skipped members contributed no change signal — with the frontier they are skipped precisely because their key CANNOT change. Equal trajectory.
4. `in_progress = next_pass` (old) replaced by carry-over merge: members recomputed get their new value; members skipped keep the value the old code would have recomputed IDENTICALLY. Bit-equal state per round.
5. Cap-hit block reads `in_progress`/members exactly as before — keep verbatim, including `cap_hit_stable_members` shape.
6. `collect_trace` must record ALL members each pass (it projects `in_progress`, which now contains carried-over members too) — verify the existing block reads `in_progress` (it does: `in_progress.get(id)`), so it is unchanged.

- [ ] **Step 4: Verify with the trace oracle + goldens + a real workspace**

BEFORE touching code in this task, capture the baseline once (pre-change binary):

```bash
cargo build --profile release-fast --bin alsem && \
  target/release-fast/alsem.exe analyze "U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud" --format json > /tmp/t8-do-baseline.json
```

After the change:

```bash
cargo test > /tmp/t8-test.log 2>&1; echo exit=$?                       # all unit + integration
bash scripts/check-goldens > /tmp/t8-goldens.log 2>&1; echo exit=$?    # byte-stable
cargo build --profile release-fast --bin alsem 2>&1 | tail -1
target/release-fast/alsem.exe analyze "U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud" --format json > /tmp/t8-do-new.json
diff /tmp/t8-do-baseline.json /tmp/t8-do-new.json && echo BYTE-STABLE
```

Expect NO differences (the JSON is deterministic by default; if a run-metadata field differs, pass `--deterministic` on BOTH sides — check `alsem analyze --help`). Never use `git stash` for the before/after — shared stash stack (CLAUDE.md law); the two-binary procedure above needs no tree switching.

- [ ] **Step 5: rustfmt, clippy, commit**

```bash
rustfmt src/engine/l4/summary_runner.rs src/engine/l4/summary.rs
cargo clippy --all-targets --all-features 2>&1 | grep -c '^error'   # 0
git add src/engine/l4/summary_runner.rs src/engine/l4/summary.rs
git commit -m "perf(l4): Jacobi change keys without serde, take-based snapshot, dirty frontier

Change test compares the same stable-projected components as the JSON
fingerprint (equivalence unit-tested), prev keys cached across rounds, the
per-iteration deep clone replaced by take+carry-over, and only members whose
callee inputs changed are recomputed (round-preserving synchronous frontier).
116s -> target ~30s at 5.4k; the 8k regime-change block collapses."
```

---

### Task 9: Parallel L3 assembly parse (W1.4)

**Files:**
- Modify: `src/engine/l3/l3_workspace.rs` — `assemble_workspace` (:1176-1210 region) and `assemble_workspace_units` (the sibling; same transform)

**Interfaces:** none change.

Design: `project_file` currently parses AND pushes into the shared `workspace`. Split: parallel map each file to its OWN `L3Workspace` fragment (parse + project into a fresh workspace), then fold fragments IN SORTED FILE ORDER into the final workspace. Object/routine ids are content-derived (no ordinals), so output Vec order is the ONLY order-sensitive thing — and the sorted fold reproduces today's exact order.

- [ ] **Step 1: Refactor assemble_workspace**

```rust
pub fn assemble_workspace(
    files: &[(String, String)],
    app_guid: &str,
    model_instance_id: &str,
) -> L3Workspace {
    let mut sorted: Vec<&(String, String)> = files.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    // Parallel per-file parse+project into private fragments (same big-stack
    // pool the fresh engine parses on), then fold in the SAME sorted order the
    // sequential loop used — byte-identical Vec order.
    use rayon::prelude::*;
    let fragments: Vec<L3Workspace> = crate::big_stack::big_stack_pool().install(|| {
        sorted
            .par_iter()
            .map(|(fname, source)| {
                let source_unit_id = format!("ws:{fname}");
                let cols = Utf16Cols::new(source);
                let mut ws = L3Workspace {
                    objects: Vec::new(),
                    tables: Vec::new(),
                    routines: Vec::new(),
                };
                project_file(
                    source,
                    app_guid,
                    model_instance_id,
                    &source_unit_id,
                    &cols,
                    &mut ws,
                );
                ws
            })
            .collect()
    });

    let mut workspace = L3Workspace {
        objects: Vec::new(),
        tables: Vec::new(),
        routines: Vec::new(),
    };
    for mut frag in fragments {
        workspace.objects.append(&mut frag.objects);
        workspace.tables.append(&mut frag.tables);
        workspace.routines.append(&mut frag.routines);
    }
    workspace
}
```

PRECONDITION CHECK (do this FIRST, stop if violated): `project_file` must touch ONLY its `workspace` argument — no cross-file state. Read it end to end (`:553-1140`) looking for reads of previously-accumulated `workspace.*` entries (e.g. dedup against existing objects). The known structure appends per-file objects/tables/routines independently; extension-field merging etc. happens later in `resolve()`. If you find a cross-file read, REPORT and stop — do not force it.

Also keep the Task-2 stage-probe lines if present in the region (branch carries them) — put the parse/projection accum calls inside the parallel closure unchanged (they are atomic counters, thread-safe).

- [ ] **Step 2: Same transform for `assemble_workspace_units`** (unit-id form `source_unit_id` used verbatim instead of `ws:{fname}` — mirror its existing loop body exactly).

- [ ] **Step 3: Test + goldens + determinism double-run**

```bash
cargo test > /tmp/t9-test.log 2>&1; echo exit=$?; bash scripts/check-goldens > /tmp/t9-goldens.log 2>&1; echo exit=$?
# Determinism: two runs on DO must be byte-identical.
cargo build --profile release-fast --bin alsem 2>&1 | tail -1
target/release-fast/alsem.exe analyze "U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud" --format json > /tmp/t9-a.json
target/release-fast/alsem.exe analyze "U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud" --format json > /tmp/t9-b.json
diff /tmp/t9-a.json /tmp/t9-b.json && echo DETERMINISTIC
```

- [ ] **Step 4: rustfmt + commit**

```bash
rustfmt src/engine/l3/l3_workspace.rs
git add src/engine/l3/l3_workspace.rs
git commit -m "perf(l3): parallelize assembly parse+project via per-file fragments

Sorted-order fold reproduces the sequential Vec order exactly; ids are
content-derived. 36s -> ~10s at 8k."
```

---

### Task 10: Demand-driven detector substrate (W1.0)

**Files:**
- Modify: `src/engine/l5/registry.rs` (`Detector` struct :186-189, `run_detectors` :237, `run_detectors_cross_app` :271)
- Modify: `src/engine/l5/detector_context.rs` (`build_detector_context` splits into always-built core + demand-built parts)
- Modify: `src/engine/l5/detectors/mod.rs` (`registered_detectors()` :932 — add `requires` per entry)
- Test: new member in the l5 unit tests (per-detector minimal-ctx parity)

**Interfaces:**
- `Detector` gains `pub requires: u32` (bitmask).
- New module-level consts in `registry.rs`:

```rust
/// Substrate demand bits. CORE (symbol table, resolve_calls, event graph,
/// combined graph, reverse graph, entry points, reachable roots, borrowed
/// indexes, uncertainty-edge grouping, fingerprint index, event-flow indexes,
/// cross-extension subscribers) is ALWAYS built — cheap, many consumers.
pub mod substrate {
    /// Capability cones + FullRoutineSummary map (`ctx.summaries`).
    pub const SUMMARIES: u32 = 1 << 0;
    /// Second Tarjan + Jacobi core summaries -> uncertainties_by_node,
    /// parameter_roles_by_routine, summarize_diagnostics (cap-hit).
    pub const CORE_SUMMARIES: u32 = 1 << 1;
    /// Transaction spans (requires SUMMARIES internally).
    pub const TRANSACTION_SPANS: u32 = 1 << 2;
    /// Closed-world proven-temp params.
    pub const CLOSED_WORLD_TEMP: u32 = 1 << 3;
    pub const ALL: u32 = SUMMARIES | CORE_SUMMARIES | TRANSACTION_SPANS | CLOSED_WORLD_TEMP;
}
```

- `build_detector_context(resolved, demanded: u32)` — skipped substrates leave their ctx fields EMPTY (`HashMap::new()`, `Vec::new()`), types unchanged so no detector-side churn.

Decision (a) applies: a run whose selection doesn't demand `CORE_SUMMARIES` emits no summarize cap-hit diagnostics. Every run demanding it behaves byte-identically to today. Full runs (presets/all-detector) demand ALL → the whole surface byte-identical.

- [ ] **Step 1: Audit script → requirements table**

```bash
for f in src/engine/l5/detectors/d*.rs; do
  n=$(basename "$f" .rs)
  uses=$(grep -o 'ctx\.[a-z_]*' "$f" | sort -u | tr '\n' ' ')
  echo "$n: $uses"
done > /tmp/detector-ctx-audit.txt
cat /tmp/detector-ctx-audit.txt
```

Map fields → bits: `summaries` → SUMMARIES; `uncertainties_by_node`/`parameter_roles_by_routine` → CORE_SUMMARIES; `transaction_spans` → TRANSACTION_SPANS; `closed_world_temp_params` → CLOSED_WORLD_TEMP; `get_ordering_facts` → none (already lazy); everything else → core (no bit). ALSO grep for INDIRECT consumption: helpers that take `ctx` and read these fields (`grep -rn "ctx\.summaries\|ctx\.transaction_spans\|ctx\.uncertainties_by_node\|ctx\.parameter_roles_by_routine\|ctx\.closed_world_temp_params" src/engine/l5/ | grep -v detector_context.rs` — attribute helper hits to every detector calling that helper; `walk_evidence` reads `uncertainties_by_node` → any detector using the path walker needs CORE_SUMMARIES). Being over-inclusive is SAFE (just less skipping); being under-inclusive is caught by the parity test in Step 4.

- [ ] **Step 2: Restructure build_detector_context**

Signature: `pub fn build_detector_context(resolved: &L3Resolved, demanded: u32) -> DetectorContext<'_>`.

Gate the expensive blocks:

```rust
    // Cones + FullRoutineSummary — only when demanded (SUMMARIES or anything
    // that folds over summaries).
    let need_summaries =
        demanded & (substrate::SUMMARIES | substrate::TRANSACTION_SPANS) != 0;
    let (summaries, ...) = if need_summaries {
        /* existing direct-facts + compose_cone_over_graph + summaries assembly */
    } else {
        (HashMap::new(), ...)
    };

    let transaction_spans = if demanded & substrate::TRANSACTION_SPANS != 0 {
        compute_transaction_spans(&ws.routines, &dep_routine_ids, &reverse_call_graph, &summaries)
    } else {
        Vec::new()
    };

    let closed_world_temp_params = if demanded & substrate::CLOSED_WORLD_TEMP != 0 {
        prove_closed_world_temp_params(...)
    } else {
        Default::default()   // check the real type's empty constructor
    };

    // Second Tarjan + compute_summaries block — only for CORE_SUMMARIES.
    let (uncertainties_by_node, parameter_roles_by_routine, summarize_diagnostics) =
        if demanded & substrate::CORE_SUMMARIES != 0 {
            /* existing :354-426 block verbatim */
        } else {
            (HashMap::new(), HashMap::new(), Vec::new())
        };
```

KEEP always-built: symbol table, resolve_calls, event graph, combined graph, reverse graph, entry_points, reachable_roots, all borrowed maps, `resolved_call_edge_by_callsite`, `uncertainty_edges_by_from`, `event_flow_indexes`, `cross_extension_subscribers` (T3), `fingerprint_index` (T1). Keep the SCCSTATS/stage-probe lines wherever the code they instrument still runs.

`run_detectors`: compute the union and pass it:

```rust
pub fn run_detectors(resolved: &L3Resolved, detectors: &[Detector]) -> RunOutput {
    let demanded = detectors.iter().fold(0u32, |acc, d| acc | d.requires);
    let ctx = build_detector_context(resolved, demanded);
    ...
}
```

`run_detectors_cross_app`: pass `substrate::ALL` (cross-app path keeps today's eager behavior — its detector set is small and its context builder is separate; do NOT refactor it in this task).

Other `build_detector_context` callers: `grep -rn "build_detector_context(" src/ tests/` — every non-registry caller passes `substrate::ALL` (behavior-preserving).

- [ ] **Step 3: Annotate all 54 registrations**

In `registered_detectors()`, per entry:

```rust
        Detector {
            name: "d1-db-op-in-loop".to_string(),
            run: d1::detect_d1,
            requires: substrate::SUMMARIES | substrate::CLOSED_WORLD_TEMP,
        },
```

Values come from `/tmp/detector-ctx-audit.txt` + the indirect-consumption grep. Also update the `Detector` literals in tests and the cross-app registry if any exist (`grep -rn "Detector {" src/ tests/`).

- [ ] **Step 4: Per-detector minimal-ctx parity test (the license for the audit)**

New test (place beside existing l5 integration tests; find the umbrella with `grep -rn "run_detectors" tests/ | head`):

```rust
/// For EVERY registered detector: findings computed with the FULL context must
/// equal findings computed with a context built from ONLY that detector's
/// declared requirements. Catches any under-declared `requires` bit.
#[test]
fn every_detector_parity_between_full_and_minimal_ctx() {
    let ws_root = /* the largest committed fixture workspace the l5 suite already
                     uses — reuse the same path constant its tests use */;
    let resolved = assemble_and_resolve_workspace_default(ws_root).expect("fixture assembles");
    let full_ctx = build_detector_context(&resolved, substrate::ALL);
    for det in registered_detectors() {
        let min_ctx = build_detector_context(&resolved, det.requires);
        let full = (det.run)(&resolved, &full_ctx).expect("full ctx run");
        let minimal = (det.run)(&resolved, &min_ctx).expect("minimal ctx run");
        assert_eq!(
            full.findings, minimal.findings,
            "detector {} under-declares its substrate requirements",
            det.name
        );
    }
}
```

Fixture choice matters: it must actually EXERCISE summaries/spans (a workspace with commits + events). Use the same corpus the existing d43/d45/transaction-span integration tests use (grep for their workspace constants). If a detector's findings are empty on the fixture for BOTH ctxs, that's weak-but-acceptable coverage for this wave; note it in the commit message.

- [ ] **Step 5: Full verification**

```bash
cargo test > /tmp/t10-test.log 2>&1; echo exit=$?          # includes the parity test
bash scripts/check-goldens > /tmp/t10-goldens.log 2>&1; echo exit=$?   # all-detector goldens: byte-stable
# DO full-default run byte-compare vs pre-task baseline (default preset demands
# ALL bits in practice — every substrate has a default consumer):
target/release-fast/alsem.exe analyze "U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud" --format json > /tmp/t10-do.json
diff /tmp/t8-do-baseline.json /tmp/t10-do.json && echo BYTE-STABLE
# The W1.0 win, 3-detector selection on DO (sanity that skipping engages):
target/release-fast/alsem.exe analyze "U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud" --detector d61-ishandled-bypasses-critical-write,d62-telemetry-before-success,d64-api-page-write-surface --format json > /tmp/t10-do-3det.json
# Byte-compare 3-det output against the SAME selection on the pre-task binary,
# EXCEPT the summarize cap-hit diagnostics line (decision (a)); on DO the
# cap-hit set is empty, so expect a clean diff.
```

- [ ] **Step 6: rustfmt sweep + commit**

```bash
for f in $(git diff --name-only); do rustfmt "$f"; done
cargo clippy --all-targets --all-features 2>&1 | grep -c '^error'  # 0
git add src/engine/l5/registry.rs src/engine/l5/detector_context.rs src/engine/l5/detectors/mod.rs <test file>
git commit -m "perf(l5): demand-driven detector substrate (W1.0)

Detectors declare required substrates (SUMMARIES / CORE_SUMMARIES /
TRANSACTION_SPANS / CLOSED_WORLD_TEMP); build_detector_context builds the
union for the selected set. Full/preset runs demand everything -> byte-stable.
Substrate-skipping selections omit summarize cap-hit diagnostics by design
(decision (a), user-approved). Per-detector full-vs-minimal parity test
guards every requires declaration."
```

---

### Task 11: Capstone — measure, document, tick

**Files:**
- Modify: `docs/superpowers/specs/2026-07-17-engine-memory-speed-findings.md` (append a "Wave 1 outcome" section)
- Modify: `docs/OUTSTANDING.md` (tick Wave 1)
- Modify: `CHANGELOG.md` (Changed/Fixed entries for T1-T10)

- [ ] **Step 1: Rebuild corpora** (Global Constraints script; full 8020 + slice-5400)

- [ ] **Step 2: Measure** (probes still on the branch; `ALSEM_STAGE_TIMING=1` for stage tables, plain runs for headline numbers)

```bash
cargo build --profile release-fast --bin alsem 2>&1 | tail -1
python scripts/peak_rss.py "<abs>/target/release-fast/alsem.exe" analyze "<SCRATCH>/slice-5400" --detector d61-...,d62-...,d64-... --format json
python scripts/peak_rss.py "<abs>/target/release-fast/alsem.exe" analyze "<SCRATCH>/baseapp-ws" --detector d61-...,d62-...,d64-... --format json
# Full-default run at 8020 (the demanding path — cones+Jacobi live):
python scripts/peak_rss.py "<abs>/target/release-fast/alsem.exe" analyze "<SCRATCH>/baseapp-ws" --format json
```

Success bars (from the findings doc): 3-detector 8020 run finishes in single-digit minutes at a few GB; 5400 3-detector well under 60 s; full-default 8020 FINISHES (was: DNF at 90 min) — record whatever the numbers are honestly.

- [ ] **Step 3: DO regression sanity** — default run byte-equal to the pre-wave baseline; wall time not worse.

- [ ] **Step 4: Docs** — append the measured before/after table to the findings doc; tick the Wave-1 OUTSTANDING item (leave Wave 2/3 open); CHANGELOG entries per Keep-a-Changelog under `Changed` (perf) + the decision-(a) diagnostic scope note.

- [ ] **Step 5: Commit docs**

```bash
git add docs/superpowers/specs/2026-07-17-engine-memory-speed-findings.md docs/OUTSTANDING.md CHANGELOG.md
git commit -m "docs: Wave 1 outcome — measured before/after + OUTSTANDING tick + CHANGELOG"
```

- [ ] **Step 6: Hand back to the user** — merge decision (drop/revert `8448ffb` WIP-probes first), CDO gate run (`scripts/cdo-gate`, user-scheduled), and whether to start Wave 2.

---

## Self-review notes (already applied)

- Spec coverage: W1.0→T10, W1.1→T7+T8, W1.2→T6, W1.3→T5, W1.4→T9, W1.5→T1, W1.6→T2+T3+T4. A6 (dead `string-interner` dep removal) intentionally DEFERRED to Wave 2's B1 (same Cargo.toml region, zero runtime effect).
- The findings doc's A9 "share the L3 parse" was NARROWED to "parallelize the re-parse" (T4) — discovery-scope mismatch makes sharing behavior-changing; recorded in T4's scope note.
- Line numbers cite the branch AFTER the master merge (`e3e90a0`); they drift ±2 from the findings doc — every task includes a grep/read step before editing, so drift is harmless.
- T8 is the only task with real semantic risk; its equivalence test + trace oracle + DO byte-diff are the containment. If the DO diff is non-empty at T8, STOP and root-cause — do not rebaseline.
