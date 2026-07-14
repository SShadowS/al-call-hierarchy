# Witness/Digest Optimization Implementation Plan (alsem ordering path)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cut `alsem analyze --preset transaction-integrity` on CDO from 88 s toward <30 s with byte-identical output, by removing measured redundancy in `digest_one_root` (2.3× duplicate effect facts + an O(F²) merge loop) and cheapening `compose_snapshot`.

**Architecture:** Three independent, byte-identical-by-construction cuts, ranked by the
re-profile in `.superpowers/sdd/alsem-parallel/witness-investigation.md`: the digest
par-loop is ~76 s of the 88 s and splits ~50% witness BFS / ~48% per-fact merge+projection;
both halves pay a measured 2.32× duplicate-effect-fact factor, and the merge is O(F²) with
repeated `query_hops_json` serialization concentrated in the 9 tail Page roots (1000–1552
facts each) that bound the parallel wall. Fix order: (1) O(F) effect-map index + memoized
path JSON (mechanical), (2) merge-identity effect-fact dedup (the 2.3× lever, with an
invariance proof), (3) `compose_snapshot` cached-key sort + parallel fact emission.
Explicitly REJECTED by measurement — do not implement: `valid_nodes` memo (0.4% of cost),
cross-root cone/suffix sharing (byte-unstable, payoff ≤ dedup).

**Tech Stack:** Rust, rayon (already used in `digest_query`).

## Global Constraints

- Branch: `feat/witness-digest-opt` off master `ff124fb`.
- BYTE-IDENTICAL output everywhere: full `cargo test` passes with ZERO golden diffs (digest / ordering / scoped-guarantee goldens are the oracle) — never rebaseline; a golden diff means the change is wrong.
- CDO byte-compare per code task: run `alsem analyze 'u:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud' --preset transaction-integrity --deterministic --format json` (the `--deterministic` flag pins the `generatedAt` timestamp) before/after and `fc.exe /b` the outputs — must be identical.
- `rustfmt <file>` per touched file, NEVER `cargo fmt`. `cargo clippy --all-targets --all-features` clean per task. CHANGELOG.md per code task.
- Commits: stage only intended paths (never `git add -A`; never stage `.panel/`, `demo-out/`, `finish-cleanup.ps1`, `finish-t1-cleanup.ps1`, `scripts/peak_rss.py`). Trailer: `Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>`.
- Baseline (median, CDO): default analyze 7.31 s; transaction-integrity preset 88.4 s (compose_snapshot 3.1 s, digest par-loop ~76 s, d48 3.8 s).

---

### Task 1: O(F) effect-map index + memoized path JSON in `digest_one_root`

**Files:**
- Modify: `src/engine/l5/digest.rs` (`digest_one_root` ~2097-2420: `AccumulatedEffect` struct, the merge arm at ~2230-2290, the insert arm)
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: existing `query_hops_json(hops: &[QueryWitnessHop]) -> String` (digest.rs:1628), unchanged.
- Produces: no signature changes. `AccumulatedEffect` gains `all_path_jsons: Vec<String>` (parallel to `all_paths`); a local `effect_index: HashMap<String, usize>` shadows the insertion-ordered `effect_map` Vec. Task 2 builds on this file state but touches only the loop head.

- [ ] **Step 1: Capture the CDO baseline output for byte-compare**

```powershell
cd U:\Git\al-call-hierarchy; cargo build --release --bin alsem
.\target\release\alsem.exe analyze 'u:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud' --preset transaction-integrity --deterministic --format json 1>$env:TEMP\witness-base.json 2>$null
```
Also time it with a Stopwatch and record (expect ~88 s).

- [ ] **Step 2: Implement the index + memoized JSON**

In `digest_one_root`:

(a) Extend the local `AccumulatedEffect` struct with a parallel JSON cache:

