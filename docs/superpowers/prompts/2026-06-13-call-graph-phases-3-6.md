Drive the AL call-graph resolution engine (alch-engine, Rust, BC/AL static analysis) toward near-zero real-`unknown`, autonomously to the end. Use superpowers: brainstorm only if needed, then `superpowers:writing-plans` to write the plan, then `superpowers:subagent-driven-development` to execute task-by-task with TDD; use `superpowers:systematic-debugging` for any failure (root cause before fixes). Do not stop for check-ins. Valid stops: BLOCKED you cannot resolve, a genuine product-behavior fork, or all work below complete + full `cargo test` green.

## Mandate (non-negotiable framing)
- Pursue the BEST solution, not the simplest/easiest/quickest. Time is NOT a constraint; refactoring is always on the table; the project is unreleased.
- The al-sem TypeScript reference is RETIRED. The engine is Rust-owned: tests assert Rust-owned baselines (regen via `REGEN_TEMP_GOLDENS=1 cargo test`) + structural CONTRACT oracles, NEVER al-sem byte-parity. We control ALL downstream consumers (CLI formats, snapshots, fingerprints, SARIF, digests, prove/diff) — change any output shape when it improves the product, updating consumers + goldens together.
- The product moat is PRECISE whole-program call-graph resolution. North-star metric = real-`unknown` edge rate on real BC apps; drive toward zero where the residual is provably dynamic.
- The engine NEVER panics — every path fails closed to a conservative default (`unknown`), never aborts the LSP shipping path. Additive to `src/engine/*`.

## READ FIRST (source of truth)
1. `docs/superpowers/specs/2026-06-13-call-graph-resolution-redesign.md` — the redesign spec (§9 build sequence, §7 FP risk).
2. `CLAUDE.md` — "Project Direction & The Moat", "Testing Philosophy & Goldens", "Working Principle".
3. Auto-memory `MEMORY.md` (best-solution-mantra, al-sem-retired-rust-owned, call-graph-resolution-redesign).

## World state (already shipped to origin/master, do NOT redo)
- Phases 1+2 done: honest taxonomy (`src/engine/l3/resolution_class.rs`), intrinsic builtin catalog (`src/engine/l3/member_builtins.rs`, phf) for Record/RecordRef/FieldRef/KeyRef + framework types applied on the MEMBER path in `src/engine/l3/call_resolver.rs` before `unknown`; contract oracles in `tests/l3cg_oracles.rs`.
- d22 implicit-Rec temp-state root-cause fix in `src/engine/l2/body_walk.rs` (flows `source_temp_state` for the implicit `Rec` argument binding).
- al-sem byte-parity retired; all downstream differential tests migrated to Rust-owned in-repo goldens (r4f, r3a1, cli-b digest/fingerprint/prove/snapshot read `tests/r0-corpus` fixtures + `tests/cli-b-goldens/`; gate preflight oracles re-pointed to `ws-txn-d47-pos-file`).
- Diagnostics: `aldump --l3-call-graph-stats <ws>` (honest-taxonomy histogram + realUnknownRate) and `aldump --l3-unknown-breakdown <ws>` (attributes every `unknown` edge to an `UnknownReason`). `UnknownReason` enum lives in `call_resolver.rs`.
- Full `cargo test` is green at master. Branch `engine-d22` == master == origin/master.

## Measured starting point — CDO app `U:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud`
`aldump --l3-call-graph-stats`: 13971 edges, builtin 3639, unknown 3295, realUnknownRate 23.6% (was 42.2% before phases 1-2), ZERO unresolved→resolved regressions.
`aldump --l3-unknown-breakdown` attributes the 3295 residual unknowns (100% attributed):
- bare-unresolved 1247 — bare `Foo()` not in own object, not in the ~25-entry global-builtin allowlist (`src/engine/l3/al_builtins.rs`). AL forbids unqualified cross-object calls → these are overwhelmingly missing PLATFORM GLOBAL functions (GuiAllowed, CreateInStream, Database::, StrSubstNo overloads, etc.).
- untracked-receiver 881 — member `x.M()` where `x` is not a captured local/param/global routine variable (object-level globals, CurrPage/CurrReport, return-value chains).
- record-table-procedure 812 — `Rec.SomeProc()`, a real user table procedure (resolvable via `symbol_table.rs::routines_in_object(tableObj)`).
- compound-receiver 243 — `a.b.M()` chained/expression receiver (`receiver.rs::simple_receiver_name` declines).
- non-object-receiver-type 70 — receiver type is Variant/primitive/unrecognized (the dynamic floor).
- framework-method-not-in-catalog 39 — framework methods missing from the phase-2 catalog.
- interface-no-impl 2, enum-static 1.

