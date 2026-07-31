# Transaction-Spans Interning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cut `context.transaction_spans` — measured at **76.16 s, 39.0 % of the whole
`alsem analyze` wall on BC Base App 8020** — to a small fraction of that, with
**byte-identical output**, by replacing its `String`-keyed BFS, its per-visited-routine
`Vec<String>` materialization and its per-commit-op payload clones with interned `u32`
ids, borrowed id windows and shared payloads.

**Architecture:** `compute_transaction_spans` is a faithful al-sem port that still carries
al-sem's data model. **Task 0's census identified which part of that model actually costs
the 58.90 s, and it is not the part this plan originally assumed** — see the REVISION
below. The cost is `aggregate_span`'s per-visited-routine union: for every routine in
every span's cone it calls `ConeDerivedStore::writes_tables_of` / `publishes_events_of`,
each of which resolves that routine's whole interned cone window into a **freshly
allocated `Vec<String>`**, inserts every element into a `BTreeSet<String>` (another
allocation each) and drops it. Measured on 8020: **261,772,789 strings allocated and
discarded**, 2,023 per visited routine. The fix keeps the algorithm and results EXACTLY as
they are and changes only the representation — union the interned `ResId`s in a bitset and
resolve to `String` ONCE per span template.

## REVISION 2026-07-31 (after Task 0) — what the census falsified

The plan as first written blamed three cost centres. The census priced them:

| assumed cost centre | measured population (8020) | verdict |
|---|---|---|
| `String`-keyed backward BFS (`backward_cone`) | 927 walks / **129,350** visited steps total | **FALSIFIED** — cannot be seconds. Task 1 as written (SpanIndex/CSR/interned-ix BFS) is NOT built. |
| per-visited-routine `Vec<String>` materialization | **261,772,789** strings allocated and dropped | **CONFIRMED — this is the whole cost.** Now Task 1. |
| per-commit-op payload deep clone | 1,061 spans from 927 templates ⇒ only 134 duplicated payloads, 2,390,888 payload strings retained | **MOSTLY FALSIFIED** — the sharing saves ~13 % of a retained set that is not the bottleneck. Demoted to optional Task 2, gated on a measurement. |

The original Task 1/2/3 texts are preserved below the line as ARCHIVED so the falsified
predictions stay auditable; the live tasks are the revised ones.

**Tech Stack:** Rust (edition per workspace `Cargo.toml`), no new dependencies. Reuses
`crate::engine::l4::cone_derived::{ConeDerivedStore, ResId}` and the existing
`crate::engine::perf_trace` (`pt::span`) instrumentation.

## Global Constraints

- **Output must stay byte-identical at every task boundary.** The gate is
  `scripts/check-goldens` (all 9 targets, zero files under `tests/` moved) PLUS a DO
  workspace `--deterministic` SHA-256 comparison against the Task-0 baseline. A task
  that moves either is not done.
- **`TransactionSpan`'s VALUES may not change; its TYPES may** (Task 3 changes four
  fields from `Vec<String>` to `Arc<[String]>`). We own every consumer — `d8`, `d9`,
  `d50`, `detector_context.rs` and their tests — and update them in the same commit.
- **Determinism is a hard requirement.** Every id list in a `TransactionSpan` is sorted;
  the interner is assigned in sorted-id order specifically so that ordering survives the
  representation change without a re-sort. No `HashMap` iteration order may reach output.
- **Build with `TREE_SITTER_AL_PATH` set** — this is a git worktree and does not have its
  own submodule checkout:
  `export TREE_SITTER_AL_PATH=U:/Git/al-call-hierarchy/tree-sitter-al`
- **Never `cargo fmt`.** Format only touched files: `rustfmt <file>`.
- **Never pipe a gate through `| tail`** — the exit code becomes `tail`'s. Redirect to a
  log and `grep` it.
- **Measurement honesty.** 8020 wall figures for this corpus/probe swing **±80 s** run to
  run (recorded in `docs/OUTSTANDING.md`). A claim rests on same-run traced SPAN totals
  (`context.transaction_spans`), never on `analyze.total` alone.
- **Test doctrine (`CLAUDE.md`, "Testing Philosophy & Goldens").** A test must pin the
  USE, not the helper, and must be proven to discriminate: break the thing, watch it fail,
  revert, watch it pass, and RECORD both outcomes in the commit message.

## Environment

```bash
export WT=U:/Git/al-call-hierarchy/.claude/worktrees/perf-transaction-spans
export TREE_SITTER_AL_PATH=U:/Git/al-call-hierarchy/tree-sitter-al
export CORPUS_8020='C:/Users/SShadowS/AppData/Local/Temp/claude/U--Git-al-call-hierarchy/66efc3ec-d07b-48e2-8181-95ce2f62dd04/scratchpad/corpus-8020'
export DO_WS='U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud'
cd "$WT"
```

`corpus-8020` = BC Base App, 8,020 `.al` files / 100,941 routines (the corpus that
actually exercises this code path). `DO_WS` = the real Continia customer workspace —
small (4,842 routines), fast (~10 s), and the byte-identity oracle.

## File Structure

| File | Responsibility after this plan |
|---|---|
| `src/engine/l5/transaction_spans.rs` (modify) | Unchanged public entry point `compute_transaction_spans`; internally an interned-ix BFS + `ResId` aggregation. Gains a private `SpanIndex` substrate section and the `ALSEM_TXSPAN_CENSUS` census. |
| `src/engine/l4/cone_derived.rs` (modify) | Gains two borrowing accessors — `writes_table_ids_of` / `event_ids_of` — beside the existing `String`-materializing ones. No behaviour change to existing methods. |
| `src/engine/l5/detectors/d8.rs`, `d9.rs`, `d50.rs` (modify, Task 3 only) | Consume `Arc<[String]>` payload fields instead of `Vec<String>`. |
| `src/engine/l5/detector_context.rs` (modify, Task 3 only) | Test/struct construction sites updated for the new field types. |
| `docs/2026-07-31-transaction-spans-measurements.md` (create, Task 0; appended each task) | The measurement ledger: baseline, per-task span totals, population census, and the honest caveats. |

No new module: the substrate is ~120 lines and is only ever used by this one function.
Splitting it out would separate code that changes together for no reader benefit.

---

### Task 0: Baseline, census, and the identity oracle

**Files:**
- Modify: `src/engine/l5/transaction_spans.rs` (add the env-gated census)
- Create: `docs/2026-07-31-transaction-spans-measurements.md`

**Interfaces:**
- Produces: the baseline artifacts every later task compares against —
  `logs/txspan-base-do.sha256` (DO output SHA), `logs/trace-txspan-base.json` (8020 trace),
  and the population figures in the measurements doc.

- [ ] **Step 1: Build the baseline binary**

```bash
cd "$WT"
cargo build --profile release-fast --bin alsem > logs/txspan-base-build.log 2>&1
echo "EXIT=$?"
grep -E "^error" logs/txspan-base-build.log || echo "build clean"
```

Expected: `EXIT=0`.

- [ ] **Step 2: Capture the DO byte-identity baseline**

```bash
mkdir -p logs
./target/release-fast/alsem.exe analyze "$DO_WS" --format json --deterministic \
  > logs/txspan-base-do.json 2> logs/txspan-base-do.err
echo "EXIT=$?"
sha256sum logs/txspan-base-do.json | tee logs/txspan-base-do.sha256
```

Expected: `EXIT=0` and a recorded SHA. **This SHA is the identity oracle for Tasks 1-3.**

- [ ] **Step 3: Capture the 8020 baseline trace**

```bash
ALSEM_TRACE=1 ALSEM_TRACE_DETAIL=hot ALSEM_TRACE_FILE="$WT/logs/trace-txspan-base.json" \
  ./target/release-fast/alsem.exe analyze "$CORPUS_8020" --format json \
  > logs/txspan-base-8020.json 2> logs/txspan-base-8020.err
echo "EXIT=$?"
python - <<'EOF'
import json, collections
d = json.load(open('logs/trace-txspan-base.json'))
stack = collections.defaultdict(list); tot = collections.Counter()
for e in d:
    k = (e['pid'], e['tid'])
    if e['ph'] == 'B': stack[k].append(e)
    elif e['ph'] == 'E' and stack[k]:
        b = stack[k].pop(); tot[b['name']] += e['ts'] - b['ts']
for n in ('analyze.total', 'context.transaction_spans'):
    print(f"{n}: {tot[n]/1e6:.2f}s")
EOF
```

