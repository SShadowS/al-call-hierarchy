# Uncertainty Identity Substrate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish uncertainty identity ONCE, where it is discovered — in the
context-build hash-cons pool — and have every consumer read ids instead of
re-deriving identity from `Arc<[Uncertainty]>` values.

**Architecture:** `UncertaintySetPool` (`src/engine/l5/detector_context.rs`) already
computes exactly the identity this engine keeps re-deriving: it keys per-node sets by
CONTENT, collapsing 27,037 nodes onto 10,112 distinct allocations (8020), over a
19,311-value vocabulary. It then throws that identity away, returning only
`Arc<[Uncertainty]>`. Consequences, all live today: d1 re-interns 2,229,391 elements per
run into a d1-local `UncertaintyTable`; d1 memoizes per-set work behind a RAW POINTER key
whose soundness needs a `keep_alive` `Arc` clone to argue; `ctx.uncertainties_by_node`
retains 102 MiB of records that are 192× duplicated by value; and `d1_reach`'s
`unc` flags do a `HashMap<String, _>` lookup to answer "is this node's set empty".
This plan promotes the pool to a real interner — `UncertaintyId` per distinct VALUE,
`UncertaintySetId` per distinct SET, elements stored once in a flat pool — and migrates
consumers to it.

**Tech Stack:** Rust, no new dependencies. Touches `src/engine/l5/detector_context.rs`,
`d1_cohort.rs`, `d1_dataflow.rs`, `d1_reach.rs`, `detectors/d1.rs`, and the r4 golden
family.

## What this is and is not

**IS:** an identity/ownership correction that (a) deletes a pointer-keyed cache and its
soundness argument, (b) delivers `docs/OUTSTANDING.md`'s parked
*"`ctx.uncertainties_by_node` — Step 2: intern the ELEMENTS to ids (~−88 MiB)"* as a
consequence, and (c) makes *"FindingConfidence carries ids, not records (−78 MiB)"*
natural rather than awkward, because ids become run-global instead of d1-local.

**IS NOT:** a speed win of any size worth quoting. After `12f7e95` and `d92df42`,
`detector.d1` is 4.32 s of a 70.89 s run and the whole remaining uncertainty-union cost is
~215 ms. **Do not justify this arc with wall-clock; justify it with the memory figures and
the deleted re-derivation.** A capstone that leads with a speed number is wrong.

## Global Constraints

- **Byte-identical output at every task boundary.** Gate: `scripts/check-goldens` (zero
  files under `tests/` moved) PLUS both corpora `--deterministic`:
  DO `f022f677d2650b2399fc3aa5a7625bc6c078d90dd51cdb80e1e3705808fee3ea`,
  8020 `36151bf67e17620724abb6b2cdbad55bcf8f97ffe3c3237782a0cf4c25ecc5fb`.
- **Task 0 is a HARD precondition, not a warm-up.** See below.
- Build with `export TREE_SITTER_AL_PATH=U:/Git/al-call-hierarchy/tree-sitter-al` — a
  worktree has no submodule checkout, and the pre-commit hook's own golden build needs it
  exported too or it fails on `al-syntax`'s build script rather than on any golden.
- `rustfmt <file>` per file, never `cargo fmt`. `cargo clippy --all-targets
  --all-features` clean is the standing bar — fix warnings, do not `allow` them.
- Every test states its precondition by hand AND carries a discrimination proof (break it,
  observe the failure, revert, observe the pass, record both). **Assert that a scripted
  break actually applied** (`assert s.count(old) == 1`) — an unasserted scripted break
  produced a false PASS earlier in this arc when `rustfmt` had reflowed the target text.

## Environment

```bash
export WT=U:/Git/al-call-hierarchy/.claude/worktrees/perf-d1-followups
export TREE_SITTER_AL_PATH=U:/Git/al-call-hierarchy/tree-sitter-al
export CORPUS_8020='C:/Users/SShadowS/AppData/Local/Temp/claude/U--Git-al-call-hierarchy/66efc3ec-d07b-48e2-8181-95ce2f62dd04/scratchpad/corpus-8020'
export DO_WS='U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud'
cd "$WT"
```

---

### Task 0: Close the golden hole this arc would otherwise walk through

**This is the precondition `docs/OUTSTANDING.md` records for the parked Step-2 item, and
it is still open — verified this session.** Exactly ONE golden in the repository carries a
non-empty `confidence.evidence`: `tests/r4-goldens/ws-d1-uncertain-path.r4.golden.json`, a
**d1** finding. Every other uncertainty-bearing confidence output — d2's, d46's, d48's —
has **zero** golden coverage of any kind. This arc rewrites the code that produces all of
them. Without this task, a regression in d2/d46/d48's uncertainty path turns no golden
red, and the arc's "byte-identical" claim would be measuring a corpus that never exercises
three of its four consumers.

