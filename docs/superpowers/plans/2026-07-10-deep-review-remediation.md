# Deep-Review Findings & Remediation Roadmap (2026-07-10)

> **Status:** ROADMAP — findings inventory + tiered execution order. Each tier becomes its
> own spec/plan/SDD arc; this document is the source of record for what was found, what
> was verified, and in what order it gets fixed. Tasks here are scoped, not step-listed.

**Goal:** Close every confirmed defect from the 2026-07-09/10 deep review, in an order
that makes the measurement instruments honest FIRST, so every later fix is provable.

**Method (provenance of findings):** Two independent review passes.

1. **Night 1 (local workflow, 15 agents):** 5 dimensions — resolver core, test
   integrity, security, L5 determinism, panic robustness. Every non-minor finding
   adversarially re-verified by a second agent instructed to refute by default.
   9 confirmed, 1 refuted.
2. **Night 2 (cloud routine, fable fleet):** the 15 dimensions dropped from night 1 —
   architecture, receiver typing, arg dispatch, al-syntax crate, graph/indexer, LSP
   server, watcher/incremental, dependency ingest, concurrency, performance, golden
   hygiene, CLI surfaces, L4/detectors, docs drift, dead code. ~60 raw findings,
   12 refutation passes; one claim refuted outright (and replaced by a real bug found
   during refutation), several severities re-graded.

**Verification legend (per finding):**

- `VERIFIED` — decisive lines read by hand in this repo after the review; claim held verbatim.
- `FLEET-CONFIRMED` — survived the review's own adversarial refutation pass; not yet re-read by hand.
- Findings must be **re-verified at task-dispatch time** regardless of tag (standard
  pre-dispatch scouting); a tag here is provenance, not a substitute.

## Global constraints (binding, from CLAUDE.md + standing doctrine)

- An unprovable edge MUST become Unknown/Ambiguous — never a guessed `resolved`. Fail closed.
- al-sem is retired. Goldens are Rust-owned; regen via `REGEN_TEMP_GOLDENS=1 cargo test`.
  **A regen diff is a measurement, never an auto-bless** — read it before committing.
- Best solution, not simplest. Root causes, not symptoms. All downstream consumers are ours.
- CDO frozen-baseline gate: `cargo test --release --test program_resolve_harness -- --test-threads=1`
  with `CDO_WS` set; clippy bar `cargo clippy --release --all-features --all-targets -- -D warnings`.
- `rustfmt <file>` per-file, never `cargo fmt`. Stage named paths only. CHANGELOG per task.
- Folders named `.dependencies` are ordinary source folders. Never premise logic on the name.

---

## The meta-finding: the north-star zero is currently unfalsifiable

The 0.0000% real-unknown rate on CDO is not a lie, but it is not currently a
*measurement* either. Four independent mechanisms force it to read zero regardless of
engine correctness:

1. **Missed edges become `builtin`** — `Page.RunModal(Page::"X")` resolves from the
   PAGE_INSTANCE catalog (`member_catalog.rs:307`) as Exact/Complete instead of an
   entry-trigger Run edge; `builtin` is by definition not a hole. Invisible **by taxonomy**. `VERIFIED`
2. **Missed edges vanish entirely** — `abi_ingest.rs:442` drops every `local`/`internal`
   dependency routine; subscriber wiring then hits `0 => continue`
   (`index.rs:323`). The edge is neither Resolved nor Unknown nor Ambiguous — it does
   not exist. Invisible **by absence**. `VERIFIED`
3. **The ratchet never runs** — `program_resolve_harness.rs:1321` silently returns
   without `CDO_WS`; no workflow sets `CDO_WS` or `ENFORCE_CDO_WS`. `VERIFIED`
4. **The reporting command cannot fail** — `aldump --l3-call-graph-stats <bad path>`
   emits an all-zero histogram with `realUnknownRate: 0.0` and exits SUCCESS
   (`aldump.rs:856-874`). A jq-based CI ratchet passes forever. `VERIFIED`

Every fix below is ultimately in service of making that number falsifiable again.

---

## Tier 0 — Make the instruments honest

Nothing later can be trusted until these land, because these are how we know the later
fixes worked.