```rust
            /// `query_hops_json` of each `all_paths[i]`, parallel to `all_paths` —
            /// computed once at projection/merge and reused for every later merge's
            /// sort tiebreak + dedupe (removes the repeated re-serialization the
            /// giant tail roots paid; see witness-investigation.md §3).
            all_path_jsons: Vec<String>,
```

(b) Next to `let mut effect_map: Vec<(String, AccumulatedEffect)> = Vec::new();` add:

```rust
        // O(1) key → position index over effect_map (same keys, same positions —
        // the Vec keeps insertion order for output; this only replaces the O(F)
        // `iter().position()` scan that made the loop O(F²) on 1500-fact roots).
        let mut effect_index: HashMap<String, usize> = HashMap::new();
```

(c) After the `projected_paths` / `path_conds` computation add:

```rust
            let projected_jsons: Vec<String> =
                projected_paths.iter().map(|p| query_hops_json(p)).collect();
```

(d) Replace `let existing_pos = effect_map.iter().position(|(k, _)| k == &key);` with:

```rust
            let existing_pos = effect_index.get(&key).copied();
```

(e) Rewrite the merge arm to carry `(path, cond, json)` triples — replacing the existing
`merged: Vec<(Vec<QueryWitnessHop>, EffectConditionality)>` block, its sort, and its
dedupe (keep everything else in the arm — truncation, temp-state merge — verbatim):

```rust
                let mut merged: Vec<(
                    Vec<QueryWitnessHop>,
                    crate::engine::l5::conditionality::EffectConditionality,
                    String,
                )> = Vec::new();
                {
                    let existing = &effect_map[pos].1;
                    for (i, p) in existing.all_paths.iter().enumerate() {
                        let c = existing
                            .all_path_conds
                            .get(i)
                            .copied()
                            .unwrap_or(crate::engine::l5::conditionality::UNKNOWN);
                        let j = existing
                            .all_path_jsons
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| query_hops_json(p));
                        merged.push((p.clone(), c, j));
                    }
                }
                for ((p, c), j) in projected_paths
                    .iter()
                    .cloned()
                    .zip(path_conds.iter().copied())
                    .zip(projected_jsons.iter().cloned())
                {
                    merged.push((p, c, j));
                }
                merged.sort_by(|a, b| {
                    if a.0.len() != b.0.len() {
                        return a.0.len().cmp(&b.0.len());
                    }
                    a.2.cmp(&b.2)
                });
                let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                let mut unique_paths: Vec<Vec<QueryWitnessHop>> = Vec::new();
                let mut unique_conds: Vec<crate::engine::l5::conditionality::EffectConditionality> =
                    Vec::new();
                let mut unique_jsons: Vec<String> = Vec::new();
                for (p, c, j) in merged {
                    if seen.insert(j.clone()) {
                        unique_paths.push(p);
                        unique_conds.push(c);
                        unique_jsons.push(j);
                    }
                }
```

and at the end of the arm store `acc.all_path_jsons = unique_jsons;` alongside the
existing `acc.all_paths = unique_paths;` assignments.

INVARIANCE NOTE (keep as a code comment): the sort previously compared
`query_hops_json(&a.0)` vs `query_hops_json(&b.0)` computed on the fly; `a.2`/`b.2` are
the SAME strings computed once — ordering and dedupe sets are unchanged byte-for-byte.

(f) In the insert (else) arm, populate the new fields:

```rust
                effect_index.insert(key.clone(), effect_map.len());
                effect_map.push((
                    key,
                    AccumulatedEffect {
                        // ...existing fields verbatim...
                        all_path_jsons: projected_jsons,
                        // ...
                    },
                ));
```

(the `effect_index.insert` line goes immediately before the existing `effect_map.push`).

- [ ] **Step 3: Validate**

Run: `cargo test --lib digest && cargo test --lib ordering` then full `cargo test` and
`cargo clippy --all-targets --all-features`.
Expected: all green, zero golden diffs, clippy clean.

- [ ] **Step 4: CDO byte-compare + timing**