**Files:**
- Create: `tests/r0-corpus/ws-d2-uncertain/` (fixture) — or extend `tests/r0-corpus/ws-d2`
  if it can be made uncertainty-bearing without disturbing its existing golden
- Modify: the r4 golden manifest that declares corpus directories
- Regenerate: `tests/r4-goldens/`

- [ ] **Step 1: Establish that the hole is real, not assumed**

```bash
cd "$WT"
# -U/--multiline is REQUIRED: the pattern spans a newline, and `grep -P` without
# it silently returns NOTHING — a false negative that reads exactly like "no
# coverage gap here". This bit once already in this arc.
rg -U --files-with-matches '"evidence":\s*\[\s*\{' tests/ | sort
```

**VERIFIED 2026-07-31 with the multiline search**: exactly one r4 golden matches —
`tests/r4-goldens/ws-d1-uncertain-path.r4.golden.json`, a d1 finding — plus seven
`tests/cli-b-goldens/prove/*` files, which are the `prove` command's surface, not the r4
finding-confidence family. The hole is real and this task stands as written.

- [ ] **Step 2: Find what makes d2 emit uncertainty-bearing confidence**

`d2`'s confidence comes from `sub_summary_uncertainties` (see
`src/engine/l5/detectors/d2.rs:547`'s doc). Read that function and its caller to learn
which fixture shape produces a NON-EMPTY set — an unresolved-callee sub-routine reachable
from an event-fanout loop. Write the fixture from that reading, not by guessing.

- [ ] **Step 3: Add the fixture and regenerate**

```bash
REGEN_TEMP_GOLDENS=1 cargo test --test r4 > logs/t0-regen.log 2>&1; echo "EXIT=$?"
git status --short tests/ | head
```

Expected: a NEW golden appears carrying a non-empty `confidence.evidence`. Inspect the
diff — a regen is a measurement, never an auto-bless.

- [ ] **Step 4: Prove the new golden discriminates**

Break the union deliberately — in `d2.rs`, drop one element from the uncertainty list it
feeds to `to_confidence` — and confirm the NEW golden goes red while the pre-existing ones
stay green. Revert; confirm green. **Record both outcomes.** A golden that cannot fail is
not coverage, and this task exists solely to create coverage that can.

- [ ] **Step 5: Commit**

```bash
rustfmt src/engine/l5/detectors/d2.rs 2>/dev/null || true
git add tests/ && git commit -m "test(d2): cover uncertainty-bearing confidence with a golden that can fail"
```

---

### Task 1: Promote the pool to an interner (`UncertaintyIndex`)

**Files:**
- Modify: `src/engine/l5/detector_context.rs`

**Interfaces produced:**

```rust
/// A distinct uncertainty VALUE, interned run-globally at context build.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct UncertaintyId(pub u32);

/// A distinct uncertainty SET (the hash-consed per-node set), interned likewise.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct UncertaintySetId(pub u32);

pub struct UncertaintyIndex {
    values: Vec<Uncertainty>,          // id -> value
    keys: Vec<Box<str>>,               // id -> uncertainty_key(value), precomputed once
    lites: Vec<UncertaintyLite>,       // id -> the confidence mapper's view, precomputed once
    by_value: HashMap<Uncertainty, UncertaintyId>,
    set_elems: Vec<UncertaintyId>,     // flat pool
    set_span: Vec<Range<u32>>,         // set id -> window into set_elems
    by_set: HashMap<Box<[UncertaintyId]>, UncertaintySetId>,
}

impl UncertaintyIndex {
    pub fn intern_set(&mut self, set: Vec<Uncertainty>) -> UncertaintySetId;
    pub fn elements(&self, s: UncertaintySetId) -> &[UncertaintyId];
    pub fn is_empty_set(&self, s: UncertaintySetId) -> bool;
    pub fn value(&self, id: UncertaintyId) -> &Uncertainty;
    pub fn key(&self, id: UncertaintyId) -> &str;
    pub fn lite(&self, id: UncertaintyId) -> &UncertaintyLite;
    pub fn dedupe(&self, ids: &[UncertaintyId]) -> Vec<UncertaintyId>;  // last-write-wins by key, key-sorted
}
```