### T0.1 `aldump --l3-call-graph-stats` must fail on failure `VERIFIED`
`src/bin/aldump.rs:856-874`: fail-closed/empty layout → stderr warning +
`Histogram::default()` + `ExitCode::SUCCESS`; cross-app variant emits `{"error":...}`
with exit 0. Fix: distinct non-zero exit for "layout unusable"; `{"error":...}` must
exit non-zero. Audit all aldump modes for the same shape (other modes reportedly
already exit FAILURE — confirm and align).

### T0.2 Wire the CDO ratchet into machinery `VERIFIED`
`tests/program_resolve_harness.rs:1321` (Test 13) + bare-skip siblings at 866, 3089,
3393, 5243, `tests/program_graph.rs:5`, `tests/snapshot_robustness.rs:5`: route through
`cdo_ws_or_enforce()` (harness:2839) so `ENFORCE_CDO_WS=1` makes a lost workspace fail
loudly. Add an automated executor (scheduled self-hosted runner or release-gate script
exporting `CDO_WS` + `ENFORCE_CDO_WS=1`). Fleet count correction: 5 genuinely silent
CDO-gated tests, not 9 (two are `#[ignore]`d, two assert fixtures first).

### T0.3 NEW assertion: builtin-edge justification audit
The check that would have caught RunModal, and nothing like it exists. For every edge
classified `builtin` whose call site names a statically-known object (keyword receiver +
`DatabaseReference` arg, or object-typed receiver), assert the method is a genuine
platform intrinsic that does NOT dispatch into user code. Entry-dispatching intrinsics
(Run/RunModal family) must be flagged. Expected first catch: the RunModal population on
CDO. Design as a harness audit test (same family as `enforce_audit_ran`).

### T0.4 realUnknownRate split-brain: one semantic, one owner `FLEET-CONFIRMED`
L3 histogram and fresh-resolver histogram emit **different semantics under the same
`realUnknownRate` key** (L3 excludes memberNotFound/ambiguous; fresh engine counts
MemberNotFound as Unknown). CLAUDE.md points the north star at the L3 command while
graphify exports use the fresh resolver and L5 detectors use legacy L3 source-only
edges. Decide the single authoritative metric (fresh resolver), rename or delete the
other key, and re-point CLAUDE.md.

### T0.5 Performance targets: measure or delete `VERIFIED`
`benches/telemetry_hot_path.rs` registers only `bench_disabled`
(`criterion_group!(benches, bench_disabled)`); its own header promises an enabled-path
bound that does not exist; CI never runs bench. The four CLAUDE.md targets (index
100/1000 files, <1ms queries, <50ms updates) are asserted nowhere. Either build a bench
suite CI runs with bounds, or strike the table from CLAUDE.md. Doctrine: an unmeasured
target is a claim, not a target.

### T0.6 Golden regen completeness `FLEET-CONFIRMED`
≥5 golden families have NO regen path (157 R0 identity goldens `differential.rs:263`,
31 l3eg event-graph `:1930`, r3a2-trace, cli-c policy, cli-c cache), while
`tests/r0-goldens/README.md:43-51` documents a regen mechanism that was never written.
With al-sem retired there is no sanctioned rebaseline path for intentional improvements
in those surfaces. Add REGEN branches (assert path == regen path), fix the README.
Related: four manifest oracle files are read by no test (`fixtureCount: 157` is
decorative) — wire or delete. Also `REGEN_TEMP_GOLDENS` is presence-tested
(`is_ok()`), so `REGEN_TEMP_GOLDENS=0 cargo test` rewrites every golden — value-test it
everywhere in the same sweep.

---

## Tier 1 — Stop deleting edges (the moat)

### T1.1 L4 repeat-body CRITICAL + break/continue joins `VERIFIED` (C-1, H-9)
`src/engine/l4/cfg_walker.rs:583` `let body_node = node.children.as_ref().and_then(|c| c.first());`
— but L2 emits repeat bodies FLAT (`ir_walk.rs:1474-1476` `block_items`, vs `While`
wrapping in one block child at 1454-1456). Every other consumer walks all children
(`control_context.rs:327-344`, `return_summary.rs:223`, `dep_artifact_l4.rs:851`); the
contract is documented at `control_context.rs:320-321`. On the canonical
`repeat … until Next() = 0` loop, statements 2..n never update dataflow state —
detectors d39/d40/d42 verdicts wrong, and the wrong values are ALREADY BAKED into the
Rust-owned goldens (regen re-blesses them).