Expected: both spans print. Record them in the measurements doc.

- [ ] **Step 4: Write the population census**

Add to `src/engine/l5/transaction_spans.rs`, immediately above `compute_transaction_spans`:

```rust
/// `ALSEM_TXSPAN_CENSUS=1` — the population this module actually processes, printed
/// to stderr at the end of `compute_transaction_spans`. Diagnostic only: no
/// production path reads it and it allocates nothing unless the env var is set.
/// Mirrors the `C1_CONE_CENSUS` convention in `crate::engine::l4::cone_census`.
///
/// The four figures are the ones that PRICE this module: `templates` is how many
/// backward BFS walks actually run (the cache already collapses ops of one seed
/// routine onto one walk), `visited_total` is the number of per-routine aggregate
/// steps summed over those walks — the population every per-visited-routine cost is
/// multiplied by — and `payload_strings` is how many `String`s the emitted spans
/// retain, which is what Task 3's sharing removes.
struct TxSpanCensus {
    seed_routines: usize,
    templates: usize,
    visited_total: usize,
    spans_emitted: usize,
    payload_strings: usize,
}

impl TxSpanCensus {
    fn enabled() -> bool {
        std::env::var("ALSEM_TXSPAN_CENSUS").as_deref() == Ok("1")
    }

    fn report(&self) {
        eprintln!(
            "[txspan-census] seed_routines={} templates={} visited_total={} \
             spans_emitted={} payload_strings={} mean_cone={:.1}",
            self.seed_routines,
            self.templates,
            self.visited_total,
            self.spans_emitted,
            self.payload_strings,
            if self.templates == 0 {
                0.0
            } else {
                self.visited_total as f64 / self.templates as f64
            },
        );
    }
}
```

Wire it: give `compute_transaction_spans` a `let mut census = TxSpanCensus { seed_routines: 0, templates: 0, visited_total: 0, spans_emitted: 0, payload_strings: 0 };`, increment
`seed_routines` once per distinct seed id seen, `templates`/`visited_total` inside
`span_template`'s cache-miss arm (pass `&mut census` in), `spans_emitted` per pushed span,
`payload_strings` by `t.routines_in_span.len() + t.writes_tables.len() + t.publishes_events.len() + t.span_roots.len()` per pushed span, and end the function with:

```rust
    if TxSpanCensus::enabled() {
        census.report();
    }
    spans
```

- [ ] **Step 5: Run the census on both corpora**

```bash
cargo build --profile release-fast --bin alsem > logs/txspan-census-build.log 2>&1; echo "EXIT=$?"
ALSEM_TXSPAN_CENSUS=1 ./target/release-fast/alsem.exe analyze "$DO_WS" --format json \
  > /dev/null 2> logs/txspan-census-do.err
grep txspan-census logs/txspan-census-do.err
ALSEM_TXSPAN_CENSUS=1 ./target/release-fast/alsem.exe analyze "$CORPUS_8020" --format json \
  > /dev/null 2> logs/txspan-census-8020.err
grep txspan-census logs/txspan-census-8020.err
```

Expected: one `[txspan-census]` line per corpus. **Record both in the measurements doc.**
`visited_total` on 8020 is the number this whole plan is about — if it is small (say under
1 M) then the cost is NOT where this plan assumes and the remaining tasks must be
re-scoped before implementing. Say so in the doc rather than proceeding on the assumption.

- [ ] **Step 6: Verify the census changed nothing**

```bash
./target/release-fast/alsem.exe analyze "$DO_WS" --format json --deterministic \
  > logs/txspan-t0-do.json 2>/dev/null
sha256sum logs/txspan-t0-do.json
cat logs/txspan-base-do.sha256
```

Expected: the two SHAs match (filenames differ; compare the hashes).

- [ ] **Step 7: Commit**

```bash
rustfmt src/engine/l5/transaction_spans.rs
git add src/engine/l5/transaction_spans.rs docs/2026-07-31-transaction-spans-measurements.md
git commit -m "perf(l5): census the transaction-spans population before changing it"
```

---

## LIVE TASKS (post-census)

### Task R1: Union interned ids in a bitset; resolve once per template

**Files:**
- Modify: `src/engine/l4/cone_derived.rs` (borrowing accessors + `resolve_res`)
- Modify: `src/engine/l5/transaction_spans.rs` (`aggregate_span`)
- Test: both of the above

**Interfaces:**
- Produces:
  - `ConeDerivedStore::writes_table_ids_of(&self, routine_id: &str) -> &[ResId]`
  - `ConeDerivedStore::event_ids_of(&self, routine_id: &str) -> &[ResId]`
  - `ConeDerivedStore::resolve_res(&self, id: ResId) -> &str`
  - `ConeDerivedStore::res_universe_len(&self) -> usize` — the bitset width
  - `struct ResBitset { words: Vec<u64> }` with `insert_all(&mut self, ids: &[ResId])`,
    `clear(&mut self)`, `iter_ids(&self) -> impl Iterator<Item = ResId> + '_`

**Why a bitset and NOT a `Vec<ResId>` accumulator.** Accumulating the windows would push
**261,772,789** `u32`s — 1.05 GB of scratch — to dedupe at the end. That trades 58.90 s of
CPU for a gigabyte of RSS, which this arc explicitly must not do (`context.transaction_spans`
retains only 73 MB today). A bitset over the resource-id universe is a few KB, costs one
word-OR per id, and dedupes for free. This paragraph exists because the accumulator is the
obvious first move and it is wrong.

- [ ] **Step 1: Write the failing tests**

In `src/engine/l4/cone_derived.rs`'s `mod tests` — the accessors agree with the
`String`-materializing ones, stated over TWO routines so a window/pool mix-up cannot pass:

```rust
    #[test]
    fn id_accessors_agree_with_the_string_accessors() {
        use crate::engine::l5::test_support::{cone_store_of, coverage, fact, summary};
        use std::collections::HashMap;

        let mut summaries: HashMap<String, crate::engine::l5::full_summary::FullRoutineSummary> =
            HashMap::new();
        summaries.insert(
            "r/one".to_string(),
            summary(
                "r/one",
                vec![
                    fact("insert", "table", Some("t/B")),
                    fact("insert", "table", Some("t/A")),
                    fact("publish", "event", Some("e/Z")),
                ],
                vec![],
                Some(coverage("complete")),
            ),
        );
        summaries.insert(
            "r/two".to_string(),
            summary(
                "r/two",
                vec![fact("insert", "table", Some("t/A"))],
                vec![],
                Some(coverage("complete")),
            ),
        );
        let store = cone_store_of(&summaries);

        for rid in ["r/one", "r/two"] {
            let mut via_ids: Vec<String> = store
                .writes_table_ids_of(rid)
                .iter()
                .map(|id| store.resolve_res(*id).to_string())
                .collect();
            via_ids.sort();
            assert_eq!(via_ids, store.writes_tables_of(rid), "writes mismatch for {rid}");

            let mut ev: Vec<String> = store
                .event_ids_of(rid)
                .iter()
                .map(|id| store.resolve_res(*id).to_string())
                .collect();
            ev.sort();
            assert_eq!(ev, store.publishes_events_of(rid), "events mismatch for {rid}");
        }
    }

    #[test]
    fn res_bitset_dedupes_and_iterates_ascending() {
        let mut bs = ResBitset::new(8);
        bs.insert_all(&[5, 1, 5, 3]);
        bs.insert_all(&[3, 7]);
        assert_eq!(bs.iter_ids().collect::<Vec<ResId>>(), vec![1, 3, 5, 7]);
        bs.clear();
        assert_eq!(bs.iter_ids().collect::<Vec<ResId>>(), Vec::<ResId>::new());
    }
```

In `src/engine/l5/transaction_spans.rs`'s `mod tests` — the union is deduped across
routines and a missing summary still poisons coverage (this is the REGRESSION pin for the
rewrite; Step 4 proves it discriminates):