## Work items (ordered; this order is deliberate — keep it)
Reviewed + endorsed by an external model (Gemini 3.1 Pro). Sequence cheap reclassification (no new resolved edges) BEFORE graph expansion (Phase 3), so the metric is baselined cleanly and the FP wave is isolated.

1. **Enum refactor (do FIRST).** Replace the stringly-typed `CallEdge.dispatch_kind: String` and `resolution: String` (TS hangover) with strict Rust enums `DispatchKind` / `Resolution`. FOLD the existing `UnknownReason` into the taxonomy as `Resolution::Unknown(UnknownReason)` so every `unknown` edge has a compiler-enforced cause ("unattributed" becomes structurally impossible). Keep the projection boundary (`call_graph_projection.rs::project_edge`, `coverage.rs`, `snapshot.rs::map_dispatch_kind`, the differential `countCoverage`) emitting the SAME golden strings via enum→str — internal-only refactor, goldens stay byte-stable. Update every match site. Verify the full suite stays green with NO golden changes.

2. **Generated bare-global catalog (kills ~1247).** A hand-maintained list of hundreds of AL platform globals will drift per BC version — wrong architecture. Build an OFFLINE generator (a checked-in script/`build.rs`-adjacent tool, NOT runtime) that emits a `phf_set!`/`phf_map!` of AL compiler-intrinsic GLOBAL functions into `al_builtins.rs` (or a new `global_builtins.rs`), from an authoritative source: prefer dumping `Microsoft.Dynamics.Nav.CodeAnalysis.dll` symbols if available on the machine; else scrape the MS Learn AL method reference. Check in the generated file + the generator + a provenance note (BC version). Apply on the BARE path (`call_resolver.rs` `PCallee::Bare` `NotFound` branch) — pure reclassification, NO new resolved-to-routine edges. Also reconsider regenerating `member_builtins.rs` from the same generator (it was hand-built) so both catalogs share one source of truth.

3. **Framework catalog gaps (~39).** Extend `member_builtins.rs`; extend `--l3-unknown-breakdown` if needed to NAME the missing `(kind, method)` pairs so the gap list is concrete.

4. **Re-measure CDO** (`--l3-call-graph-stats` + `--l3-unknown-breakdown`) and confirm: unknown dropped by ~1286, realUnknownRate down, ZERO new resolved edges, no detector FP regressions. Commit this as the clean reclassification baseline.

5. **Phase 3 — Record table-procedure dispatch (~812; first NEW resolved edges).** Implement the spec's ReceiverType lattice + Phase-A/B typed dispatch: route the EXISTING object path through it first (goldens stable — proves the refactor sound), then add Record-receiver dispatch — a Record method NOT in the builtin catalog resolves via `routines_in_object(tableObj)` using the receiver's effective table (reuse the d22 `record_types` effective-own-table logic). PAIR with a CDO re-triage (spec §7): every phase that lands new edges is followed by a detector-precision check to catch latent transitive FPs before they erode trust.

6. **Phase 4 — receiver-type scope + globals + return-type + chained inference (untracked 881 + compound 243).** Persist the receiver-type environment L2→L3 (§3): WITH-receiver + implicit-`Rec`; consult Locals→Params→Globals→implicit-Rec→enclosing-WITH. Add object-global var resolution, `CurrPage`/`CurrReport` host typing, method/return-type inference for chained `a.b.M()` and `GetX().M()` (cap recursion ~3). Re-triage CDO.

7. **Phase 5 — intra-procedural TableID/Enum const-prop (dynamic→static).** Cheap L3 const tracker: `MyId := Database::Customer` → `RecordRef.Open(MyId)`/`GetTable(rec)` flows a static `Record{table_id}` (dynamic→resolved); cross-procedural/DB-derived stays honestly `dynamic` (NOT `unknown`). Reclassify the ~70 non-object/Variant floor as `dynamic` where genuinely runtime-typed. No L4→L3 feedback (keep layers acyclic).