Companion (H-9, `FLEET-CONFIRMED`): `ir_walk.rs:1502` lowers `Break|Continue` to inert
`"other"`; cfg_walker joins exit states only for exit/error/try, so state threads
through statements a break skips — refuter walked a concrete while-loop to a converged
`dirty_at_exit = No` on a path that exits dirty (unsound direction). Currently masked
inside `repeat` by C-1; goes live the moment C-1 is fixed — **fix as one change**.

Fix: walk ALL children in the repeat arm (synthesize a block as control_context does);
dedicated CFN kinds for break/continue + per-loop break-state join (stopgap: saturate
loops containing break/continue). Then regen L4 goldens and **read the diff as the
blast-radius measurement** — every changed value is a previously-wrong shipped verdict.
Add multi-statement-repeat fixtures (corpus has none).

### T1.2 Dependency-ingest trio `H-1 VERIFIED, H-2/H-3 FLEET-CONFIRMED`
- **H-1 local/internal routine drop:** `abi_ingest.rs:442` — AL `local` on a publisher
  restricts raising, not subscribing; modern BaseApp integration events are
  `local procedure` (13,581 publisher attributes in BaseApp SymbolReference discarded).
  `inject_platform_event_publishers` (build.rs:232) excludes IntegrationEvent.
  Corollary: `is_internal` drop makes the InternalsVisibleTo friend map inert for
  symbol-only deps. Fix: ingest all routines WITH an access field; enforce visibility
  at resolution time; publisher-kind routines stay subscription-eligible; 0-candidate
  subscriptions become orphan diagnostics (not silent `continue`).
- **H-2 duplicate versions:** `build.rs:80-84` binds first GUID match from a
  version-LEXICOGRAPHIC sort (`dependencies.rs:346-348` — the "stable tiebreak" comment
  is false: it picks the winner); MinVersion never consulted on the GUID branch; a
  same-version file copy double-ingests → identical RoutineNodeIds →
  `sig_counts[""] >= 2` (param_sig_key hardcoded `""`, abi_ingest.rs:515) →
  `abi_overload_collapsed` poisons the ENTIRE app (build.rs:415-427). Fix: GUID-level
  dedup choosing highest MinVersion-compatible version (reuse `is_version_compatible`);
  diagnostic on conflict.
- **H-3 NUL-padded SymbolReference:** legacy path tolerates NUL padding
  (`app_package.rs:413-420`); engine path uses strict `serde_json::from_str`
  (`symbol_reference.rs:961`) → empty ABI with `error: Some(..)` that has ZERO
  production reads; I/O errors swallowed by `unwrap_or_else(default)`
  (abi_ingest.rs:203); `abi_check.rs:319` re-parses with the same strict parser
  (integrity harness synchronized-blind). Fix: shared tolerant first-value parse;
  propagate `abi.error` as a per-app ingest diagnostic.

### T1.3 RunModal entry-trigger dispatch `VERIFIED`
`src/program/resolve/extract.rs:371`: the sole `CalleeShape::ObjectRun` gate is
`kind_text == "keyword_identifier" && method_lc == "run"`. `Page.RunModal`/`Report.RunModal`
fall to the generic Member path → PAGE_INSTANCE/REPORT_INSTANCE catalog builtin
(member_catalog.rs:307/316) → the named target's OnOpenPage/OnPreReport subtree drops
out of the graph. The direct-call unit test (`object_run_page_resolves_to_onopenpage_not_onrun`)
bypasses the classifier. Fix: accept `runmodal` for Page/Report keyword receivers
(Codeunit has no RunModal), thread the same DatabaseReference target extraction, add an
END-TO-END fixture through `resolve_call_site_obligation`. Mirror check:
`engine/l2/ir_walk.rs:316-318` has the same `"run"`-only gate. T0.3's audit is the
regression guard.