```rust
    #[test]
    fn aggregate_unions_deduped_and_a_missing_summary_breaks_coverage() {
        let routines = vec![
            routine("root", "trigger"),
            routine("mid", "procedure"),
            op_commit_routine("committer", "procedure", &["c/op"]),
        ];
        let graph = graph_from_edges(
            &["root", "mid", "committer"],
            &[edge("root", "mid", "cs1"), edge("mid", "committer", "cs2")],
        );
        let reverse = build_reverse_call_graph(&graph);

        let mut summaries: HashMap<String, FullRoutineSummary> = HashMap::new();
        // BOTH write t/A; committer also writes t/B. `root` has NO summary at all.
        summaries.insert(
            "committer".to_string(),
            summary(
                "committer",
                vec![
                    fact("insert", "table", Some("t/A")),
                    fact("insert", "table", Some("t/B")),
                ],
                vec![],
                Some(coverage("complete")),
            ),
        );
        summaries.insert(
            "mid".to_string(),
            summary(
                "mid",
                vec![fact("insert", "table", Some("t/A"))],
                vec![],
                Some(coverage("complete")),
            ),
        );

        let no_deps = BTreeSet::new();
        let spans = compute_transaction_spans(
            &routines,
            &no_deps,
            &reverse,
            &summaries,
            &cone_store_of(&summaries),
        );
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].writes_tables, vec!["t/A", "t/B"], "sorted + deduped union");
        assert!(
            !spans[0].coverage_complete,
            "`root` has no summary, so the span is not coverage-complete"
        );
    }
```

- [ ] **Step 2: Run to verify failure**

```bash
cd "$WT"
cargo test -p al-call-hierarchy --lib cone_derived > logs/r1-red.log 2>&1
grep -E "^error|cannot find|no method" logs/r1-red.log | head -5
```

Expected: FAIL — `no method named writes_table_ids_of`, `cannot find type ResBitset`.

- [ ] **Step 3: Implement**

In `src/engine/l4/cone_derived.rs`, beside `writes_tables_of`:

```rust
    /// The routine's written-TableId window as interned ids, BORROWED — the
    /// allocation-free half of [`Self::writes_tables_of`]. The window is sorted and
    /// deduped BY ID (`freeze_ids`), which is NOT the order of the resolved strings,
    /// so a caller that needs string order resolves and sorts. A caller that unions
    /// across many routines should union the IDS and resolve once — that is the whole
    /// point of this accessor (`transaction_spans` was allocating 261,772,789 strings
    /// per 8020 run through the `String` variant).
    pub fn writes_table_ids_of(&self, routine_id: &str) -> &[ResId] {
        window(&self.writes_all_pool, &self.row(routine_id).table_writes_all)
    }

    /// The routine's published-EventId window as interned ids, BORROWED. Same
    /// contract as [`Self::writes_table_ids_of`].
    pub fn event_ids_of(&self, routine_id: &str) -> &[ResId] {
        window(&self.events_pool, &self.row(routine_id).event_publishes)
    }

    /// Resolve one interned resource id, for callers that union borrowed id windows
    /// and materialize the result themselves.
    pub fn resolve_res(&self, id: ResId) -> &str {
        self.interner.resolve(id)
    }

    /// Number of distinct interned resource ids — the width a [`ResBitset`] needs to
    /// hold any set this store can produce.
    pub fn res_universe_len(&self) -> usize {
        self.interner.len()
    }
```

And the bitset, in the same file next to `ResId`:

```rust
/// A dense set of [`ResId`]s over the store's resource-id universe. Exists so a caller
/// unioning many routines' id windows pays one word-OR per id and no allocation per
/// element, instead of resolving each window into a `Vec<String>` and inserting every
/// element into a `BTreeSet<String>`.
///
/// `iter_ids` yields ascending ID order, which is intern order — NOT lexicographic. A
/// caller that needs the old `BTreeSet<String>` order resolves and then sorts by string.
#[derive(Debug, Default)]
pub struct ResBitset {
    words: Vec<u64>,
}

impl ResBitset {
    pub fn new(universe_len: usize) -> Self {
        ResBitset {
            words: vec![0u64; universe_len.div_ceil(64)],
        }
    }

    pub fn insert_all(&mut self, ids: &[ResId]) {
        for &id in ids {
            let w = (id as usize) / 64;
            if w >= self.words.len() {
                self.words.resize(w + 1, 0);
            }
            self.words[w] |= 1u64 << ((id as usize) % 64);
        }
    }

    pub fn clear(&mut self) {
        self.words.iter_mut().for_each(|w| *w = 0);
    }

    pub fn iter_ids(&self) -> impl Iterator<Item = ResId> + '_ {
        self.words.iter().enumerate().flat_map(|(wi, &w)| {
            let mut bits = w;
            std::iter::from_fn(move || {
                if bits == 0 {
                    return None;
                }
                let b = bits.trailing_zeros();
                bits &= bits - 1;
                Some((wi as u32) * 64 + b)
            })
        })
    }
}
```

In `src/engine/l5/transaction_spans.rs`, rewrite `aggregate_span` to take two caller-owned
bitsets and resolve once:

```rust
fn aggregate_span(
    visited: &BTreeSet<String>,
    summaries: &HashMap<String, FullRoutineSummary>,
    cone_derived: &ConeDerivedStore,
    writes_bs: &mut ResBitset,
    events_bs: &mut ResBitset,
    census: &mut TxSpanCensus,
) -> (Vec<String>, Vec<String>, bool) {
    writes_bs.clear();
    events_bs.clear();
    let mut coverage_complete = true;
    for rid in visited {
        let Some(summary) = summaries.get(rid) else {
            coverage_complete = false;
            continue;
        };
        writes_bs.insert_all(cone_derived.writes_table_ids_of(&summary.routine_id));
        events_bs.insert_all(cone_derived.event_ids_of(&summary.routine_id));
        if reachable_coverage(summary, None) != "complete" {
            coverage_complete = false;
        }
    }
    let writes = resolve_sorted_ids(writes_bs, cone_derived, census);
    let events = resolve_sorted_ids(events_bs, cone_derived, census);
    (writes, events, coverage_complete)
}

/// Resolve a `ResId` set into the sorted-unique `Vec<String>` the old
/// `BTreeSet<String>` produced. The set is already unique (a bitset cannot hold a
/// duplicate) and the interner is injective, so id-dedupe IS string-dedupe; the sort is
/// by STRING because intern order is not lexicographic.
fn resolve_sorted_ids(
    bs: &ResBitset,
    cone_derived: &ConeDerivedStore,
    census: &mut TxSpanCensus,
) -> Vec<String> {
    let mut out: Vec<String> = bs
        .iter_ids()
        .map(|id| cone_derived.resolve_res(id).to_string())
        .collect();
    census.materialized_strings += out.len();
    out.sort();
    out
}
```

Allocate the two bitsets ONCE in `compute_transaction_spans` (before the seed loops) and
thread them through `span_template`:

```rust
    let mut writes_bs = ResBitset::new(cone_derived.res_universe_len());
    let mut events_bs = ResBitset::new(cone_derived.res_universe_len());
```

- [ ] **Step 4: Run the tests, then prove they discriminate**

```bash
cargo test -p al-call-hierarchy --lib cone_derived > logs/r1-green-cone.log 2>&1
cargo test -p al-call-hierarchy --lib transaction_spans > logs/r1-green-tx.log 2>&1
grep -E "test result" logs/r1-green-cone.log logs/r1-green-tx.log
```

Expected: `ok` for both, every pre-existing `transaction_spans` test included — they are
the semantic oracle for this rewrite.

Then break each, record PASS→FAIL→PASS:
1. `resolve_sorted_ids`: drop the `out.sort()`. Expected:
   `aggregate_unions_deduped_and_a_missing_summary_breaks_coverage` FAILS whenever intern
   order differs from lexicographic — if it passes, the fixture is too weak; extend it
   with a table pair whose intern order is the reverse of their string order and re-run.
2. `aggregate_span`: change the missing-summary arm to a bare `continue;`. Expected: the
   same test FAILS on the coverage assertion.
3. `ResBitset::insert_all`: use `=` instead of `|=`. Expected:
   `res_bitset_dedupes_and_iterates_ascending` FAILS.
4. `writes_table_ids_of`: return the whole `&self.writes_all_pool`. Expected:
   `id_accessors_agree_with_the_string_accessors` FAILS.

- [ ] **Step 5: Byte-identity gate**

