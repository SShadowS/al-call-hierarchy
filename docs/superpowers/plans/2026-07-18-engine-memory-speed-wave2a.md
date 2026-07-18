# Engine Memory/Speed Wave 2a (Targeted Detector-Loop Fixes) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collapse the measured L5 detector-loop wall (d19 988s / d1 448s→77+min / d12 425s; loop = 94% of full-default wall at 5400) via two byte-stable mechanical fixes licensed by the Wave-2 measurements, then re-measure; plus the trigger-edge SCC-fusion verification study.

**Architecture:** Two hot-path fixes (the shared `fingerprint_of` per-finding quadratic; d1's per-edge cone re-allocation), a re-measure capstone, and one report-only investigation. B1/B2 (interning, shared cones) are EXPLICITLY deferred to a Wave-2b plan written AFTER the re-measure — their sizing depends on numbers these fixes will change.

**Tech Stack:** Rust, existing crate `al-call-hierarchy` (HYPHEN in `-p`). No new dependencies.

## Global Constraints

- Byte-stable output everywhere: `bash scripts/check-goldens` clean after every task; full `cargo test` green; `cargo clippy --all-targets --all-features` clean; DO workspace default-run JSON byte-identical pre/post (procedure in T1).
- Evidence base: `docs/superpowers/specs/2026-07-18-wave2-measurements.md` (§4a verified root cause) + `.superpowers/sdd/w2-detector-anatomy.md` (file:line complexity analysis). Read both before implementing.
- `rustfmt <file>` per touched file — NEVER `cargo fmt`. Stage explicitly — NEVER `git add -A`. No `git stash`. No `| tail` on long runs (redirect to file + grep).
- Branch: `worktree-wave2-measure-design` (worktree `U:\Git\al-call-hierarchy\.claude\worktrees\wave2-measure-design`). It carries a re-armed WIP probes commit (`2b0d1ce`) — env-gated `ALSEM_STAGE_TIMING` instrumentation used by T3's re-measure. Leave probe lines alone; the probe sweep happens at merge time (same procedure as Wave 1: coordinated removal commit, then gate re-run).
- DO workspace: `U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud`. Measurement builds: `cargo build --profile release-fast --bin alsem`; kill stale `alsem.exe` on link error os 5.
- Corpus rebuild script for T3: see the Wave-1 plan's Global Constraints (`docs/superpowers/plans/2026-07-18-engine-memory-speed-wave1.md`) — same Base App extraction + slice-5400.
- Suggested dispatch models: T1, T2 → Opus implementer + Opus reviewer (hot-path semantics); T3 → Sonnet (measurement runbook); T4 → Opus (source-verification judgment); T5 → orchestrator.

---

### Task 1: Structural fingerprint substitution (kills the d19/d12 quadratic)

**Files:**
- Modify: `src/engine/l5/fingerprint.rs` (`FingerprintIndex::build` :64-85, `fingerprint_of` :91-165)
- Test: same file's `#[cfg(test)]` module

**Interfaces:** `FingerprintIndex` gains a private `model_instance_prefixes: Vec<String>` (the distinct `"{modelInstanceId}/"` prefixes present in `stable_by_id` keys); `fingerprint_of`'s signature and output are UNCHANGED.

Measured cost being removed: per `fingerprint_of` CALL — one collect+sort of all ~100k `stable_by_id` entries PLUS an up-to-100k-entry `starts_with` scan per BYTE of `root_cause_key` (`fingerprint.rs:123-152`). O(F·(R log R + L·R)). New cost: O(L) per call + one HashMap probe per candidate id occurrence.

Semantic ground rules (verified in source):
- Internal RoutineIds have FIXED structure `"{modelInstanceId}/{64 lowercase hex}"` (`engine/ids.rs:192-194`). All ids in one run share the same modelInstanceId, so all keys in `stable_by_id` have identical length; at any byte position at most ONE id can match. The current longest-first sort is therefore inert among routine ids — replacement output is a pure function of "is there an id-shaped substring here that is a key of `stable_by_id`".
- Routines with empty `normalized_signature_hash` are NOT in `stable_by_id` (`fingerprint.rs:74-78`) — an id-shaped substring that misses the map must be copied VERBATIM (today's behavior: no entry matches, scan advances byte-by-byte through it).

- [ ] **Step 1: Write the equivalence test FIRST (against the current implementation)**

Add to the tests module a copy of the CURRENT substitution loop as a test-local oracle `fn oracle_substitute(key: &str, stable_by_id: &HashMap<String,String>) -> String` (paste the body of :115-153 verbatim, minus the surrounding struct), then:

```rust
    #[test]
    fn structural_substitution_matches_scan_oracle() {
        // Build a stable_by_id with ids of the REAL shape: "{mid}/{64-hex}".
        let mid = "r0"; // also test a 16-char mid in a second map
        let mk = |h: char| format!("{mid}/{}", std::iter::repeat(h).take(64).collect::<String>());
        let id_a = mk('a');
        let id_b = mk('b');
        let id_absent = mk('c'); // id-shaped but NOT in the map (empty-hash routine)
        let mut map: HashMap<String, String> = HashMap::new();
        map.insert(id_a.clone(), "STABLE_A".to_string());
        map.insert(id_b.clone(), "STABLE_B".to_string());

        let keys = [
            format!("op/{id_a}/cs1"),                    // embedded with suffix
            format!("{id_a}|{id_b}"),                    // two ids
            format!("{id_absent}/op3"),                  // id-shaped, absent -> verbatim
            format!("prefix {id_a}{id_b} adjacent"),     // adjacent ids
            "no ids at all".to_string(),
            format!("{mid}/short-not-hex"),              // prefix but not 64-hex
            format!("{id_a}"),                           // exact
        ];
        for k in &keys {
            assert_eq!(
                substitute_stable_ids(k, &map, &["r0/".to_string()]),
                oracle_substitute(k, &map),
                "divergence on key: {k}"
            );
        }
    }
```

Run: `cargo test -p al-call-hierarchy --lib structural_substitution -- --nocapture` — FAILS (function not defined). That's the RED step.

- [ ] **Step 2: Implement `substitute_stable_ids`**

```rust
/// Replace every stable_by_id KEY occurring in `key` with its value, scanning
/// structurally instead of trying every id at every byte. Ids have the fixed
/// shape "{modelInstanceId}/{64 lowercase hex}" (engine/ids.rs:192), so we find
/// candidate occurrences by scanning for each known "{mid}/" prefix and
/// checking the following 64 bytes for lowercase-hex; the candidate substring
/// is then a single HashMap probe. A candidate that misses the map is copied
/// verbatim (identical to the old scan's behavior for empty-hash routines).
fn substitute_stable_ids(
    key: &str,
    stable_by_id: &HashMap<String, String>,
    prefixes: &[String],
) -> String {
    let bytes = key.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len);
    let mut pos = 0usize;
    'outer: while pos < len {
        for p in prefixes {
            let plen = p.len();
            if key[pos..].starts_with(p.as_str()) && pos + plen + 64 <= len {
                let candidate = &key[pos..pos + plen + 64];
                let hex = &bytes[pos + plen..pos + plen + 64];
                if hex
                    .iter()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
                {
                    if let Some(stable) = stable_by_id.get(candidate) {
                        out.push_str(stable);
                        pos += plen + 64;
                        continue 'outer;
                    }
                }
            }
        }
        let ch = key[pos..].chars().next().expect("valid UTF-8");
        out.push(ch);
        pos += ch.len_utf8();
    }
    out
}
```

In `FingerprintIndex::build`, derive the prefix set once:

```rust
        let mut model_instance_prefixes: Vec<String> = stable_by_id
            .keys()
            .filter_map(|k| k.find('/').map(|i| k[..=i].to_string()))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        model_instance_prefixes.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
```

(add the field; longest-prefix-first preserves the old longest-first shadowing for the pathological case of one mid being a prefix of another). In `fingerprint_of`, replace the whole `else` branch at :117-153 with `substitute_stable_ids(finding.root_cause_key.as_str(), &self.stable_by_id, &self.model_instance_prefixes)`.

SEMANTIC CAUTION the implementer must check, not assume: grep the tests + goldens for rootCauseKeys containing id-shaped substrings with UPPERCASE hex or non-standard mid — if any golden depends on the old scan matching something the structural scan wouldn't (or vice versa), STOP and report. The equivalence oracle test plus full goldens are the net.

- [ ] **Step 3: GREEN + gates**

```bash
cargo test -p al-call-hierarchy --lib structural_substitution   # PASS
cargo test > /tmp/w2a-t1-test.log 2>&1; echo exit=$?             # 0
bash scripts/check-goldens > /tmp/w2a-t1-goldens.log 2>&1; echo exit=$?  # 0
cargo clippy --all-targets --all-features 2>&1 | grep -c '^error'        # 0
```

DO byte-compare: build release-fast; run `alsem analyze <DO> --format json` before (baseline captured pre-task) and after; `diff` must be empty.

- [ ] **Step 4: rustfmt + commit**

```bash
rustfmt src/engine/l5/fingerprint.rs
git add src/engine/l5/fingerprint.rs
git commit -m "perf(l5): structural stable-id substitution in fingerprint_of

Replaces the per-finding sort of all ~100k stable ids + per-byte all-ids
starts_with scan (O(F*(R log R + L*R))) with an O(L) structural scan +
HashMap probe. Ids have fixed shape {mid}/{64-hex}; equivalence pinned by
a scan-oracle unit test. Kills the d19/d12 detector-loop quadratic and
d1's fingerprint tail."
```

---

### Task 2: Non-allocating reachable + memoized touches_db (d1's cone explosion)

**Files:**
- Modify: `src/engine/l5/full_summary.rs` (add `reachable_iter`)
- Modify: `src/engine/l5/capability_query.rs` (`touches_db_of` :133-144 and the sibling helpers that walk `reachable()` — `writes_db_of`/`writes_tables_of`/`publishes_events_of` etc.: switch scans to the iterator; keep `reachable()` itself for callers needing a Vec)
- Modify: `src/engine/l5/detectors/d1.rs` (:614-632 region — the `expand`/`touches_db_of` per-edge probe: memoize per routine)
- Test: d1's existing unit tests + a memo-consistency unit test

**Interfaces:**
- `FullRoutineSummary::reachable_iter(&self) -> impl Iterator<Item = &CapabilityFact>` (direct chained with inherited — same order as `reachable()`).
- d1 gains a local `touches_db_memo: HashMap<&str, EffectPresence>` (routine id → answer), built lazily inside its walk (entry API), keyed by the summary's routine id.

Measured cost being removed: per EDGE examined in d1's 500-node `walk_evidence` DFS, `touches_db_of` → `reachable()` allocates a fresh Vec of direct∪inherited refs (`full_summary.rs:46-56`) — cone sizes reach thousands at 8020 density; d1 alone ran 77+ min. New cost: first probe per routine walks the chain iterator (early-exits on the first `"table"` fact); repeat probes are O(1) memo hits.

- [ ] **Step 1: Add `reachable_iter` + switch `touches_db_of`**

```rust
    /// Iterator form of [`reachable`] — same order (direct first, then
    /// inherited), zero allocation. Prefer this in hot paths that early-exit.
    pub fn reachable_iter(&self) -> impl Iterator<Item = &CapabilityFact> {
        self.capability_facts_direct
            .iter()
            .chain(self.capability_facts_inherited.iter())
    }
```

```rust
pub fn touches_db_of(s: &FullRoutineSummary) -> EffectPresence {
    for f in s.reachable_iter() {
        if f.resource_kind == "table" {
            return EffectPresence::Yes;
        }
    }
    if s.inherited_status() == "complete" {
        EffectPresence::No
    } else {
        EffectPresence::Unknown
    }
}
```

Convert the OTHER `capability_query.rs` helpers that do `for f in s.reachable()` scans the same way (grep `s.reachable()` / `.reachable()` in that file; each is a mechanical swap — the iteration order is identical so results are byte-identical). Do NOT touch callers outside capability_query that use `reachable()`'s Vec (e.g. if the path walker indexes into it) — grep first, list them in the report.

- [ ] **Step 2: Memoize d1's per-edge probe**

At the walk site (`d1.rs:614-632` region — anchor on the `touches_db_of` call the anatomy doc cites at :625), thread a memo through the DFS: build `let mut touches_db_memo: std::collections::HashMap<&str, EffectPresence> = HashMap::new();` at the per-candidate walk's entry point (or once per detector run if the walk helper's signature allows — prefer once per run), and replace the direct call with:

```rust
    let touches = *touches_db_memo
        .entry(summary.routine_id.as_str())
        .or_insert_with(|| touches_db_of(summary));
```

(`EffectPresence` is `Copy` — verify; if not, derive or clone.) Lifetime note: the memo borrows `&str` from summaries owned by `ctx` — outlives the walk; if the borrow checker objects at the chosen scope, key by `String` (clone) — correctness first, the id clone is nothing next to the removed Vec.

- [ ] **Step 3: Memo-consistency test + gates**

Unit test: for a small fixture ctx (reuse d1's existing test fixtures), assert `touches_db_of(s)` equals the memoized path's answer for every routine, and that `reachable_iter` yields exactly `reachable()`'s sequence (zip-compare on a summary with both direct + inherited facts).

```bash
cargo test -p al-call-hierarchy --lib d1 > /tmp/w2a-t2-d1.log 2>&1; echo exit=$?
cargo test > /tmp/w2a-t2-test.log 2>&1; echo exit=$?
bash scripts/check-goldens > /tmp/w2a-t2-goldens.log 2>&1; echo exit=$?
cargo clippy --all-targets --all-features 2>&1 | grep -c '^error'
```

DO byte-compare as in T1.

- [ ] **Step 4: rustfmt + commit**

```bash
rustfmt src/engine/l5/full_summary.rs src/engine/l5/capability_query.rs src/engine/l5/detectors/d1.rs
git add src/engine/l5/full_summary.rs src/engine/l5/capability_query.rs src/engine/l5/detectors/d1.rs
git commit -m "perf(l5): zero-alloc reachable iteration + memoized touches_db in d1

touches_db_of and siblings early-exit over a chained iterator instead of
allocating the direct+inherited Vec per probe; d1 memoizes the per-routine
answer across its walk_evidence DFS (was: one cone-sized allocation per
edge examined, measured 77+ min alone at 8020)."
```

---

### Task 3: Re-measure (the license for Wave-2b)

**Files:** none (measurement runbook; results go to T5's docs).

- [ ] **Step 1:** Rebuild corpora if scratchpad is gone (Wave-1 plan's script); `cargo build --profile release-fast --bin alsem`.
- [ ] **Step 2:** Runs, each via `python scripts/peak_rss.py` (sequential, NO concurrent alsem processes — the Wave-2 measurement runs were contention-polluted; do not repeat that):
  1. slice-5400 full-default (`--format json`, no detector flag) — pre-fix contended baseline was 2608s/93.9% loop; expect the loop share to collapse.
  2. baseapp-ws 8020 full-default — pre-fix run never finished d1 (77+ min in d1, killed at 2.5h cap). Cap this run at 2h via the detached monitor-script pattern if >10 min foreground (Wave-1 plan describes the detached monitor).
  3. baseapp-ws 8020 3-detector triple (regression guard: was 90.3s/6.1GB after Wave 1 — must not regress).
  4. DO default (was 10.7s — must not regress, output byte-identical).
- [ ] **Step 3:** With `ALSEM_STAGE_TIMING=1` on the 5400 run, extract the new per-detector table (same extraction as `.superpowers/sdd/w2-jacobi-5400-analysis.md`); confirm d19/d12/d1 fell into the pack, and record the NEW top-5 (the residual list decides Wave-2b's detector content, if any).
- [ ] **Step 4:** Hand the numbers to T5.

---

### Task 4: Trigger-edge SCC-fusion verification (report-only)

**Files:** none modified — investigation report `.superpowers/sdd/w2-trigger-edge-verdict.md`.

Question (from `2026-07-18-wave2-measurements.md` §2): the 846-member SCC's intra-edges are 41% `implicit-trigger` (1,077/2,610), and that share explodes with corpus density (18% at 5400). Are those edges PRECISE (real BC control flow: the write really can fire that trigger) or OVER-APPROXIMATED (e.g. any-write-to-table → every trigger of that table, regardless of field/op applicability)?

- [ ] **Step 1:** Read the implicit-trigger edge construction (grep `implicit-trigger` in `src/engine/l4/` — the combined-graph fold) and document the exact preconditions an edge requires (op kind, table match, trigger kind).
- [ ] **Step 2:** Extract 15-20 intra-SCC implicit-trigger edges from the 846-SCC (the SCCANATOMY probe prints members; add a TEMPORARY dump of `(from, to)` pairs for `kind == "implicit-trigger"` inside the anatomy block if needed — throwaway, on the probes commit, never merged). For each sampled edge: open both routines in the extracted Base App source (`<scratchpad>/baseapp-ws/src/`), judge whether the write-site can actually reach that trigger (correct table? op that fires it? Rec.Insert(false) suppression?).
- [ ] **Step 3:** Verdict per edge + aggregate: precise / over-approximated-with-cause. If over-approximation is found, name the missing precondition and estimate the SCC effect (how many of the 1,077 edges the fix would drop) — the FIX itself is NOT in this plan (it touches advisory-graph semantics; goldens + a d43-d45 event-detector FP review gate it; it becomes a Wave-2b item with its own licensing).
- [ ] **Step 4:** Report file + one-paragraph summary for T5.

---

### Task 5: Capstone — docs + OUTSTANDING + CHANGELOG

- [ ] **Step 1:** Fold T3 numbers + T4 verdict into `docs/superpowers/specs/2026-07-18-wave2-measurements.md` (fill every PENDING slot; add a "Wave-2a outcome" section mirroring the findings doc's §7b style).
- [ ] **Step 2:** CHANGELOG `[Unreleased]` → `Changed`: the two fixes with before/after numbers. OUTSTANDING: tick Wave-2a; write the Wave-2b item (B1/B2 + any residual detector + the trigger-edge fix if licensed) pointing at the updated measurements doc.
- [ ] **Step 3:** Commit docs. Merge decision + probe sweep happen with the user (same procedure as Wave 1).

---

## Self-review notes

- Spec coverage: measured hot spots → T1 (fingerprint), T2 (d1 cone); doctrine gates → T3 (re-measure before Wave-2b), T4 (verify before building the trigger fix). B1/B2 intentionally absent — licensed by T3's numbers, planned in Wave-2b.
- No placeholders: every code step carries real code; T3/T4 are runbooks with exact commands/paths by reference to the Wave-1 plan (same session artifacts).
- Type consistency: `substitute_stable_ids` free fn + `model_instance_prefixes: Vec<String>` field used consistently in T1; `reachable_iter` name used consistently in T2.
- Risk ranking: T1 is the highest-leverage and lowest-risk (oracle-pinned); T2's only subtlety is memo scope/borrow — clone-key fallback sanctioned; T4 is read-only.