### T1.4 al-syntax preproc + trivia lowering `FLEET-CONFIRMED` (H-6/H-7/H-8)
- `collect_routines` (lower/mod.rs:500) drops `preproc_split_procedure` entirely — no
  RoutineDecl, no SyntaxIssue, lost calls not even `unknown` (refuter verified
  empirically against the pinned grammar: the rule inlines headers with no inner
  `procedure` node).
- `lower_case_body` (lower/mod.rs:1100-1101) skips `preproc_conditional_case`/
  `preproc_split_case_extended`; `preproc_split_case_branch` yields a fabricated empty
  block with no issue (:1178-1181). `lower_stmt`'s `_` arm (:1078-1084) does not
  descend 8 statement-position preproc kinds (shared then-branch lost); `#region`
  markers become phantom Unknown statements.
- Trivia-as-named-extras: a comment as first child of a parenthesized expression
  replaces the real expression (:1226); a mid-argument comment becomes a phantom
  Unknown arg breaking arity-exact dispatch (:1211-1219); a comment inside
  `[EventSubscriber(...)]` args shifts positional indexing in `parse_event_subscriber_ir`
  (event.rs:46-66) → whole subscriber silently unregistered (found BY the refuter after
  refuting the original attribute-args claim).

Fix: treat split/guarded kinds as unioned wrappers (pattern already exists:
`PreprocSplitDeclaration` at object level); statement catch-all descends like the
expression one; filter Trivia-class kinds in ALL argument/child-list reads; add `#if`
fixtures — the corpus contains none, which is why this survived every prior review.

### T1.5 Receiver singleton-before-variable shadowing `FLEET-CONFIRMED` (H-4)
`receiver.rs:713-728` returns `Framework(Session/NavApp/…)` BEFORE Step 2's
`caller_scope_symbol` (:777): `var Session: Codeunit "Telemetry Wrapper";
Session.LogMessage(...)` → false builtin edge (SESSION catalog has `logmessage`);
`Session: Record Session` (virtual table 2000000009) → false Unknown. L3 sibling
deliberately does variables-first with a comment explaining exactly this
(`receiver_type.rs:283-286`). Fix: move the singleton match after Step 2
(`currpage`/`currreport`/`this` may stay early). CDO re-measure required.

### T1.6 Arg-dispatch text-inequality elimination `FLEET-CONFIRMED` (H-5)
`arg_dispatch.rs:1188-1192` treats normalized-text inequality as PROOF of
var-incompatibility, but `normalize_type_text` (sig_fp.rs:67-72) only collapses
whitespace runs (`Dictionary of [Integer,Text]` vs `… [Integer, Text]` "provably
incompatible") while `base_keyword` (:164-175) erases generic args (wrong-instantiation
by-value Dictionary is an "exact" match) — refuter walked `pick_candidate` to a
confident wrong pick on compiling AL. Related: member-field args hardcode
`var_passable: false` (:496-501) as an ELIMINATION, and the plan-doc trail shows the
premise was reversed without investigation. Fix: parsed type discriminators instead of
text equality; UNDECIDED (blocks the pick) instead of elimination when unprovable;
compiler-probe the field var-passability question.

### T1.7 Step 2b with-scope gate (night 1) `FLEET-CONFIRMED`
`receiver.rs:821-830`: dataitem-name arm types a bare identifier as Record with no
`WithState::NoWithProven` gate, and runs BEFORE gated Steps 3a/4b — the file's own
comment (:862) calls the ungated shape "a false Source edge, the cardinal sin". Fix:
mirror the 3a/4b gate; audit `resolve_report_implicit_rec_table`'s modify() fallback.

**Tier-1 exit gate:** CDO re-measure. The frozen-baseline SHA WILL change (T1.2/T1.3/T1.5
add real edges). That is the point: each delta must be attributed to a named fix, then
the baseline re-frozen. genuine_wrong stays 0 throughout.

---

## Tier 2 — Crash & DoS