```bash
TREE_SITTER_AL_PATH=U:/Git/al-call-hierarchy/tree-sitter-al \
  cargo build --profile release-fast --bin alsem > logs/r1-build.log 2>&1
echo "CARGO_EXIT=$?"; ls -la target/release-fast/alsem.exe   # mtime MUST move (lock hazard)
./target/release-fast/alsem.exe analyze "$DO_WS" --format json --deterministic \
  > logs/txspan-r1-do.json 2>/dev/null
sha256sum logs/txspan-r1-do.json; cat logs/txspan-base-do.sha256
bash scripts/check-goldens > logs/r1-goldens.log 2>&1; echo "EXIT=$?"
git status --short tests/ | head
```

Expected: SHA equals `f022f677d2650b2399fc3aa5a7625bc6c078d90dd51cdb80e1e3705808fee3ea`,
`check-goldens` EXIT=0, zero files under `tests/` modified.

- [ ] **Step 6: Measure and commit**

```bash
ALSEM_TXSPAN_CENSUS=1 ALSEM_TRACE=1 ALSEM_TRACE_DETAIL=hot \
  ALSEM_TRACE_FILE="$WT/logs/trace-txspan-r1.json" \
  ./target/release-fast/alsem.exe analyze "$CORPUS_8020" --format json \
  > /dev/null 2> logs/r1-8020.err
grep txspan-census logs/r1-8020.err
# span totals via the Task 0 Step 3 python snippet against trace-txspan-r1.json
rustfmt src/engine/l4/cone_derived.rs src/engine/l5/transaction_spans.rs
git add -u src/engine docs/2026-07-31-transaction-spans-measurements.md
git commit -m "perf(l5): span unions ride interned ids in a bitset, resolved once"
```

The commit message states: `materialized_strings` before/after (261,772,789 → measured),
the `context.transaction_spans` span total before/after, the four discrimination results,
and the DO SHA.

### Task R2 (OPTIONAL — gated): share span payloads across a seed's commit ops

**Gate:** build this ONLY if, after R1, `context.transaction_spans` is still worth
attacking AND the retained payload matters. The census says 1,061 spans come from 927
templates, so sharing removes duplicate payloads for **134 spans** — roughly 13 % of
2,390,888 retained payload strings. If R1 leaves the span under ~5 s, record the
measurement and SKIP this task rather than spending a consumer-wide type change on it.

If built, it is the ARCHIVED Task 3 below, unchanged: `TransactionSpan`'s four payload
fields become `Arc<[String]>`, emission becomes `Arc::clone`, and `d8`/`d9`/`d50`/`d17`/
`detector_context` are updated in the same commit. Its `Arc::ptr_eq` test and its
discrimination proof are as written there.

### Task R3: Capstone

As ARCHIVED Task 4 below, with one addition: the CHANGELOG and the measurements doc must
record the **falsified premise** (interned-ix BFS, 129,350 visited steps) alongside the
confirmed one, because the census that killed it is the reusable lesson — measure the
population before building the taxonomy for it.

---

## ARCHIVED TASKS (premise falsified by Task 0 — kept for audit, NOT executed)

### Task 1 (ARCHIVED): Interned-ix backward cone