```powershell
cargo build --release --bin alsem
.\target\release\alsem.exe analyze 'u:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud' --preset transaction-integrity --deterministic --format json 1>$env:TEMP\witness-t1.json 2>$null
fc.exe /b $env:TEMP\witness-base.json $env:TEMP\witness-t1.json
```
Expected: `FC: no differences encountered`. Record the new elapsed (the O(F²) removal
targets the tail roots — expect a material drop). If outputs differ, STOP and fix root
cause; do not proceed.

- [ ] **Step 5: rustfmt, CHANGELOG, commit**

`rustfmt src/engine/l5/digest.rs`. CHANGELOG under `Changed`:
```markdown
- digest per-root merge loop de-quadratized: O(1) HashMap index over the insertion-ordered
  effect map (was an O(F) scan per fact → O(F²) on 1500-fact Page roots) + `query_hops_json`
  memoized per path instead of re-serialized on every merge sort/dedupe. Byte-identical
  (fc-verified on CDO). transaction-integrity preset: 88.4 s → <measured> s.
```
```powershell
git add src/engine/l5/digest.rs CHANGELOG.md
git commit -m "perf: O(1) effect-map index + memoized path JSON in digest_one_root

Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

---

### Task 2: Merge-identity effect-fact dedup (the 2.3× lever)

**Files:**
- Modify: `src/engine/l5/digest.rs` (`digest_one_root` loop head, right after the `detail` computation)
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: Task 1's file state (no structural dependency — only the loop head changes).
- Produces: no signature changes. New private `fn effect_fact_loop_identity(fact: &Fact, detail: &[(String, String)]) -> String` in digest.rs.

**Invariance argument (must survive review):** two facts with equal loop-identity keys
produce byte-identical loop contributions: `reconstruct_witness_paths` reads only
`(rid, provenance, witness_operation_id, witness_callsite_id, fact_equivalent-fields)`
— all in the key — so outcomes are identical; `project_path`/`compute_path_conditionality`
are pure over the outcome; `dedupe_key` reads `(effect_type, terminal, fact.op,
resource_kind, resource_id, detail)` — all in the key; the merge then dedupes the
duplicate's identical paths by hops-JSON and merges `temp_state`/`had_truncation`
idempotently for equal inputs, so the duplicate's entire merge is a NO-OP; the
first-insert-only fields (`evidence*`, `provenance`, `fact_subject`) are set by the
representative. Duplicate BFS outcomes' `diagnostics`/`incomplete` are NOT consumed in
`digest_one_root` (only `reconstruct_witness_paths_pub` forwards them — different caller),
so skipping emits nothing different. Measured dedup factor on CDO: ≤2.32× (13,025 distinct
identities vs 30,204 case-C calls).

- [ ] **Step 1: Add the identity key fn**

Near `dedupe_key` (~1808) add — and CHECK the real `Fact` struct field list first; the key
must include EVERY `Fact` field `digest_one_root` reads, and over-inclusion is always safe
(it only lowers the dedup factor), so when in doubt, include:

```rust
/// Identity of a fact's ENTIRE `digest_one_root` loop contribution. Two facts with
/// equal keys produce byte-identical witness outcomes, projections, dedupe keys, and
/// merge inputs — so every occurrence after the first is a provable no-op on
/// `effect_map` (the merge dedupes identical paths by hops-JSON and merges
/// temp_state/truncation idempotently). Skipping them changes nothing in the output
/// while removing the measured ~2.3× duplicate-fact cost (witness-investigation.md §3).
/// Fields: provenance + witness anchors (BFS inputs), op/resource_kind/resource_id +
/// the serialized `extra` (fact_equivalent's resource_arg_source / dispatch objectType,
/// plus temp_state), subject (fact_subject), and the computed effect detail.
fn effect_fact_loop_identity(fact: &Fact, detail: &[(String, String)]) -> String {
    format!(
        "{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}",
        fact.provenance,
        fact.op,
        fact.resource_kind,
        fact.resource_id.as_deref().unwrap_or(""),
        fact.witness_callsite_id.as_deref().unwrap_or(""),
        fact.witness_operation_id.as_deref().unwrap_or(""),
        fact.subject,
        serde_json::to_string(&fact.extra).unwrap_or_default(),
        detail_json(detail),
    )
}
```

If `fact.extra`'s type does not implement `Serialize`, derive it (it originates from the
serialized snapshot, so the derive should exist or be trivially addable) — do NOT
substitute a hand-rolled partial serialization that could miss a field. If `Fact` carries
additional fields consumed anywhere in `digest_one_root` (grep the function body for
`fact.`), add them to the key.

- [ ] **Step 2: Skip duplicates in the loop head**

In `digest_one_root`, after `let detail = effect_detail_of(...)` and BEFORE the witness
reconstruction, add:

```rust
            // Merge-identity dedup — see effect_fact_loop_identity's doc. Must run
            // BEFORE reconstruct_witness_paths (the expensive half) and keep FIRST
            // occurrence order so effect_map insertion order is unchanged.
            if !seen_identity.insert(effect_fact_loop_identity(fact, &detail)) {
                continue;
            }