### T2.1 Stack-overflow hardening everywhere the lowerer runs `FLEET-CONFIRMED` (H-14)
`snapshot/parse.rs:38-44` documents an OBSERVED overflow on BaseApp source; its 32 MiB
pool is the only `stack_size` in the repo. Unhardened same-lowerer sites: LSP indexer
(global rayon pool), didSave (main LSP thread, ~1 MiB on Windows), watcher thread, CLI
`--analyze`. Fix: one shared big-stack pool for ALL `al_syntax::parse` call sites.
Root-cause companion (night 1): depth-budget or work-stack in `lower_expr`
(lower/mod.rs:1185) so hostile input degrades to a SyntaxIssue instead of relying on
stack size; same for `walk_cfg` recursion (scc.rs already went iterative for this).

### T2.2 Zip/gzip decompression caps (night 1) `FLEET-CONFIRMED`
Unbounded `read_to_end` at `app_package.rs:403`, `abi_ingest.rs:225`,
`snapshot/embedded.rs:66`, `deps/app_package_zip.rs:71`, gunzip at
`snapshot_deserialize.rs:185-189`. Fix: `take(MAX_UNCOMPRESSED)` + hard parse error on
overflow, uniformly. (No zip-slip exists — extraction never writes entry-derived paths;
this is resource exhaustion only.)

### T2.3 Detector isolation contract `VERIFIED` (night 1)
`[profile.release] panic = "abort"` (Cargo.toml:174) makes `catch_unwind`
(registry.rs:270) inert in every shipped binary; the documented degrade-to-diagnostic
contract exists only under `cargo test`. Decision: detectors return `Result` (we own
all of them) and Err → detect-stage diagnostic. Add the missing panicking/failing
detector test. Fix the false comment at `finding.rs:268-270` (`map_table_id` runs
OUTSIDE the isolation).

### T2.4 Small panics + release-inert guards `VERIFIED` (night 1)
- `diff_parser.rs:58` `unquote_path` lone-quote slice panic (`--- "` in a truncated
  diff) — guard `len < 2`, degrade to DiffParseError.
- `cbor.rs:83` map-16 `debug_assert` compiles out of release while the comment says
  "an invariant we ENFORCE" — make it a real error path.
- `strip_trailing_temporary` byte-slice with `to_lowercase()` length
  (receiver.rs:2998, duplicated at l3/record_types.rs:60) — Unicode `ẞ` panic/abort;
  fix both copies, then dedup.

---

## Tier 3 — The legacy LSP pipeline: migrate, don't patch

H-10..H-13 (`H-10 VERIFIED`, rest `FLEET-CONFIRMED`) are all one engine:
`graph.rs`/`indexer.rs`/`protocol.rs`/`server.rs`/`handlers.rs`.