**Files:**
- Modify: `src/engine/l5/transaction_spans.rs`
- Test: `src/engine/l5/transaction_spans.rs` (its own `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces, for Task 2:
  - `struct SpanIndex { ids: Vec<String>, by_id: HashMap<String, u32>, callers_start: Vec<u32>, callers: Vec<u32>, is_commit: Vec<bool> }`
  - `impl SpanIndex { fn build(routines: &[L3Routine], commits_by_routine: &BTreeMap<String, Vec<String>>, reverse: &ReverseCallGraph) -> Self; fn ix(&self, id: &str) -> Option<u32>; fn id(&self, ix: u32) -> &str; fn callers_of_ix(&self, ix: u32) -> &[u32]; fn len(&self) -> usize }`
  - `fn backward_cone_ix(seed: u32, index: &SpanIndex, visited_stamp: &mut [u32], gen: u32, queue: &mut Vec<(u32, u32)>, out: &mut Vec<u32>)` — fills `out` with the visited ix set **sorted ascending**, which is lexicographic id order by construction.

**Why sorted-id interning is load-bearing:** `routines_in_span` and `span_roots` are
`BTreeSet<String>`-derived, i.e. sorted by the id STRING. If ix assignment follows sorted
id order, an ascending-ix `Vec<u32>` resolves directly to that same order with no re-sort.
Any other assignment order silently changes output order.

- [ ] **Step 1: Write the failing tests**

Add to the existing `mod tests` in `src/engine/l5/transaction_spans.rs`:

```rust
    /// The interner's ix order IS lexicographic id order — the property every
    /// output list's ordering rests on. Hand-stated: the ids are supplied in
    /// DELIBERATELY unsorted order, so an implementation that interns in
    /// insertion order fails this.
    #[test]
    fn span_index_ix_order_is_lexicographic_id_order() {
        let routines = vec![
            routine("zeta", "procedure"),
            routine("alpha", "procedure"),
            routine("mid", "procedure"),
        ];
        let graph = graph_from_edges(
            &["zeta", "alpha", "mid"],
            &[edge("alpha", "mid", "cs1"), edge("mid", "zeta", "cs2")],
        );
        let reverse = build_reverse_call_graph(&graph);
        let commits: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let index = SpanIndex::build(&routines, &commits, &reverse);

        let ordered: Vec<&str> = (0..index.len() as u32).map(|i| index.id(i)).collect();
        let mut sorted = ordered.clone();
        sorted.sort();
        assert_eq!(ordered, sorted, "ix order must equal sorted id order");
    }

    /// The BFS reproduces the OLD `backward_cone` semantics exactly, including
    /// the two subtle rules: a committing routine other than the seed is entered
    /// but NOT expanded, and the seed itself is expanded even though it commits.
    #[test]
    fn backward_cone_ix_stops_at_a_foreign_committer_but_expands_the_seed() {
        // root -> outer_committer -> mid -> seed_committer
        // Cone from seed_committer = {seed_committer, mid, outer_committer}
        // (root is behind outer_committer, which is absorbing).
        let routines = vec![
            routine("root", "trigger"),
            op_commit_routine("outer_committer", "procedure", &["o/op"]),
            routine("mid", "procedure"),
            op_commit_routine("seed_committer", "procedure", &["s/op"]),
        ];
        let graph = graph_from_edges(
            &["root", "outer_committer", "mid", "seed_committer"],
            &[
                edge("root", "outer_committer", "cs1"),
                edge("outer_committer", "mid", "cs2"),
                edge("mid", "seed_committer", "cs3"),
            ],
        );
        let reverse = build_reverse_call_graph(&graph);
        let mut commits: BTreeMap<String, Vec<String>> = BTreeMap::new();
        commits.insert("outer_committer".to_string(), vec!["o/op".to_string()]);
        commits.insert("seed_committer".to_string(), vec!["s/op".to_string()]);

        let index = SpanIndex::build(&routines, &commits, &reverse);
        let mut stamp = vec![0u32; index.len()];
        let mut queue = Vec::new();
        let mut out = Vec::new();
        let seed = index.ix("seed_committer").expect("seed interned");
        backward_cone_ix(seed, &index, &mut stamp, 1, &mut queue, &mut out);

        let got: Vec<&str> = out.iter().map(|&i| index.id(i)).collect();
        assert_eq!(got, vec!["mid", "outer_committer", "seed_committer"]);
        assert!(!got.contains(&"root"), "root is behind an absorbing committer");
    }

    /// The generation stamp must isolate consecutive walks that share one buffer.
    /// Hand-stated: run two cones over the SAME stamp array with different gens
    /// and assert the second is not polluted by the first.
    #[test]
    fn backward_cone_ix_generation_stamp_isolates_consecutive_walks() {
        // a -> b   and   c -> d  (two disjoint chains)
        let routines = vec![
            routine("a", "procedure"),
            op_commit_routine("b", "procedure", &["b/op"]),
            routine("c", "procedure"),
            op_commit_routine("d", "procedure", &["d/op"]),
        ];
        let graph = graph_from_edges(
            &["a", "b", "c", "d"],
            &[edge("a", "b", "cs1"), edge("c", "d", "cs2")],
        );
        let reverse = build_reverse_call_graph(&graph);
        let mut commits: BTreeMap<String, Vec<String>> = BTreeMap::new();
        commits.insert("b".to_string(), vec!["b/op".to_string()]);
        commits.insert("d".to_string(), vec!["d/op".to_string()]);

        let index = SpanIndex::build(&routines, &commits, &reverse);
        let mut stamp = vec![0u32; index.len()];
        let mut queue = Vec::new();
        let mut out = Vec::new();

        backward_cone_ix(index.ix("b").unwrap(), &index, &mut stamp, 1, &mut queue, &mut out);
        let first: Vec<&str> = out.iter().map(|&i| index.id(i)).collect();
        assert_eq!(first, vec!["a", "b"]);

        backward_cone_ix(index.ix("d").unwrap(), &index, &mut stamp, 2, &mut queue, &mut out);
        let second: Vec<&str> = out.iter().map(|&i| index.id(i)).collect();
        assert_eq!(second, vec!["c", "d"], "gen 2 must not see gen 1's marks");
    }
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cargo test -p al-call-hierarchy --lib transaction_spans > logs/t1-red.log 2>&1
echo "EXIT=$?"; grep -E "^error|cannot find" logs/t1-red.log | head -5
```

Expected: FAIL — `cannot find type SpanIndex` / `cannot find function backward_cone_ix`.

- [ ] **Step 3: Implement the substrate**

Add above `backward_cone` in `src/engine/l5/transaction_spans.rs`:

```rust
/// Interned routine-id substrate for the span walks: `u32` ix per routine id, a
/// CSR reverse adjacency, and a per-ix "this routine commits" flag.
///
/// **Ix assignment is SORTED-ID order, and that is load-bearing.** Every id list a
/// `TransactionSpan` carries is sorted by the id STRING (they were `BTreeSet<String>`s).
/// Assigning ix in sorted order makes an ascending-ix `Vec<u32>` resolve to exactly that
/// order, so the walks never re-sort and the output ordering survives the representation
/// change by construction rather than by a later `sort()` that a reader has to trust.
///
/// The id universe is the UNION of the model routines, the reverse map's keys and every
/// caller `from` — the reverse graph can name nodes that are not in `routines` (dependency
/// routines, graph-only nodes), and the old `String` walk visited them happily.
struct SpanIndex {
    ids: Vec<String>,
    by_id: HashMap<String, u32>,
    /// CSR: callers of ix `i` are `callers[callers_start[i]..callers_start[i + 1]]`.
    callers_start: Vec<u32>,
    callers: Vec<u32>,
    is_commit: Vec<bool>,
}

impl SpanIndex {
    fn build(
        routines: &[L3Routine],
        commits_by_routine: &BTreeMap<String, Vec<String>>,
        reverse: &ReverseCallGraph,
    ) -> Self {
        let mut universe: BTreeSet<&str> = BTreeSet::new();
        for r in routines {
            universe.insert(r.id.as_str());
        }
        for (callee, edges) in reverse {
            universe.insert(callee.as_str());
            for e in edges {
                universe.insert(e.from.as_str());
            }
        }

        let ids: Vec<String> = universe.iter().map(|s| (*s).to_string()).collect();
        let by_id: HashMap<String, u32> = ids
            .iter()
            .enumerate()
            .map(|(i, s)| (s.clone(), i as u32))
            .collect();

        // Per-ix caller lists, built into CSR in one counting pass + one fill pass.
        let n = ids.len();
        let mut counts = vec![0u32; n];
        for (callee, edges) in reverse {
            let Some(&c_ix) = by_id.get(callee.as_str()) else {
                continue;
            };
            counts[c_ix as usize] += edges.len() as u32;
        }
        let mut callers_start = vec![0u32; n + 1];
        for i in 0..n {
            callers_start[i + 1] = callers_start[i] + counts[i];
        }
        let mut fill = callers_start.clone();
        let mut callers = vec![0u32; callers_start[n] as usize];
        for (callee, edges) in reverse {
            let Some(&c_ix) = by_id.get(callee.as_str()) else {
                continue;
            };
            for e in edges {
                let Some(&f_ix) = by_id.get(e.from.as_str()) else {
                    continue;
                };
                callers[fill[c_ix as usize] as usize] = f_ix;
                fill[c_ix as usize] += 1;
            }
        }

        let mut is_commit = vec![false; n];
        for id in commits_by_routine.keys() {
            if let Some(&ix) = by_id.get(id.as_str()) {
                is_commit[ix as usize] = true;
            }
        }

        SpanIndex {
            ids,
            by_id,
            callers_start,
            callers,
            is_commit,
        }
    }

    fn ix(&self, id: &str) -> Option<u32> {
        self.by_id.get(id).copied()
    }

    fn id(&self, ix: u32) -> &str {
        &self.ids[ix as usize]
    }

    fn callers_of_ix(&self, ix: u32) -> &[u32] {
        let s = self.callers_start[ix as usize] as usize;
        let e = self.callers_start[ix as usize + 1] as usize;
        &self.callers[s..e]
    }

    fn len(&self) -> usize {
        self.ids.len()
    }
}

/// Backward BFS over the interned reverse graph. Semantics are the OLD
/// `backward_cone`'s, verbatim: first visit wins (so recorded depth is the shortest
/// distance), a node at `MAX_DEPTH` is recorded but not expanded, and a COMMITTING node
/// other than the seed is recorded but not expanded (a prior span bounds the trace).
///
/// `visited_stamp` is a generation-stamped scratch array reused across every walk in a
/// run — `stamp[ix] == gen` means "visited in THIS walk" — so a walk costs nothing to
/// set up and nothing to clear. `out` is cleared on entry and left holding the visited
/// set **sorted ascending**, i.e. lexicographic id order (see `SpanIndex`).
fn backward_cone_ix(
    seed: u32,
    index: &SpanIndex,
    visited_stamp: &mut [u32],
    gen: u32,
    queue: &mut Vec<(u32, u32)>,
    out: &mut Vec<u32>,
) {
    out.clear();
    queue.clear();
    queue.push((seed, 0));
    let mut head = 0usize;
    while head < queue.len() {
        let (ix, depth) = queue[head];
        head += 1;
        if visited_stamp[ix as usize] == gen {
            continue;
        }
        visited_stamp[ix as usize] = gen;
        out.push(ix);
        if depth as usize >= MAX_DEPTH {
            continue;
        }
        if ix != seed && index.is_commit[ix as usize] {
            continue;
        }
        for &caller in index.callers_of_ix(ix) {
            if visited_stamp[caller as usize] != gen {
                queue.push((caller, depth + 1));
            }
        }
    }
    out.sort_unstable();
}
```

- [ ] **Step 4: Route `span_template` through it**

Change `backward_cone`'s call site only — keep `aggregate_span` / `span_roots_of` on
`BTreeSet<String>` for now, so this task changes ONE thing. In `span_template`, replace
the `let visited = backward_cone(seed, commits_by_routine, reverse);` line with:

```rust
        let mut cone_ix: Vec<u32> = Vec::new();
        *gen += 1;
        let seed_ix = index.ix(seed).expect("seed routine is in the index universe");
        backward_cone_ix(seed_ix, index, visited_stamp, *gen, queue, &mut cone_ix);
        let visited: BTreeSet<String> =
            cone_ix.iter().map(|&i| index.id(i).to_string()).collect();
```

and thread `index: &SpanIndex, visited_stamp: &mut [u32], gen: &mut u32, queue: &mut Vec<(u32, u32)>`
through `span_template`'s signature and both call sites in `compute_transaction_spans`.
Build the substrate once in `compute_transaction_spans`, immediately after
`commits_by_routine` is complete:

```rust
    let index = SpanIndex::build(routines, &commits_by_routine, reverse);
    let mut visited_stamp = vec![0u32; index.len()];
    let mut walk_gen = 0u32;
    let mut walk_queue: Vec<(u32, u32)> = Vec::new();
```

Delete the old `backward_cone` function — it now has zero call sites. (Leaving it would
be dead code that a future reader mistakes for the live path.)

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test -p al-call-hierarchy --lib transaction_spans > logs/t1-green.log 2>&1
echo "EXIT=$?"; grep -E "test result" logs/t1-green.log
```

Expected: `test result: ok.` with every pre-existing `transaction_spans` test still passing
— they are the semantic oracle for this rewrite.

- [ ] **Step 6: Prove the tests discriminate**

Run each break, record PASS/FAIL, then revert:

1. In `SpanIndex::build`, change `universe` from `BTreeSet<&str>` to a `Vec<&str>` filled
   in iteration order (no sort). Expected:
   `span_index_ix_order_is_lexicographic_id_order` FAILS.
2. In `backward_cone_ix`, drop the `ix != seed &&` guard so the seed is also absorbing.
   Expected: `backward_cone_ix_stops_at_a_foreign_committer_but_expands_the_seed` FAILS
   (the cone collapses to `{seed_committer}`).
3. In `backward_cone_ix`, replace `visited_stamp[ix] == gen` with `visited_stamp[ix] != 0`.
   Expected: `backward_cone_ix_generation_stamp_isolates_consecutive_walks` FAILS.

```bash
# after each edit
cargo test -p al-call-hierarchy --lib transaction_spans > logs/t1-break-N.log 2>&1
grep -E "test result|FAILED" logs/t1-break-N.log
git checkout -- src/engine/l5/transaction_spans.rs   # only if the break was uncommitted scratch
```

- [ ] **Step 7: Byte-identity gate**

```bash
cargo build --profile release-fast --bin alsem > logs/t1-build.log 2>&1; echo "EXIT=$?"
./target/release-fast/alsem.exe analyze "$DO_WS" --format json --deterministic \
  > logs/txspan-t1-do.json 2>/dev/null
sha256sum logs/txspan-t1-do.json; cat logs/txspan-base-do.sha256
bash scripts/check-goldens > logs/t1-goldens.log 2>&1; echo "EXIT=$?"
grep -E "FAIL|test result: FAILED" logs/t1-goldens.log || echo "goldens clean"
git status --short tests/ | head
```

Expected: SHAs equal, `check-goldens` EXIT=0, **zero files under `tests/` modified**.

- [ ] **Step 8: Measure and commit**

```bash
ALSEM_TRACE=1 ALSEM_TRACE_DETAIL=hot ALSEM_TRACE_FILE="$WT/logs/trace-txspan-t1.json" \
  ./target/release-fast/alsem.exe analyze "$CORPUS_8020" --format json \
  > /dev/null 2> logs/t1-8020.err
# same python span-total snippet as Task 0 Step 3, against trace-txspan-t1.json
rustfmt src/engine/l5/transaction_spans.rs
git add src/engine/l5/transaction_spans.rs docs/2026-07-31-transaction-spans-measurements.md
git commit -m "perf(l5): transaction-span cones walk interned ix, not cloned Strings"
```

The commit message MUST state the three discrimination results from Step 6 and the
`context.transaction_spans` span total before/after.

---

### Task 2 (ARCHIVED): Aggregate over `ResId` sets instead of per-routine `Vec<String>`

**Files:**
- Modify: `src/engine/l4/cone_derived.rs` (two borrowing accessors)
- Modify: `src/engine/l5/transaction_spans.rs` (`aggregate_span`, `span_roots_of`)
- Test: `src/engine/l4/cone_derived.rs`, `src/engine/l5/transaction_spans.rs`

**Interfaces:**
- Consumes: `SpanIndex` / `backward_cone_ix` from Task 1 (the cone is already a `Vec<u32>`).
- Produces:
  - `ConeDerivedStore::writes_table_ids_of(&self, routine_id: &str) -> &[ResId]`
  - `ConeDerivedStore::event_ids_of(&self, routine_id: &str) -> &[ResId]`
  - `fn aggregate_span_ix(cone: &[u32], index: &SpanIndex, summary_by_ix: &[Option<&FullRoutineSummary>], cone_derived: &ConeDerivedStore, writes_acc: &mut Vec<ResId>, events_acc: &mut Vec<ResId>) -> (Vec<String>, Vec<String>, bool)`

**The cost being removed:** `aggregate_span` calls `cone_derived.writes_tables_of(..)` and
`publishes_events_of(..)` ONCE PER VISITED ROUTINE. Each call resolves interned ids into a
freshly allocated `Vec<String>` (one `String` per id), sorts it, hands it over to be
inserted into a `BTreeSet<String>` (another allocation per element), and drops it. Over
`visited_total` (Task 0's census figure) that is the module's dominant allocation traffic.
Unioning the `ResId`s and resolving ONCE per template is exactly equivalent: the interner
is injective, so deduping ids equals deduping strings.

- [ ] **Step 1: Write the failing tests**

In `src/engine/l4/cone_derived.rs`'s `mod tests`:

```rust
    /// The borrowing accessors expose the SAME ids the materializing ones resolve —
    /// stated over a store with two routines so a swapped-window bug cannot pass.
    #[test]
    fn id_accessors_agree_with_the_string_accessors() {
        let store = test_store_two_routines();  // see helper below
        for rid in ["r/one", "r/two"] {
            let mut via_ids: Vec<String> = store
                .writes_table_ids_of(rid)
                .iter()
                .map(|id| store.resolve_res(*id).to_string())
                .collect();
            via_ids.sort();
            assert_eq!(via_ids, store.writes_tables_of(rid), "writes mismatch for {rid}");

            let mut ev: Vec<String> = store
                .event_ids_of(rid)
                .iter()
                .map(|id| store.resolve_res(*id).to_string())
                .collect();
            ev.sort();
            assert_eq!(ev, store.publishes_events_of(rid), "events mismatch for {rid}");
        }
    }
```

If `cone_derived.rs` has no two-routine test store helper, build one inline in the test
from `crate::engine::l5::test_support::cone_store_of` over two hand-written summaries —
one writing `t/B` and `t/A` and publishing `e/Z`, the other writing `t/A` and publishing
nothing. The two-routine shape is the point: a single-routine store passes even if the
accessor ignores the row and returns the whole pool.

In `src/engine/l5/transaction_spans.rs`'s `mod tests`:

```rust
    /// The union is over the WHOLE cone, sorted and deduped, and one routine's
    /// missing summary still poisons `coverage_complete` — hand-stated with a
    /// deliberate duplicate table across two routines so a missing dedupe shows.
    #[test]
    fn aggregate_unions_deduped_and_a_missing_summary_breaks_coverage() {
        let routines = vec![
            routine("root", "trigger"),
            routine("mid", "procedure"),
            op_commit_routine("committer", "procedure", &["c/op"]),
        ];
        let graph = graph_from_edges(
            &["root", "mid", "committer"],
            &[edge("root", "mid", "cs1"), edge("mid", "committer", "cs2")],
        );
        let reverse = build_reverse_call_graph(&graph);

        let mut summaries: HashMap<String, FullRoutineSummary> = HashMap::new();
        // BOTH write t/A; committer also writes t/B. `root` has NO summary at all.
        summaries.insert(
            "committer".to_string(),
            summary(
                "committer",
                vec![fact("insert", "table", Some("t/A")), fact("insert", "table", Some("t/B"))],
                vec![],
                Some(coverage("complete")),
            ),
        );
        summaries.insert(
            "mid".to_string(),
            summary(
                "mid",
                vec![fact("insert", "table", Some("t/A"))],
                vec![],
                Some(coverage("complete")),
            ),
        );

        let no_deps = BTreeSet::new();
        let spans = compute_transaction_spans(
            &routines,
            &no_deps,
            &reverse,
            &summaries,
            &cone_store_of(&summaries),
        );
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].writes_tables, vec!["t/A", "t/B"], "sorted + deduped union");
        assert!(
            !spans[0].coverage_complete,
            "`root` has no summary, so the span is not coverage-complete"
        );
    }
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p al-call-hierarchy --lib cone_derived > logs/t2-red-cone.log 2>&1
cargo test -p al-call-hierarchy --lib transaction_spans > logs/t2-red-tx.log 2>&1
grep -E "^error|cannot find" logs/t2-red-cone.log logs/t2-red-tx.log | head -5
```

Expected: FAIL — `no method named writes_table_ids_of` (the tx test may already pass on the
old code; that is fine — it is a REGRESSION pin for this task, and Step 6 proves it
discriminates).

- [ ] **Step 3: Add the borrowing accessors**

In `src/engine/l4/cone_derived.rs`, beside `writes_tables_of`:

```rust
    /// The routine's written-TableId window as interned ids, BORROWED — the
    /// allocation-free half of [`Self::writes_tables_of`]. The window is already
    /// sorted-and-deduped BY ID (`freeze_ids`), which is NOT the same order as the
    /// resolved strings, so a caller that needs string order must resolve and sort;
    /// callers that union across many routines (transaction spans) should union the
    /// ids and resolve ONCE.
    pub fn writes_table_ids_of(&self, routine_id: &str) -> &[ResId] {
        window(&self.writes_all_pool, &self.row(routine_id).table_writes_all)
    }

    /// The routine's published-EventId window as interned ids, BORROWED. Same
    /// contract as [`Self::writes_table_ids_of`].
    pub fn event_ids_of(&self, routine_id: &str) -> &[ResId] {
        window(&self.events_pool, &self.row(routine_id).event_publishes)
    }

    /// Resolve one interned resource id. Exposed so a caller that unions
    /// [`Self::writes_table_ids_of`] windows can materialize the result itself.
    pub fn resolve_res(&self, id: ResId) -> &str {
        self.interner.resolve(id)
    }
```

- [ ] **Step 4: Rewrite the aggregation**

Replace `aggregate_span` with:

```rust
/// Aggregate writes/events/coverage over a cone given as interned ixes.
///
/// Same values as the old `BTreeSet<String>` aggregation, at one resolve per DISTINCT
/// resource instead of one `String` allocation per (routine, resource) pair: the ids are
/// unioned as `ResId`s (the interner is injective, so id-dedupe IS string-dedupe) and
/// resolved once at the end. `writes_acc`/`events_acc` are caller-owned scratch reused
/// across templates.
fn aggregate_span_ix(
    cone: &[u32],
    index: &SpanIndex,
    summary_by_ix: &[Option<&FullRoutineSummary>],
    cone_derived: &ConeDerivedStore,
    writes_acc: &mut Vec<ResId>,
    events_acc: &mut Vec<ResId>,
) -> (Vec<String>, Vec<String>, bool) {
    writes_acc.clear();
    events_acc.clear();
    let mut coverage_complete = true;
    for &ix in cone {
        let Some(summary) = summary_by_ix[ix as usize] else {
            coverage_complete = false;
            continue;
        };
        writes_acc.extend_from_slice(cone_derived.writes_table_ids_of(&summary.routine_id));
        events_acc.extend_from_slice(cone_derived.event_ids_of(&summary.routine_id));
        if reachable_coverage(summary, None) != "complete" {
            coverage_complete = false;
        }
    }
    let _ = index; // ids are resolved through the store, not the span index
    (
        resolve_unique_sorted(writes_acc, cone_derived),
        resolve_unique_sorted(events_acc, cone_derived),
        coverage_complete,
    )
}

/// Dedupe interned ids, resolve them, and sort by the resolved STRING — the exact
/// output the old `BTreeSet<String>` produced. Sorting ids first makes the dedupe
/// linear; the final sort is by string because ix order is intern order, not
/// lexicographic (unlike `SpanIndex`, whose order is deliberately lexicographic).
fn resolve_unique_sorted(acc: &mut Vec<ResId>, cone_derived: &ConeDerivedStore) -> Vec<String> {
    acc.sort_unstable();
    acc.dedup();
    let mut out: Vec<String> = acc
        .iter()
        .map(|id| cone_derived.resolve_res(*id).to_string())
        .collect();
    out.sort();
    out
}
```

Replace `span_roots_of` with an ix version — a root is a routine with no callers, which
is now `index.callers_of_ix(ix).is_empty()`:

```rust
/// Span roots = cone routines with no reverse callers. `cone` is ascending-ix, i.e.
/// already lexicographic (see `SpanIndex`), so the filtered result needs no re-sort.
fn span_roots_of_ix(cone: &[u32], index: &SpanIndex) -> Vec<String> {
    cone.iter()
        .filter(|&&ix| index.callers_of_ix(ix).is_empty())
        .map(|&ix| index.id(ix).to_string())
        .collect()
}
```

In `span_template`, stop materializing the `BTreeSet<String>` entirely: build
`routines_in_span` as `cone_ix.iter().map(|&i| index.id(i).to_string()).collect()` (already
lexicographic), and call the two new functions. Build `summary_by_ix` once in
`compute_transaction_spans`:

```rust
    let summary_by_ix: Vec<Option<&FullRoutineSummary>> =
        (0..index.len()).map(|i| summaries.get(index.id(i as u32))).collect();
```

- [ ] **Step 5: Run the tests**

```bash
cargo test -p al-call-hierarchy --lib cone_derived > logs/t2-green-cone.log 2>&1
cargo test -p al-call-hierarchy --lib transaction_spans > logs/t2-green-tx.log 2>&1
grep -E "test result" logs/t2-green-cone.log logs/t2-green-tx.log
```

Expected: `ok` for both, every pre-existing test included.

- [ ] **Step 6: Prove discrimination**

1. Delete the `acc.dedup()` in `resolve_unique_sorted`. Expected:
   `aggregate_unions_deduped_and_a_missing_summary_breaks_coverage` FAILS (`t/A` twice).
2. Change the `else` arm for a missing summary from `coverage_complete = false; continue;`
   to just `continue;`. Expected: the same test FAILS on the coverage assertion.
3. In `writes_table_ids_of`, return the whole `&self.writes_all_pool` instead of the row
   window. Expected: `id_accessors_agree_with_the_string_accessors` FAILS.

Record all three PASS→FAIL→PASS outcomes.

- [ ] **Step 7: Byte-identity gate + measure + commit**

Same commands as Task 1 Step 7/8 with `t2` in place of `t1`. Then:

```bash
rustfmt src/engine/l4/cone_derived.rs src/engine/l5/transaction_spans.rs
git add src/engine/l4/cone_derived.rs src/engine/l5/transaction_spans.rs \
        docs/2026-07-31-transaction-spans-measurements.md
git commit -m "perf(l5): span aggregation unions interned ids, resolves once"
```

---

### Task 3 (ARCHIVED — superseded by Task R2): Share span payloads instead of cloning them per commit op

**Files:**
- Modify: `src/engine/l5/transaction_spans.rs` (`TransactionSpan` field types + emission)
- Modify: `src/engine/l5/detectors/d8.rs`, `d9.rs`, `d50.rs`, `d17.rs` (construction sites)
- Modify: `src/engine/l5/detector_context.rs` (any literal `TransactionSpan` in tests)

**Interfaces:**
- Produces: `TransactionSpan { routines_in_span: Arc<[String]>, writes_tables: Arc<[String]>, publishes_events: Arc<[String]>, span_roots: Arc<[String]>, .. }` — the scalar fields
  (`seed_kind`, `commit_operation_id`, `seed_callsite_id`, `commit_routine_id`,
  `coverage_complete`) are unchanged.

**The cost being removed:** every emitted span deep-clones all four payloads
(`t.routines_in_span.clone()` etc.) even though every span from one seed routine holds
IDENTICAL payloads — that is what `SpanTemplate` exists for. Task 0's `payload_strings`
census is the exact volume. `Arc<[String]>` makes the emission a refcount bump.

- [ ] **Step 1: Write the failing test**

In `src/engine/l5/transaction_spans.rs`'s `mod tests`:

```rust
    /// Two commit ops in ONE routine produce two spans that SHARE their payload
    /// allocations — the property `SpanTemplate` existed for but never delivered.
    /// Pinned with `Arc::ptr_eq`, which no golden can see.
    #[test]
    fn spans_of_one_seed_routine_share_payload_allocations() {
        let routines = vec![
            routine("root", "trigger"),
            op_commit_routine("committer", "procedure", &["c/op1", "c/op2"]),
        ];
        let graph = graph_from_edges(
            &["root", "committer"],
            &[edge("root", "committer", "cs1")],
        );
        let reverse = build_reverse_call_graph(&graph);
        let mut summaries: HashMap<String, FullRoutineSummary> = HashMap::new();
        summaries.insert(
            "committer".to_string(),
            summary(
                "committer",
                vec![fact("insert", "table", Some("t/A"))],
                vec![],
                Some(coverage("complete")),
            ),
        );
        summaries.insert(
            "root".to_string(),
            summary("root", vec![], vec![], Some(coverage("complete"))),
        );

        let no_deps = BTreeSet::new();
        let spans = compute_transaction_spans(
            &routines,
            &no_deps,
            &reverse,
            &summaries,
            &cone_store_of(&summaries),
        );
        assert_eq!(spans.len(), 2, "one span per commit op");
        assert_eq!(spans[0].routines_in_span, spans[1].routines_in_span);
        assert!(
            std::sync::Arc::ptr_eq(&spans[0].routines_in_span, &spans[1].routines_in_span),
            "same seed routine ⇒ ONE payload allocation, not a clone per op"
        );
        assert!(std::sync::Arc::ptr_eq(&spans[0].writes_tables, &spans[1].writes_tables));
        assert!(std::sync::Arc::ptr_eq(&spans[0].span_roots, &spans[1].span_roots));
    }
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p al-call-hierarchy --lib transaction_spans > logs/t3-red.log 2>&1
grep -E "^error|no method" logs/t3-red.log | head -5
```

Expected: FAIL — `Arc::ptr_eq` does not apply to `Vec<String>`.

- [ ] **Step 3: Change the types**

In `TransactionSpan`, change the four payload fields to `Arc<[String]>`; in `SpanTemplate`
likewise; in `span_template`'s cache-miss arm build them with
`let routines_in_span: Arc<[String]> = cone_ix.iter().map(|&i| index.id(i).to_string()).collect::<Vec<_>>().into();`
(and the same `.into()` for the other three). At both emission sites replace
`t.routines_in_span.clone()` with `Arc::clone(&t.routines_in_span)` (and the rest).
Add `use std::sync::Arc;` to the module's imports.

- [ ] **Step 4: Update the consumers**

Compile-driven; the known sites are:
- `d8.rs:163` and `d9.rs:101`: `id_list(span.writes_tables.clone())` → `id_list(span.writes_tables.iter().cloned())` if `id_list` takes an `IntoIterator<Item = String>`; check its signature in `src/engine/l5/finding.rs:307` and adapt (do NOT change `id_list` itself).
- `d9.rs:44/49/91/93`, `d50.rs:295`, `d8.rs:83`: `.len()`, `.iter()`, `.is_empty()` all work
  unchanged through `Arc<[String]>`'s deref to `[String]`.
- Literal `TransactionSpan { .. }` constructions in tests (`d50.rs:604-706`, `d17.rs:471`,
  `detector_context.rs`): wrap each payload in `.into()`, e.g.
  `routines_in_span: vec!["a".to_string()].into(),`.

```bash
cargo check --all-targets > logs/t3-check.log 2>&1; echo "EXIT=$?"
grep -E "^error" logs/t3-check.log | head -20
```

Iterate until clean. `--all-targets` is required: it covers test targets that `--bins`
misses (rust-analyzer's inline diagnostics are not a substitute — they go stale).

- [ ] **Step 5: Run the tests**

```bash
cargo test -p al-call-hierarchy --lib transaction_spans > logs/t3-green.log 2>&1
cargo test -p al-call-hierarchy --lib d8 > logs/t3-d8.log 2>&1
cargo test -p al-call-hierarchy --lib d9 > logs/t3-d9.log 2>&1
cargo test -p al-call-hierarchy --lib d50 > logs/t3-d50.log 2>&1
grep -E "test result" logs/t3-green.log logs/t3-d8.log logs/t3-d9.log logs/t3-d50.log
```

Expected: `ok` throughout.

- [ ] **Step 6: Prove discrimination**

In `span_template`'s emission, replace `Arc::clone(&t.routines_in_span)` with
`t.routines_in_span.iter().cloned().collect::<Vec<_>>().into()` (a fresh allocation with
equal contents). Expected: `spans_of_one_seed_routine_share_payload_allocations` FAILS on
the `ptr_eq` assertion while the equality assertion still PASSES — which is exactly the
point: the value test cannot see this, only the pointer test can. Revert; re-run; PASS.

- [ ] **Step 7: Byte-identity gate + measure + commit**

Same gate as Task 1 Step 7 with `t3`. Then:

```bash
rustfmt src/engine/l5/transaction_spans.rs src/engine/l5/detectors/d8.rs \
        src/engine/l5/detectors/d9.rs src/engine/l5/detectors/d50.rs
git add -u src/engine/l5 docs/2026-07-31-transaction-spans-measurements.md
git commit -m "perf(l5): span payloads are shared, not deep-cloned per commit op"
```

---

### Task 4 (ARCHIVED — superseded by Task R3): Capstone — re-measure, gate everything, record honestly

**Files:**
- Modify: `docs/2026-07-31-transaction-spans-measurements.md`
- Modify: `CHANGELOG.md`
- Modify: `docs/OUTSTANDING.md`

- [ ] **Step 1: Full-suite + clippy**

```bash
cargo test > logs/t4-tests.log 2>&1; echo "EXIT=$?"
grep -E "test result: FAILED|^error" logs/t4-tests.log | head
cargo clippy --all-targets --all-features > logs/t4-clippy.log 2>&1; echo "EXIT=$?"
grep -E "^error|^warning: unused" logs/t4-clippy.log | head
```

Expected: EXIT=0 for both.

- [ ] **Step 2: Golden gate, stated as zero movement**

```bash
bash scripts/check-goldens > logs/t4-goldens.log 2>&1; echo "EXIT=$?"
git status --short tests/ | tee logs/t4-tests-dirty.txt
```

Expected: EXIT=0 and `logs/t4-tests-dirty.txt` EMPTY. A moved golden here means the output
changed and the arc's core claim is false — investigate, never rebaseline.

- [ ] **Step 3: Both-corpus determinism**

```bash
./target/release-fast/alsem.exe analyze "$DO_WS" --format json --deterministic \
  > logs/txspan-final-do.json 2>/dev/null
sha256sum logs/txspan-final-do.json; cat logs/txspan-base-do.sha256
./target/release-fast/alsem.exe analyze "$CORPUS_8020" --format json --deterministic \
  > logs/txspan-final-8020.json 2>/dev/null
sha256sum logs/txspan-final-8020.json
# and the same on the Task-0 binary if it still exists, else state that 8020's
# baseline SHA was not captured and the DO SHA is the identity evidence
```

Expected: the DO SHA equals the Task-0 baseline exactly.

- [ ] **Step 4: Final measurement, with the noise caveat**

Run the 8020 trace three times (the ±80 s swing is on `analyze.total`; the SPAN total is
the claim). Record in the measurements doc: `context.transaction_spans` before/after for
each run, the median, the census figures from Task 0, and one explicit sentence naming
what is NOT claimed (e.g. any `analyze.total` movement beyond the span's own delta is
unattributed second-order effect, not this arc's).

- [ ] **Step 5: CHANGELOG**

Add an `### Performance — transaction spans (interned ix, shared payloads)` section under
`## [Unreleased]`, following the house style of the two sections above it: a probe-shape
statement ("two probe shapes, never compared"), a before/after table for the span total,
the census population, the byte-identity evidence (DO SHA + zero golden movement), and an
explicit "not claimed" paragraph.

- [ ] **Step 6: OUTSTANDING**

In the Wave-2/3 (Track B) entry, mark the `context.transaction_spans` work done with its
measured numbers, and re-rank what is now the largest span from the Task-4 trace (expect
`preflight.fresh_coverage` ~21.5 s and `context.capability_cones` ~21.2 s to become the
top two). Correct the stale d1 follow-up sizings if this arc's traces move them again.

- [ ] **Step 7: Commit**

```bash
git add CHANGELOG.md docs/OUTSTANDING.md docs/2026-07-31-transaction-spans-measurements.md
git commit -m "docs: transaction-spans arc capstone — measured, with what it does not claim"
```

---

## Self-Review

**Spec coverage.** The three cost centres named in the measurement (String-keyed BFS,
per-visited-routine `Vec<String>` materialization, per-op payload clone) map to Tasks 1, 2
and 3 respectively; Task 0 sizes them first so a wrong premise stops the plan rather than
being built on; Task 4 gates and records. The `MAX_DEPTH`, absorbing-committer and
missing-summary semantics each carry a named test.

**Placeholder scan.** Every step names exact files, exact commands and real code. The one
deliberate under-specification is Task 3 Step 4's `id_list` call shape, which depends on a
signature the implementer must read (`src/engine/l5/finding.rs:307`) — the step says so and
says not to change `id_list`.

**Type consistency.** `SpanIndex`/`backward_cone_ix` (Task 1) are consumed unchanged by
Task 2's `aggregate_span_ix`/`span_roots_of_ix`; `ResId` is `u32` from
`crate::engine::l4::cone_derived`; the `Arc<[String]>` field types introduced in Task 3
match the `.into()` construction shown in the same task.

**Risk noted:** `SpanIndex::build` allocates one `String` per universe id plus a
`HashMap<String, u32>` — on 8020 that is ~100 k entries, built ONCE, against the millions
of transient `String`s it removes. If Task 0's census shows `templates` is tiny (few
seeds), Tasks 1-3 are not worth their complexity; the plan says to re-scope in that case
instead of proceeding.