8. **Phase 6 — guard-predicate edge annotations + L5 guard-intersection suppression (§7).** At L2/L3 tag each call/op edge with a minimal high-impact guard set: `GuiAllowed`, `IsTemporary`, `HasFilter` (extend as needed). CRITICAL refinement: walk CFG BLOCK DOMINANCE (not just AST nesting) and carry POLARITY — AL leans on early-exit guards (`if not GuiAllowed then exit;` then the call). Tag `GuiAllowed:true` vs `GuiAllowed:false`. Cone stays flow-insensitive (fast set-union). At L5, detectors intersect guard tags along the reachability path and discard findings whose path requires a guard incompatible with the root context (e.g. requires `GuiAllowed:true` but crosses an edge tagged `:false`, or a background-session root). Pair with CDO re-triage.

## Validation (prove it, don't assert)
- Measure CDO before/after EVERY phase with `aldump --l3-call-graph-stats` and `--l3-unknown-breakdown`. Report the reclassification deltas + the new real-`unknown` rate + the breakdown shift. Build release binaries: `cargo build --release --bin aldump` (+ `alsem`).
- Extend CONTRACT oracles in `tests/l3cg_oracles.rs`: every `resolved` edge's `to` exists in the symbol table; every `builtin` method is in a catalog; every `unknown` has no inferable receiver type AND a concrete `UnknownReason`; `dynamic` only where genuinely runtime-typed; no edge is both `builtin` and `resolved`.
- TDD: inline-AL fixture tests (mirror `tests/l3cg_member_builtins.rs` / `tests/gap_audit_*.rs`: `assemble_and_resolve_default` → walk the projected graph) for each new resolution path.
- Detector precision: per-detector TP% on CDO must NOT regress as edges are added (the §7 guard work is the lever). Use the `triage-findings` skill against `alsem analyze` output when re-triaging.
- Rebaseline moved Rust-owned goldens via `REGEN_TEMP_GOLDENS=1`; inspect every diff is intended (CRLF/EOL churn that normalizes on `git add` is noise — confirm with `git diff --numstat` showing 0/0). Update manifest "matrix" oracles to the current Rust totals. Full `cargo test` must end green.

## House rules (hard constraints)
- `rustfmt <file>` per file — NEVER `cargo fmt`. Stage only intended paths — NEVER `git add -A`.
- Goldens are Rust-owned; regen ONLY via `REGEN_TEMP_GOLDENS=1`. `KNOWN_DIVERGENCES.json` stays `[]`. NEVER read goldens from or write into `U:\Git\al-sem` (frozen, read-only) at test time; vendoring an input artifact once into the repo is fine.
- Commit in logical groups with clear messages; verify (`cargo build --lib` + targeted tests) WITHIN the turn before any long full-suite run. Work on a feature branch off master (or continue `engine-d22`). Do NOT push or merge to master until full `cargo test` is green AND you have secret-scanned the diff; then the established workflow is fast-forward `master` to the branch in the main worktree (`git -C U:\Git\al-call-hierarchy merge --ff-only <branch>`) and `git -C U:\Git\al-call-hierarchy push origin master`.
- Git bash + Windows paths; no `2>nul` (creates undeletable files). Use `rg`/dedicated tools over shell `grep`/`find`.
- DISK on U: is tight and behaves oddly (NTFS dedup/VSS: deleting files may NOT immediately reclaim free space; verify with PowerShell `Get-PSDrive U`). `cargo clean` between heavy build cycles; if a build dies with LNK/disk errors, `cargo clean` + retry. A full `cargo test` debug build is ~30 GB — keep headroom.
- Update `CHANGELOG.md` (Keep a Changelog) after each feature/fix.

## Done
All 8 work items implemented; CDO real-`unknown` driven down to its provably-dynamic residual with per-phase before/after measurements reported; contract oracles extended; detector precision non-regressed (CDO re-triaged per phase); Rust-owned goldens rebaselined; full `cargo test` green; committed in logical groups; then (only after green + secret scan) fast-forward + push to origin/master.