- **H-10:** `graph.rs:768-774` `remove_file` deletes entire `incoming_calls` entries
  (containing OTHER files' live call sites); re-add re-links only the saved file's own
  calls; no repair pass. One whitespace save permanently destroys cross-file incoming
  edges.
- **H-11:** the whole pipeline interns case-sensitively (`graph.rs:441-443`) in a
  case-insensitive language — while `DependencyKey` in the SAME FILE lowercases
  "because AL is case-insensitive".
- **H-12:** UTF-8 byte columns served as (default) UTF-16 — `position_encoding` never
  set (verified: zero hits in src/), no converter on the LSP path, inbound comparisons
  mix encodings (graph.rs:722-725). Every range on a line containing æøå/emoji is wrong.
- **H-13:** `path_to_uri` (protocol.rs:60-83) hand-encodes 5 chars; fluent-uri rejects
  raw non-ASCII → every URI under e.g. `/home/user/Løsninger/` becomes
  `file:///unknown`.
- Plus the watcher/diagnostics mediums (published once at startup, never refreshed;
  inotify overflow dropped; deps frozen at startup; no debounce; reindex holds both
  locks while parsing).

These are not independent bugs; they are one superseded engine. **Decision (per
doctrine — best solution, unreleased, all consumers ours): migrate the LSP surface onto
`src/program/resolve` + al-syntax IR and delete the legacy graph/indexer/parser path.**
This tier needs its own brainstorm → spec → plan cycle (scope: which LSP features map
onto the program engine, incremental-update story, position-encoding negotiation,
percent-encoded URIs via the already-present `percent_encoding` dep). Fixing H-10/H-11
in place is the fallback only if migration surfaces a blocker; patching UTF-16 and URI
handling is required in EITHER path (they live in protocol/server, not the graph).

---

## Tier 4 — Cheap, high-leverage hygiene

- **T4.1 CLAUDE.md/README rewrite `VERIFIED` (H-18):** "Adding New AL Constructs" points
  at retired `language.rs` queries; Key Modules names a nonexistent top-level
  `resolver.rs` and omits `src/engine`/`src/program`/`crates/al-syntax`; v3 section
  instructs using deleted `node_util::block_statements`; README + CLAUDE.md both show
  the nonexistent `--no-lsp` flag (clap hard-errors). Rewrite against the real tree.
- **T4.2 `fingerprint_query.rs:747` HashMap-order candidates (night 1):** build from
  sorted `stable_ids` like `digest_cli.rs::build_selector_indexes`, then dedup the two
  implementations.
- **T4.3 CLI flag honesty `FLEET-CONFIRMED`:** aldump mutual-exclusion array missing 3
  mode flags; `alsem prove --no-roots-config`/`--alpackages` + `fingerprint
  --no-roots-config` parsed-never-read (fingerprint even emits `"rootsConfigIgnored":
  false` against the flag); `--lsp` never read; `--analyze` always exits 0.
- **T4.4 r3b sweep hard-require (night 1):** `r3b_incremental_nondeterminism.rs:75`
  `if let Ok(rd)` → `.expect` like its siblings; floors above the 6 hardcoded fixtures.
- **T4.5 L4 diagnostics honesty `FLEET-CONFIRMED`:** fixpoint cap-hit (1000 iters,
  summary_runner.rs:1087) ships partial summaries as definite facts with stderr-only
  notice (the transfer function is non-monotone via apply_call — the cap is
  load-bearing); d44/d45 discard their computed truncation counts. Surface both as
  Uncertainty/diagnostics.
- **T4.6 Mediums appendix sweep:** app.json single-reader with BOM strip (~8 divergent
  readers today; serde_json rejects BOM — empirically verified by the fleet);
  `EnumExtensionTypes` missing from the engine BARE table; `this.X` local-hijack
  (receiver_type.rs:187); ASCII-only case-fold vs compiler Unicode fold (index.rs:215 —
  `Løbenr`/`LØBENR` false real-unknown); preproc duplicate-var first-match-wins
  (receiver.rs:2964); dead-code allows hiding rot (`VariableBinding`, telemetry dedup,
  policy-loader warnings). Triage each: fix or file with a wake condition.

---

## Explicitly refuted claims (do not re-litigate)

1. Attribute-args comment shifting `publisher_include_sender` — refuted (kind gate);
   replaced by the REAL EventSubscriber positional bug (in T1.4).
2. `fingerprint_query.rs:531` non-total comparator — abstractly true, unreachable
   through any real fact constructor (night 1 refuter traced all of them).
3. model_instance_id doc-drift; app_package "truncation" framing; "nine silent CDO
   tests" (five); "contradictory metrics in one run" (split-brain is real but
   cross-invocation) — all corrected as noted inline above.

## Execution protocol

- Order: **T0 → T1 → T2 → T3 → T4.** T0 and T2 are independent enough to interleave if
  a T1 arc blocks; T3 must not start before T1 lands (migration wants the program
  engine's edge semantics settled).
- Each tier = its own SDD arc (subagent-driven, task briefs, per-task review, whole-branch
  review, 4-option merge menu). No merge to master without explicit user request.
- Every task re-verifies its finding's decisive lines before implementing
  (pre-dispatch scouting) — provenance tags above are not proof.
- CDO gates: byte-identical baseline through T0/T2/T4 (instrument + robustness changes
  must not move resolution); T1 EXPECTS deltas — each attributed, then re-frozen;
  genuine_wrong == 0 is the invariant that never moves.
- Golden regens in T1.1: the diff IS the deliverable's measurement — reviewed
  line-by-line, never blind-committed.