```

with, before the loop:

```rust
        let mut seen_identity: std::collections::HashSet<String> = std::collections::HashSet::new();
```

- [ ] **Step 3: Validate**

Run: full `cargo test` and `cargo clippy --all-targets --all-features`.
Expected: all green, ZERO golden diffs. A golden diff here means the identity key is
missing a consumed field — fix the key, never the golden.

- [ ] **Step 4: CDO byte-compare + timing**

Same commands as Task 1 Step 4 (compare against `$env:TEMP\witness-base.json`).
Expected: `FC: no differences encountered`; elapsed drops further (dedup removes ~2.3×
of BOTH the witness and merge halves). Record elapsed. STOP on any byte difference.

- [ ] **Step 5: rustfmt, CHANGELOG, commit**

`rustfmt src/engine/l5/digest.rs`. CHANGELOG under `Changed`:
```markdown
- digest per-root loop now skips duplicate effect facts (merge-identity dedup, measured
  ~2.3× duplication on CDO): a fact identical in every consumed field to an earlier one
  contributes a provable no-op to the effect map, so its witness BFS + projection +
  merge are skipped entirely. Byte-identical (fc-verified). transaction-integrity
  preset: <t1> s → <measured> s.
```
```powershell
git add src/engine/l5/digest.rs CHANGELOG.md
git commit -m "perf: merge-identity effect-fact dedup in digest_one_root (~2.3x on CDO)

Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

---

### Task 3: `compose_snapshot` — cached-key sort + parallel fact emission

**Files:**
- Modify: `src/engine/l5/snapshot.rs` (`derive_capability_facts` ~1121-1143)
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: nothing from Tasks 1-2 (independent file).
- Produces: no signature changes.

- [ ] **Step 1: Implement**

Replace `derive_capability_facts` body:

```rust
fn derive_capability_facts(base: &R3a3SourceBase) -> Vec<SnapshotCapabilityFact> {
    use rayon::prelude::*;
    // Per-routine emission is independent; indexed par_iter + sequential flatten
    // preserves the exact sequential emission order (ties in the sort key below are
    // resolved by the STABLE sort from this input order — do not reorder).
    let per_routine: Vec<Vec<SnapshotCapabilityFact>> = base
        .ws_routines
        .par_iter()
        .map(|r| {
            // Only routines with a cone entry contribute (mirrors composeSnapshot:
            // facts come from `r.summary`, present iff the cone ran for the routine).
            if !base.cones.contains_key(&r.id) {
                return Vec::new();
            }
            let subject = stable_routine_id(&r.id, &base.routine_to_stable);
            let mut v: Vec<SnapshotCapabilityFact> = Vec::new();
            if let Some(direct) = base.direct_full.get(&r.id) {
                for f in direct {
                    v.push(snapshot_fact(f, &subject));
                }
            }
            if let Some(cone) = base.cones.get(&r.id) {
                for f in &cone.inherited {
                    v.push(snapshot_fact(f, &subject));
                }
            }
            v
        })
        .collect();
    let mut all: Vec<SnapshotCapabilityFact> = per_routine.into_iter().flatten().collect();
    // sort_by_cached_key: identical order to sort_by_key (same key, both stable),
    // but the 8-field join("|") String is built ONCE per fact instead of per
    // comparison (~128k facts on CDO).
    all.sort_by_cached_key(capability_fact_sort_key);
    all
}
```