`keys`/`lites` move here from `d1_cohort::UncertaintyTable` UNCHANGED — the d1-memory arc
established that precomputing them once per distinct value (rather than per record) is the
point; this task widens "once per d1 run" to "once per analyze run", serving every
consumer instead of one.

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn interning_equal_sets_yields_one_set_id_and_one_element_window() {
        let mut ix = UncertaintyIndex::default();
        let a = ix.intern_set(vec![u_iow("IAlpha"), u_iow("IBeta")]);
        let b = ix.intern_set(vec![u_iow("IAlpha"), u_iow("IBeta")]);
        assert_eq!(a, b, "equal sets are ONE set id");
        assert_eq!(ix.elements(a), ix.elements(b));
        assert_eq!(ix.value_count(), 2, "two distinct values, interned once each");
    }

    /// Set identity is by CONTENT and ORDER — the pool's own contract. A
    /// reordered set is a DIFFERENT set, because the union that consumes it is
    /// last-write-wins by key and therefore order-sensitive.
    #[test]
    fn a_reordered_set_is_a_distinct_set_id() {
        let mut ix = UncertaintyIndex::default();
        let a = ix.intern_set(vec![u_iow("IAlpha"), u_iow("IBeta")]);
        let b = ix.intern_set(vec![u_iow("IBeta"), u_iow("IAlpha")]);
        assert_ne!(a, b);
    }

    /// `dedupe` reproduces `UncertaintyTable::dedupe` EXACTLY: last-write-wins by
    /// key, output key-sorted. Hand-stated on the one shape where it is
    /// observable — two `interface-open-world` values sharing a key.
    #[test]
    fn dedupe_is_last_write_wins_by_key_and_key_sorted() {
        let mut ix = UncertaintyIndex::default();
        let alpha = ix.intern_value(&u_iow("IAlpha"));
        let beta = ix.intern_value(&u_iow("IBeta"));
        assert_eq!(ix.key(alpha), ix.key(beta), "precondition: one key, two values");
        assert_eq!(ix.dedupe(&[alpha, beta]), vec![beta], "later wins");
        assert_eq!(ix.dedupe(&[beta, alpha]), vec![alpha], "…and order decides it");
    }
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p al-call-hierarchy --lib detector_context`.
  Expected: `cannot find type UncertaintyIndex`.

- [ ] **Step 3: Implement `UncertaintyIndex`**, replacing `UncertaintySetPool`'s body.
  Keep `share()` as a thin wrapper that interns and then materializes the
  `Arc<[Uncertainty]>` the current `uncertainties_by_node` still hands out, so this task
  changes NOTHING downstream yet. `intern_set` must dedupe the VALUE list into ids first,
  then key `by_set` on the id slice — id-keying is cheaper than value-keying and, because
  `by_value` is injective, identifies exactly the same sets.

- [ ] **Step 4: Prove discrimination.** (1) make `by_set` key on a SORTED id slice →
  `a_reordered_set_is_a_distinct_set_id` FAILS. (2) flip `dedupe` to first-write-wins →
  `dedupe_is_last_write_wins_by_key_and_key_sorted` FAILS. Record both.

- [ ] **Step 5: Gate + commit.** Full byte-identity gate (both corpora + check-goldens);
  nothing downstream changed, so any movement here is a bug in the wrapper.

---

### Task 2: `uncertainties_by_node` carries `UncertaintySetId`

**Files:** `detector_context.rs`, `d1_reach.rs`, `d1_dataflow.rs`, `detectors/d1.rs`

Change the field to `pub uncertainties_by_node: HashMap<String, UncertaintySetId>` and add
`pub uncertainties: UncertaintyIndex`. Migrate the three read sites:

- **`d2.rs`'s `sub_summary_uncertainties`** (`src/engine/l5/detectors/d2.rs:565`) reads
  `ctx.uncertainties_by_node` DIRECTLY and returns a `&[Uncertainty]` borrow. It is a
  first-class consumer of this substrate, not an indirect one — migrate it to
  `&[UncertaintyId]` alongside d1, and note its return type is what Task 0's new golden
  covers.
- `d1_reach.rs:185` and the `unc` flags — "does this node have uncertainty" becomes
  `!ctx.uncertainties.is_empty_set(sid)`, an O(1) window check.
- `d1_dataflow.rs:408` (the `#[cfg(test)]` oracle) and `:2530`/`:2535` (the union).
- `detectors/d1.rs:1170` / the fixture builders at `:2392`, `d1_dataflow.rs:4002`/`:5339`.

- [ ] **Step 1:** Write a test asserting that a node whose set is empty is
  distinguishable from a node with NO entry — the current `HashMap::get` returns `None`
  for both, and the migration must preserve whatever the existing code does. **Read
  `d1_reach.rs:185` first and state its current behaviour in the test**, rather than
  assuming the two cases are equivalent.
- [ ] **Step 2-4:** Migrate, run the d1 suite, prove discrimination by making
  `is_empty_set` return `false` for a genuinely empty set and watching the `unc`-flag test
  fail.
- [ ] **Step 5:** Full gate + commit.

---

### Task 3: d1 consumes ids; delete the pointer-keyed cache

**Files:** `d1_cohort.rs`, `d1_dataflow.rs`

With elements already interned, `path_uncertainty_ids` becomes: collect the path's
`UncertaintySetId`s (keeping the LAST occurrence of each — the equivalence argument in the
current doc comment carries over verbatim and must be MOVED, not re-derived), look up a
memo keyed by that `SmallVec<[UncertaintySetId; 8]>`, else concatenate
`ctx.uncertainties.elements(sid)` slices and `dedupe`.

Deletions this enables — each is the point of the task, not a side effect:
- `PathUncertaintyCache::by_set`, `set_id`, `SetEntry`, and the `_keep_alive` clone: the
  pointer key and its whole soundness argument are GONE, replaced by a dense id the
  substrate guarantees.
- `UncertaintyTable::intern` / `by_value` / `entries` / `keys` / `lites` in `d1_cohort.rs`
  — now the index's job. `UncertaintyTable` either disappears or becomes a thin borrow.
- `TerminalSink::finalize`'s `UncertaintyTable` hand-off, if the index is on `ctx`.

- [ ] **Step 1:** Keep the two existing cache tests passing in their new form
  (`path_cache_*`), re-pointed at set ids rather than pointers.
- [ ] **Step 2-4:** Migrate; re-run the two discrimination breaks from `12f7e95`
  (memo-bypass, order-insensitive lookup) against the new implementation.
- [ ] **Step 5:** Full gate + commit. Expect the census to report
  `unc_elems_interned=0` — d1 interns nothing any more.

---

### Task 4: Drop the record-carrying map (the parked −88 MiB)

**Files:** `detector_context.rs` + the `Arc<[Uncertainty]>` consumers

With every consumer on ids, `share()`'s materialized `Arc<[Uncertainty]>` has no readers.
Delete it; `uncertainties_by_node` holds only set ids, and the records live once in
`UncertaintyIndex::values`.

- [ ] **Step 1:** Re-run the census the uncertainty-substrate arc used
  (its CHANGELOG section names the probe) and record live bytes/allocations for the
  structure before and after.
- [ ] **Step 2:** Full gate. **Step 3:** Commit with the measured delta, and state
  plainly that it is a BC-Base-App figure — on DO this whole structure is 2.8 MiB, so
  this is a "survive Base App" win, not a customer-workspace one (the same caveat the
  parked entry already carries).

---

### Task 5 (GATED): `FindingConfidence` carries ids

Build ONLY if Task 4's measurement leaves d1's retained `confidence` bucket where the
parked entry predicts (~115 MiB). Ids are now run-global, so the blocker that made this
awkward — a d1-local table that died with the detector — is gone. Three readers:
`project_evidence`, `merge_confidence`, `format_policy::finding_to_jv`. If Task 4 already
moved the number, re-scope and say so instead of building to a stale prediction.

---

### Task 6: Capstone

- [ ] Full `cargo test`, `cargo clippy --all-targets --all-features`, `scripts/check-goldens`.
- [ ] Both corpora `--deterministic` SHAs against the constants in Global Constraints.
- [ ] CHANGELOG section. **Lead with the memory figures and the deleted re-derivation, not
      with wall-clock** — see "What this is and is not".
- [ ] `docs/OUTSTANDING.md`: close BOTH parked uncertainty items if delivered, or restate
      what remains with a fresh wake condition.
- [ ] Record the arc's own lesson: the pool computed set identity and discarded it, and
      three separate consumers then paid to re-derive it. Look for that shape elsewhere.

## Self-Review

**Coverage.** Task 0 closes the coverage hole the whole arc depends on; 1 creates identity;
2 publishes it; 3 consumes it and deletes the re-derivation; 4 collects the memory; 5 is
gated on measurement; 6 gates and records.

**Placeholders.** Task 0 Step 2 deliberately says "read `sub_summary_uncertainties` and
write the fixture from that reading" rather than inventing fixture AL here — the shape
depends on code the plan author has not read line-by-line, and guessing it would be the
exact failure this plan's own Task 0 exists to prevent.

**Type consistency.** `UncertaintyId` is re-declared in `detector_context.rs` and REPLACES
`d1_cohort::UncertaintyId` (Task 3 deletes the latter); every signature above uses the new
one. `UncertaintySetId` appears first in Task 1 and is consumed unchanged in 2 and 3.

**Risk.** Task 2 touches `d1_reach`'s `unc` flags, which feed `ContextKey.unc` and
therefore cohort partitioning — a wrong answer there re-partitions cohorts and moves
output. That is why Task 2's test is about the empty-set/no-entry distinction specifically,
and why the byte-identity gate runs at every task rather than only at the end.