Do NOT replace the joined-String key with a field-tuple comparator — joined-with-separator
ordering differs from field-wise tuple ordering when one field is a byte-prefix of another
(byte `|` = 0x7C compares against the next field's first byte). The cached key keeps the
exact existing order.

- [ ] **Step 2: Validate**

Run: full `cargo test` (snapshot/digest goldens are the oracle) and
`cargo clippy --all-targets --all-features`.
Expected: all green, zero golden diffs.

- [ ] **Step 3: CDO byte-compare + timing**

Same byte-compare as Task 1 Step 4 (against `$env:TEMP\witness-base.json`). Record elapsed.

- [ ] **Step 4: rustfmt, CHANGELOG, commit**

`rustfmt src/engine/l5/snapshot.rs`. CHANGELOG under `Changed`:
```markdown
- `compose_snapshot`'s capability-fact materialization parallelized per routine and its
  canonical sort switched to `sort_by_cached_key` (key built once per fact instead of per
  comparison; ~128k facts on CDO). Byte-identical. compose_snapshot: 3.1 s → <measured>.
```
```powershell
git add src/engine/l5/snapshot.rs CHANGELOG.md
git commit -m "perf: parallel fact emission + cached-key sort in compose_snapshot

Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

---

### Task 4: Measurement close-out + inner-root-parallelism decision gate

**Files:**
- Modify: `docs/perf-regression-t3-vs-0.9.3.md` (append `## 12. witness/digest optimization`)
- Create: `.superpowers/sdd/alsem-parallel/witness-close-out.md` (raw runs; gitignored)

**Interfaces:**
- Consumes: per-task timings recorded in Tasks 1-3.
- Produces: docs + a go/no-go recommendation on Candidate F.

- [ ] **Step 1: Final medians**

3× runs each at branch tip: default analyze + transaction-integrity preset (both
WITHOUT `--deterministic`, matching §11's methodology). Raw runs to the close-out file.

- [ ] **Step 2: Append §12**

```markdown
## 12. alsem witness/digest optimization (2026-07-14)

Investigation: `.superpowers/sdd/alsem-parallel/witness-investigation.md`. The 88 s
transaction-integrity run was ~76 s digest par-loop, split ~50% witness BFS / ~48%
O(F²) merge+projection, both amplified 2.3× by duplicate effect facts.

| Command (CDO) | §11 baseline | After |
|---|---:|---:|
| `analyze` (default set) | 7.31 s | <measured> |
| `--preset transaction-integrity` | 88.4 s | <measured> |

Landed: O(1) effect-map index + memoized path JSON; merge-identity effect-fact dedup
(~2.3×); parallel + cached-key compose_snapshot. All byte-identical (fc-verified vs
pre-branch output with --deterministic).

Measured and REJECTED (do not re-chase): valid_nodes memo (260× dedup but 0.4% of
cost); cross-root cone/suffix sharing (first hop carries the root id — not portable —
and shared BFS budgets break byte-identity).

Remaining lever if <30 s is still wanted: Candidate F — inner-root parallelism over the
distinct-fact loop of the ~9 giant Page roots (deterministic re-assembly required);
see witness-investigation.md §5.
```
(fill measured numbers; state whether the <30 s target was met and the go/no-go on F).

- [ ] **Step 3: Commit**

```powershell
git add docs/perf-regression-t3-vs-0.9.3.md
git commit -m "docs: witness/digest optimization close-out (section 12)

Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```
