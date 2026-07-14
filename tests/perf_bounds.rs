//! CI perf-bounds gate (T3 LSP-migration arc, Task 16). Asserts the
//! ENGINE-BACKED LSP surface (`LspSnapshot` / `lsp::handlers` /
//! `lsp::updater` — the surface Task 15 cut the server over to) stays within
//! generous (~3x) margins of the CLAUDE.md Performance Targets table, so an
//! order-of-magnitude regression fails CI loudly. This file previously
//! pinned the LEGACY `Indexer`/`handlers`/`graph` pipeline (T0.5); that
//! pipeline is deleted in Task 17, so this rewrite is the last thing
//! standing in its way. See `benches/lsp_pipeline.rs` for the finer-grained
//! Criterion measurements this gate is a coarse tripwire for.
//!
//! Compiled for real ONLY under `#[cfg(not(debug_assertions))]`: a
//! debug-build timing assert is meaningless (unoptimized code can run several
//! times slower than release, for reasons unrelated to any real regression),
//! so the actual bounds checks below only exist in a release build. This is
//! deliberate, NOT a silent-skip: CI explicitly invokes
//! `cargo test --release --test perf_bounds`, so it always runs in the
//! profile where the checks are compiled in. The always-present marker test
//! below has no `cfg` gate at all, so `cargo test --test perf_bounds` (any
//! profile) never silently reports zero tests — if this file's `mod` wiring
//! ever broke, this test failing to even show up would be caught immediately
//! by the "did the binary run any tests" question, not silently pass.
//!
//! Bounds are 3x each CLAUDE.md target (the same USER DECISION binding
//! convention this file has used since T0.5 — generous by design so
//! occasional flake on a loaded CI runner doesn't cause false failures,
//! while a true order-of-magnitude regression still trips the gate). The two
//! incremental-update bounds (rung 1 / rung 2) are NEW to this rewrite —
//! their absolute targets (100ms / ~1.5s) come from the T3 Task 9 Step-3b
//! RE-MEASUREMENT against the real `Updater` code path on the real CDO
//! workspace (`.superpowers/sdd/t3-stage-split.md`'s addendum), which
//! REPLACES Task 3's earlier ~1.9s algebraic upper-bound estimate for rung 2
//! (that estimate predated `Updater`/`assemble_program_graph` entirely —
//! see the addendum for why the real number came in lower).
//!
//! # Rung rows carry TWO bounds each (t3.16 review fix-wave)
//!
//! Applied against the much smaller SYNTHETIC 1000-file corpus (no
//! dependencies), the CDO-scale absolute bounds above carry 15-30x headroom
//! by construction (measured ~20ms/~150ms on this corpus vs. 300ms/4.5s
//! bounds) — generous for CDO-scale behavior, but loose enough that a
//! genuine 10x regression ON THIS CORPUS ALONE would sail through
//! unnoticed, silently breaking this file's own "3x margin, but a real
//! order-of-magnitude regression still trips it" promise for these two rows
//! specifically (every OTHER row's bound is already sized relative to what
//! actually runs on this corpus). `RUNG1_SYNTHETIC_BOUND`/
//! `RUNG2_SYNTHETIC_BOUND` close that gap: 5x today's measured
//! synthetic-corpus baseline, asserted IN ADDITION TO (never instead of) the
//! CDO-anchored bound. Two independent regression classes are guarded: the
//! absolute bound catches CDO-scale behavior regressing below what was
//! measured there; the synthetic bound catches THIS corpus's own
//! performance regressing an order of magnitude, which the absolute bound's
//! necessary headroom cannot see.
//!
//! # The corpus's hub-call must go through a declared variable
//!
//! `tests/perf_support`'s cross-file "hub" call is a `Hub: Codeunit "..."`
//! declared-variable call, never a bare `HubObjectName.Proc0()` — real AL
//! has no syntax for invoking another object's procedure by its bare display
//! name with no declared receiver. The LEGACY pipeline's naive
//! text-matching call resolution (`callee_object` is whatever raw text sits
//! left of the dot, matched directly against object display names when no
//! variable binding exists for it — `src/indexer.rs`) tolerated the bare
//! form; the fresh program-engine resolver this file now measures does not
//! (confirmed empirically against the real resolver: a bare object-name
//! "call" classifies as `Unknown`/`UntrackedReceiver`, 0% resolved). This
//! was fixed at the SOURCE (`tests/perf_support/mod.rs`) rather than papered
//! over here — a 0-fan-in corpus would make this file's whole "real
//! hash-map fan-out, not a degenerate corpus" premise false. See that
//! module's doc for the full explanation.

#[cfg(not(debug_assertions))]
#[path = "perf_support/mod.rs"]
mod perf_support;

/// Always present regardless of build profile — guarantees `cargo test
/// --test perf_bounds` never silently reports 0 tests even if the
/// release-only module below fails to compile in.
#[test]
fn perf_bounds_binary_is_never_empty() {}

#[cfg(debug_assertions)]
#[allow(dead_code)]
/// Compile-time note (not a test): the real bounds checks live in
/// `release_checks` below and only compile under `#[cfg(not(debug_assertions))]`.
/// Run `cargo test --release --test perf_bounds` to exercise them.
const DEBUG_BUILD_SKIPS_REAL_PERF_BOUNDS: &str = "see module doc comment";

#[cfg(not(debug_assertions))]
mod release_checks {
    use super::perf_support;
    use al_call_hierarchy::config::DiagnosticConfig;
    use al_call_hierarchy::lsp::diagnostics::compute_all;
    use al_call_hierarchy::lsp::encoding::PositionEncoding;
    use al_call_hierarchy::lsp::handlers::{self, ItemData};
    use al_call_hierarchy::lsp::snapshot::LspSnapshot;
    use al_call_hierarchy::lsp::updater::{ChangeEvent, Rung, Rung1Context, Updater};
    use al_call_hierarchy::protocol::path_to_uri;
    use al_call_hierarchy::snapshot::ParsedUnit;
    use std::path::Path;
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    // 3x the CLAUDE.md target, per the binding T0.5 USER DECISION (see module doc).
    const BUILD_FULL_100_BOUND: Duration = Duration::from_millis(1500); // target: 500ms
    const BUILD_FULL_1000_BOUND: Duration = Duration::from_millis(6000); // target: 2s
    const QUERY_BOUND: Duration = Duration::from_millis(3); // target: 1ms
    // `incoming`'s OWN bound, under this corpus's deliberate 999-way real
    // fan-in — NOT the shared `QUERY_BOUND` above. Rule 1 of the live-span
    // audit (`src/lsp/handlers.rs`'s module doc) means every distinct
    // caller's position is re-derived LIVE from that caller's OWN current
    // file text (a fresh `LineTable` built per caller) rather than served
    // from a precomputed byte range the way legacy's `graph.rs` did — a
    // deliberate correctness trade (never serve a stale witness span), not a
    // regression. That makes `incoming` on a 999-way-fan-in target O(distinct
    // callers), each paying a real per-file text scan, structurally unlike
    // `prepare`/`outgoing` (whose cost doesn't scale with fan-in) — so the
    // legacy <1ms target this constant used to inherit no longer describes
    // reality once EVERY caller's span is re-derived instead of stored.
    // Measured median 20.3ms on this machine (see the task report) — target
    // set to 25ms (an editor-imperceptible latency for a "who calls this"
    // panel) with the SAME 3x-bound convention as everywhere else in this file.
    const INCOMING_HIGH_FANIN_BOUND: Duration = Duration::from_millis(75); // target: ~25ms
    // T3 Task 9 Step-3b CDO re-measurement (~10.5ms warm-context) vs. the
    // 100ms target — see module doc.
    const RUNG1_BOUND: Duration = Duration::from_millis(300); // target: 100ms
    // T3 Task 9 Step-3b CDO re-measurement (~1.464s) vs. Task 3's superseded
    // ~1.9s algebraic upper-bound estimate — see module doc.
    const RUNG2_BOUND: Duration = Duration::from_millis(4500); // target: ~1.5s

    // CORPUS-RELATIVE bounds (t3.16 review fix-wave), asserted IN ADDITION TO
    // the CDO-anchored absolute bounds above, not instead of them. The
    // absolute bounds carry 15-30x headroom against THIS SYNTHETIC corpus's
    // own measured cost (~20ms/~150ms vs. 300ms/4.5s — see the task report's
    // measured-numbers table), because their targets come from the real CDO
    // workspace, a much larger scale. That headroom means a genuine 10x
    // regression on THIS corpus alone (e.g. ~20ms -> ~200ms) would sail
    // through the absolute bound unnoticed, silently breaking this file's
    // own "3x margin, but a real order-of-magnitude regression still trips
    // it" promise for the two rung rows specifically (every other row's
    // bound is already sized relative to what runs ON this corpus). These
    // two constants close that gap: 5x today's measured synthetic-corpus
    // baseline. The absolute bound guards CDO-scale behavior from
    // regressing below what was measured there; this one guards THIS
    // corpus's OWN performance from regressing an order of magnitude, which
    // the absolute bound alone cannot catch at synthetic-corpus scale.
    const RUNG1_SYNTHETIC_BOUND: Duration = Duration::from_millis(100); // 5x ~20ms measured baseline
    const RUNG2_SYNTHETIC_BOUND: Duration = Duration::from_millis(750); // 5x ~150ms measured baseline

    // The PRODUCTION scoped-context path (F6): `Rung1Context` built ONCE,
    // reused across every sample — exactly `spawn_updater`'s hot loop, unlike
    // `RUNG1_SYNTHETIC_BOUND`'s `apply_batch` path above (which rebuilds the
    // context every call, a cost production never pays per keystroke-save).
    // Measured ~12.9ms median on this corpus (release, dev machine,
    // 2026-07-14) — 5x that baseline, per this file's own convention.
    const RUNG1_SCOPED_SYNTHETIC_BOUND: Duration = Duration::from_millis(65); // 5x ~12.9ms measured baseline

    // `compute_all` (diagnostics recompute — t3 whole-branch review, blocker
    // fix): runs on EVERY snapshot swap, including a rung-1 single-file body
    // edit (`server.rs`'s `on_swap`), which is exactly what made a pre-fix
    // O(decls * event_edges) scan inside `effective_incoming_count`
    // (`src/lsp/lens.rs`) a real quadratic blocker — see
    // `LspSnapshot::publisher_fanout`'s doc for the fix. This is a GENUINELY
    // NEW gate (no prior CDO re-measurement exists for it), so
    // `COMPUTE_ALL_BOUND`'s target is a deliberately chosen, reasoned
    // ARCHITECTURAL one rather than a CDO-anchored one: a diagnostics
    // recompute should complete in a small fraction of rung-1's own 100ms
    // CDO-anchored budget, since it runs ON TOP of that cost after every
    // swap.
    //
    // `COMPUTE_ALL_SYNTHETIC_BOUND` was ORIGINALLY 30ms (5x the ~5.4ms
    // measured baseline) — the t3 final review PROVED this margin marginal:
    // reverting the fix and re-measuring the buggy 1000-file cost gave a
    // median of ~31.04ms, but samples ranged 29.13-32.46ms — 2 of 5 buggy
    // samples were BELOW the 30ms bound. Run-to-run noise (~±5%) exceeds a
    // 3% margin (31.9ms buggy vs. 30ms bound), so a 7%-faster machine would
    // let the exact quadratic this fix-wave closed sail through silently.
    // Tightened to 15ms: ~2.8x headroom over the fixed ~5.4ms baseline, and
    // a full ~2x BELOW the buggy ~31ms median — outside the observed noise
    // band on both sides.
    const COMPUTE_ALL_BOUND: Duration = Duration::from_millis(150); // target: 50ms (architectural)
    const COMPUTE_ALL_SYNTHETIC_BOUND: Duration = Duration::from_millis(15); // ~2.8x ~5.4ms measured baseline

    // `COMPUTE_ALL_SCALING_FACTOR`: a machine-INDEPENDENT complexity-class
    // check, added because ANY magnitude bound (both above) is inherently
    // machine-speed-dependent — the exact property that made the original
    // 30ms bound marginal (see that constant's own doc). Measuring
    // `compute_all` at TWO corpus sizes and asserting the ratio stays under
    // this factor tests the ALGORITHM's complexity class directly instead
    // of an absolute time.
    //
    // FOUR rounds of empirical tuning were needed — each one driven by
    // the reviewer (or, in round 4, this file's own author) actually
    // RUNNING the gate repeatedly under real conditions rather than trusting
    // a single measurement or a plausible-sounding theoretical ratio. This
    // history is kept in full because the failure modes are non-obvious and
    // a future editor changing these numbers should understand what already
    // didn't work and why:
    //
    // **v1** — MEDIAN of 9 samples, 4x file-count ratio (250 vs. 1000
    // files), factor 10. FAILED: the reviewer ran the committed gate 5x
    // back-to-back and got 2/5 FALSE FAILURES on the FIXED implementation.
    // Root cause: the small (250-file) corpus takes only ~1-2ms, a scale
    // where OS scheduling/cache noise is a LARGE FRACTION of the
    // measurement — and that noise is STRICTLY ONE-SIDED (an interrupted
    // run is always slower, never faster), so the MEDIAN of a load-skewed
    // sample set is itself skewed upward, inflating the ratio.
    //
    // **v2** — switched to [`min_of`] instead of [`median`] (the correct
    // estimator when noise is one-sided — see that function's doc), same
    // 250-vs-1000 sizes, factor 10. Isolated `-- compute_all`-filtered runs
    // looked clean (fixed 4.37-6.89, buggy 10.82-12.51, no overlap) — but
    // FAILED when actually run as part of the FULL, UNFILTERED test binary
    // (`cargo test --release --test perf_bounds`, no test-name filter — the
    // REAL CI invocation shape): the 8 OTHER tests in this file run first
    // and leave residual system load/cache pressure that `compute_all`'s
    // small 250-file measurement doesn't fully recover from before the gate
    // starts timing, inflating the ratio; a full-suite run measured 9.966,
    // just over the factor-10 threshold.
    //
    // **v3** — per the team lead's own contingency plan, raised BOTH corpus
    // sizes so absolute times sit further above the noise floor: 500 vs.
    // 2000 (same 4x ratio), still sequential (measure ALL of 500 first,
    // then ALL of 2000). Measured via repeated FULL, UNFILTERED runs (the
    // real invocation shape, learned from v2's failure): fixed-code ratio
    // swung from ~3 to ~10.3 across just 10 runs — WORSE variance than v2,
    // and briefly overlapping the buggy range. Root cause: measuring one
    // size FULLY before starting the other is vulnerable to a SUSTAINED
    // (not merely transient) load window landing almost entirely within
    // just ONE of the two measurement phases — `min_of` alone only rejects
    // brief, one-sided spikes; it cannot recover a "true" fast measurement
    // from a phase where the ENTIRE window was under elevated load, and the
    // much-longer 2000-file phase is more likely to catch such a window
    // than the shorter 500-file phase running moments before it.
    //
    // **v4 (current)** — kept 500 vs. 2000 and `min_of`, but INTERLEAVED
    // the two measurements (`measure_compute_all_interleaved`: small, big,
    // small, big, ... within one shared timing loop, both snapshots
    // pre-built) instead of measuring one size fully before the other. This
    // makes both sizes share the EXACT SAME sequence of temporal windows, so
    // a sustained load spell hits both proportionally rather than landing on
    // just one phase. Re-measured over repeated FULL, UNFILTERED runs:
    // - FIXED code min-ratio range: **3.013-4.715** (10 separate full-suite runs)
    // - BUGGY code min-ratio range: **10.409-12.224** (6 separate full-suite
    //   runs, `effective_incoming_count` temporarily reverted to the old
    //   `event_edges` scan for the measurement, then restored)
    // A clean, non-overlapping gap 5.69 wide — far more comfortable than any
    // prior version. 7 sits with real margin on both sides (2.29 above the
    // fixed max, 3.41 below the buggy min) — unlike an absolute-time bound,
    // this margin does not shrink or grow with the machine running it,
    // since it's a ratio of two measurements taken on the SAME machine, in
    // the SAME process, in the SAME interleaved sequence. Hardened by
    // running the FIXED-code gate 10x back-to-back via the FULL, UNFILTERED
    // command (deliberately inducing load, matching how the reviewer found
    // the v1/v3 flakes) requiring 10/10 green, then the BUGGY scan 5x via
    // the same full command requiring 5/5 red — see the task report for the
    // exact run log.
    const COMPUTE_ALL_SCALING_FACTOR: u32 = 7;

    fn median(mut samples: Vec<Duration>) -> Duration {
        samples.sort();
        samples[samples.len() / 2]
    }

    /// The MINIMUM of `samples` — the statistic [`measure_compute_all`] uses
    /// instead of [`median`] (t3 final review, v2). Wall-clock measurement
    /// noise on a shared machine (OS scheduling preemption, cache/TLB
    /// eviction from other processes, CPU frequency scaling) is ONE-SIDED: an
    /// interrupted run is always slower than an uninterrupted one, never
    /// faster. That makes the minimum of repeated samples the
    /// maximum-likelihood estimator of the TRUE cost — every sample above it
    /// is explained by "plus some noise," while the median of a noise-skewed
    /// sample set is ITSELF skewed toward whatever fraction of samples
    /// happened to get interrupted (proven empirically: the median-based
    /// scaling gate this replaces flaked 2/5 times on the FIXED
    /// implementation when the reviewer ran it 5x back-to-back, deliberately
    /// under load). This is the standard estimator throughout the
    /// microbenchmarking literature (`hyperfine`, Criterion-style harnesses)
    /// for exactly this reason — see `COMPUTE_ALL_SCALING_FACTOR`'s doc for
    /// the re-measurement this motivated. Used for BOTH `compute_all`'s
    /// magnitude bounds and its scaling ratio (not just the ratio) — the
    /// same one-sided-noise argument applies to a plain "is this under Nms"
    /// check just as much as to a ratio of two such checks.
    fn min_of(samples: &[Duration]) -> Duration {
        *samples.iter().min().expect("samples is never empty")
    }

    /// A minimal `app.json` so `LspSnapshot::build_full`/`build_full_with_parsed`
    /// (which hard-require one at the workspace root, via `SnapshotBuilder`)
    /// accept the perf_support-generated directory as a workspace. No
    /// `.alpackages` is written — the synthetic corpus has zero dependencies
    /// by design, so dependency resolution sees an empty set (not an error).
    fn write_minimal_app_json(dir: &Path) {
        std::fs::write(
            dir.join("app.json"),
            r#"{
    "id": "00000000-0000-0000-0000-000000000001",
    "name": "PerfCorpus",
    "publisher": "bench",
    "version": "1.0.0.0"
}"#,
        )
        .expect("write perf-corpus app.json");
    }

    fn corpus_dir(file_count: usize) -> TempDir {
        let dir = TempDir::new().unwrap();
        write_minimal_app_json(dir.path());
        perf_support::generate_corpus(dir.path(), file_count);
        dir
    }

    /// Build a batch-built `LspSnapshot` over a fresh `file_count`-file
    /// corpus, for the query-handler bounds checks below.
    fn build_snapshot(file_count: usize) -> (TempDir, LspSnapshot) {
        let dir = corpus_dir(file_count);
        let snap = LspSnapshot::build_full(dir.path()).expect("build_full");
        (dir, snap)
    }

    /// As [`build_snapshot`], but also returns the workspace `ParsedUnit` an
    /// [`Updater`] needs to own as its mutable working state — for the
    /// rung-1/rung-2 incremental-update bounds checks. Dependency parse
    /// arenas are NOT returned here — they're dropped inside
    /// `build_full_with_parsed` once the frozen dep `DeclSurface` tier is
    /// derived (T3 Task 12's owned-DeclSurface lifecycle).
    fn build_snapshot_with_parsed(file_count: usize) -> (TempDir, LspSnapshot, ParsedUnit) {
        let dir = corpus_dir(file_count);
        let (snap, workspace) =
            LspSnapshot::build_full_with_parsed(dir.path()).expect("build_full_with_parsed");
        (dir, snap, workspace)
    }

    #[test]
    fn build_full_100_files_within_bound() {
        let dir = corpus_dir(100);

        // Warm-up: first pass pages the corpus into the OS file cache so the
        // timed runs measure the build, not cold disk I/O.
        LspSnapshot::build_full(dir.path()).expect("build_full (warm-up)");

        let mut samples = Vec::with_capacity(3);
        for _ in 0..3 {
            let start = Instant::now();
            LspSnapshot::build_full(dir.path()).expect("build_full");
            samples.push(start.elapsed());
        }
        let m = median(samples.clone());
        println!(
            "[perf_bounds] build_full_100_files: median={m:?} bound={BUILD_FULL_100_BOUND:?} samples={samples:?}"
        );
        assert!(
            m <= BUILD_FULL_100_BOUND,
            "100-file build_full median {m:?} exceeds 3x-target bound {BUILD_FULL_100_BOUND:?} (samples: {samples:?})"
        );
    }

    #[test]
    fn build_full_1000_files_within_bound() {
        let dir = corpus_dir(1000);

        LspSnapshot::build_full(dir.path()).expect("build_full (warm-up)");

        let mut samples = Vec::with_capacity(3);
        for _ in 0..3 {
            let start = Instant::now();
            LspSnapshot::build_full(dir.path()).expect("build_full");
            samples.push(start.elapsed());
        }
        let m = median(samples.clone());
        println!(
            "[perf_bounds] build_full_1000_files: median={m:?} bound={BUILD_FULL_1000_BOUND:?} samples={samples:?}"
        );
        assert!(
            m <= BUILD_FULL_1000_BOUND,
            "1000-file build_full median {m:?} exceeds 3x-target bound {BUILD_FULL_1000_BOUND:?} (samples: {samples:?})"
        );
    }

    #[test]
    fn prepare_within_bound() {
        let (dir, snap) = build_snapshot(1000);
        let uri = path_to_uri(&dir.path().join(perf_support::file_name(1)))
            .as_str()
            .to_string();

        // Line 2 is `    procedure Proc0()` in generated file content (see
        // perf_support's corpus generator); character 15 lands inside the
        // name span covering that definition.
        let warm = handlers::prepare(&snap, PositionEncoding::Utf8, &uri, 2, 15);
        assert!(warm.is_some(), "sanity: warm-up must find a definition");

        let mut samples = Vec::with_capacity(5);
        for _ in 0..5 {
            let start = Instant::now();
            let result = handlers::prepare(&snap, PositionEncoding::Utf8, &uri, 2, 15);
            samples.push(start.elapsed());
            assert!(result.is_some(), "sanity: must find a definition");
        }
        let m = median(samples.clone());
        println!("[perf_bounds] prepare: median={m:?} bound={QUERY_BOUND:?} samples={samples:?}");
        assert!(
            m <= QUERY_BOUND,
            "prepare median {m:?} exceeds 3x-target bound {QUERY_BOUND:?} (samples: {samples:?})"
        );
    }

    #[test]
    fn incoming_within_bound() {
        let (_dir, snap) = build_snapshot(1000);
        let hub_file = perf_support::file_name(perf_support::HUB_INDEX);
        let hub_proc0 = snap.decls_by_file[&hub_file]
            .iter()
            .find(|d| d.name == "Proc0")
            .expect("hub Proc0 decl")
            .id
            .clone();
        let data = ItemData { node: hub_proc0 };

        // Warm-up.
        let _ = handlers::incoming(&snap, PositionEncoding::Utf8, &data);

        let mut samples = Vec::with_capacity(5);
        for _ in 0..5 {
            let start = Instant::now();
            let result = handlers::incoming(&snap, PositionEncoding::Utf8, &data);
            samples.push(start.elapsed());
            assert_eq!(
                result.len(),
                999,
                "sanity: hub Proc0 must show real fan-in (999 = 1000 files - 1 \
                 hub; the new backend groups by distinct caller, and this \
                 corpus's hub call gives each of the 999 non-hub files exactly \
                 one distinct caller routine, so the count matches the legacy \
                 pipeline's own per-call-site count 1:1 here)"
            );
        }
        let m = median(samples.clone());
        println!(
            "[perf_bounds] incoming: median={m:?} bound={INCOMING_HIGH_FANIN_BOUND:?} samples={samples:?}"
        );
        assert!(
            m <= INCOMING_HIGH_FANIN_BOUND,
            "incoming median {m:?} exceeds bound {INCOMING_HIGH_FANIN_BOUND:?} (samples: {samples:?})"
        );
    }

    #[test]
    fn outgoing_within_bound() {
        let (_dir, snap) = build_snapshot(1000);
        let file1 = perf_support::file_name(1);
        let proc0 = snap.decls_by_file[&file1]
            .iter()
            .find(|d| d.name == "Proc0")
            .expect("file-1 Proc0 decl")
            .id
            .clone();
        let data = ItemData { node: proc0 };

        // Warm-up.
        let _ = handlers::outgoing(&snap, PositionEncoding::Utf8, &data);

        let mut samples = Vec::with_capacity(5);
        for _ in 0..5 {
            let start = Instant::now();
            let result = handlers::outgoing(&snap, PositionEncoding::Utf8, &data);
            samples.push(start.elapsed());
            assert_eq!(
                result.len(),
                3,
                "sanity: file-1 Proc0 must show real fan-out (1 cross-file \
                 qualified + 2 local)"
            );
        }
        let m = median(samples.clone());
        println!("[perf_bounds] outgoing: median={m:?} bound={QUERY_BOUND:?} samples={samples:?}");
        assert!(
            m <= QUERY_BOUND,
            "outgoing median {m:?} exceeds 3x-target bound {QUERY_BOUND:?} (samples: {samples:?})"
        );
    }

    /// Measure `compute_all`'s MINIMUM wall-clock time (see [`min_of`]'s doc
    /// for why min, not median) on a `file_count`-file event-bearing corpus:
    /// 5 warm-up iterations then 9 timed samples. Used ONLY for
    /// `compute_all_within_bound`'s magnitude bounds (a single measurement,
    /// not a ratio) — see [`measure_compute_all_interleaved`] for the
    /// SCALING assertion's two-corpus-size measurement, which needs a
    /// different (interleaved) sampling strategy for reasons that function's
    /// own doc explains.
    fn measure_compute_all(file_count: usize) -> (Duration, Vec<Duration>) {
        let (_dir, snap) = build_snapshot(file_count);
        let cfg = DiagnosticConfig::default();
        for _ in 0..5 {
            let _ = compute_all(&snap, PositionEncoding::Utf8, &cfg); // warm-up
        }

        let mut samples = Vec::with_capacity(9);
        for _ in 0..9 {
            let start = Instant::now();
            let result = compute_all(&snap, PositionEncoding::Utf8, &cfg);
            samples.push(start.elapsed());
            assert!(
                !result.is_empty(),
                "sanity: compute_all must produce entries for a {file_count}-file workspace"
            );
        }
        (min_of(&samples), samples)
    }

    /// Measure `compute_all`'s MINIMUM wall-clock time at TWO corpus sizes,
    /// INTERLEAVED (small, big, small, big, ...) rather than sequentially
    /// (all of one size, then all of the other) — the fix for a real failure
    /// mode found empirically (t3 final review v3): measuring size A fully,
    /// then size B fully, is vulnerable to a SUSTAINED (not merely
    /// transient) load window landing almost entirely within just ONE of
    /// the two measurement phases. `min_of` alone (v2) assumes noise is a
    /// series of brief, one-sided spikes a large-enough sample will dodge at
    /// least once — true for transient interrupts, but NOT for a period of
    /// genuinely elevated system load lasting as long as an entire
    /// measurement phase (confirmed: repeated full-suite runs on this
    /// machine showed the sequential 500-vs-2000 ratio swing from ~3 to
    /// ~10 for the IDENTICAL fixed code — the 2000-file phase alone was
    /// sometimes caught entirely inside such a window while the much
    /// shorter 500-file phase, running moments earlier, was not).
    /// Interleaving means both sizes share the EXACT SAME sequence of
    /// temporal windows, so a sustained load spell hits both proportionally
    /// instead of landing disproportionately on whichever phase happens to
    /// be running during it — turning a whole-run confound back into the
    /// per-sample noise `min_of` was already designed to reject.
    fn measure_compute_all_interleaved(
        small_count: usize,
        big_count: usize,
    ) -> (Duration, Duration, Vec<Duration>, Vec<Duration>) {
        let (_small_dir, small_snap) = build_snapshot(small_count);
        let (_big_dir, big_snap) = build_snapshot(big_count);
        let cfg = DiagnosticConfig::default();

        for _ in 0..5 {
            let _ = compute_all(&small_snap, PositionEncoding::Utf8, &cfg); // warm-up
            let _ = compute_all(&big_snap, PositionEncoding::Utf8, &cfg); // warm-up
        }

        let mut small_samples = Vec::with_capacity(9);
        let mut big_samples = Vec::with_capacity(9);
        for _ in 0..9 {
            let start = Instant::now();
            let small_result = compute_all(&small_snap, PositionEncoding::Utf8, &cfg);
            small_samples.push(start.elapsed());
            assert!(
                !small_result.is_empty(),
                "sanity: compute_all must produce entries for a {small_count}-file workspace"
            );

            let start = Instant::now();
            let big_result = compute_all(&big_snap, PositionEncoding::Utf8, &cfg);
            big_samples.push(start.elapsed());
            assert!(
                !big_result.is_empty(),
                "sanity: compute_all must produce entries for a {big_count}-file workspace"
            );
        }
        (
            min_of(&small_samples),
            min_of(&big_samples),
            small_samples,
            big_samples,
        )
    }

    /// `compute_all` — the full diagnostics recompute `on_swap` runs after
    /// EVERY snapshot swap (t3 whole-branch review, blocker fix). This
    /// corpus is event-bearing (2 publishers + 2 subscribers per file — see
    /// `tests/perf_support/mod.rs`'s doc), so `event_edges`/
    /// `publisher_fanout` are genuinely populated at scale — the exact
    /// condition that was missing before this fix-wave and let the
    /// O(decls * event_edges) quadratic in `effective_incoming_count` go
    /// unmeasured through 17 prior tasks.
    #[test]
    fn compute_all_within_bound() {
        let sanity_dir = corpus_dir(1000);
        let sanity_snap = LspSnapshot::build_full(sanity_dir.path()).expect("build_full");
        assert_eq!(
            sanity_snap.event_edges.len(),
            1000 * perf_support::PUBLISHERS_PER_FILE,
            "sanity: this 1000-file event-bearing corpus must have \
             PUBLISHERS_PER_FILE publisher declarations per file"
        );
        let file1 = perf_support::file_name(1);
        assert_eq!(
            sanity_snap.decls_by_file[&file1].len(),
            perf_support::PROCS_PER_FILE + perf_support::EVENT_ROUTINES_PER_FILE,
            "sanity: every file declares PROCS_PER_FILE plain procedures plus \
             EVENT_ROUTINES_PER_FILE event-bearing routines"
        );
        drop(sanity_snap);
        drop(sanity_dir);

        let (m1000, samples1000) = measure_compute_all(1000);
        println!(
            "[perf_bounds] compute_all(1000): min={m1000:?} absolute_bound={COMPUTE_ALL_BOUND:?} \
             synthetic_bound={COMPUTE_ALL_SYNTHETIC_BOUND:?} samples={samples1000:?}"
        );
        // Both MAGNITUDE bounds asserted, same dual-bound convention as rung
        // 1/2 (see COMPUTE_ALL_SYNTHETIC_BOUND's own doc for why it was
        // tightened, and why a magnitude bound alone is not enough — see the
        // SCALING assertion below, which is the one that actually matters).
        // `min_of`, not `median` — see that function's doc for why.
        assert!(
            m1000 <= COMPUTE_ALL_BOUND,
            "compute_all min {m1000:?} exceeds the architectural bound {COMPUTE_ALL_BOUND:?} \
             (samples: {samples1000:?})"
        );
        assert!(
            m1000 <= COMPUTE_ALL_SYNTHETIC_BOUND,
            "compute_all min {m1000:?} exceeds the corpus-relative bound \
             {COMPUTE_ALL_SYNTHETIC_BOUND:?} (samples: {samples1000:?}) — this is the exact \
             O(decls * event_edges) regression class the t3 whole-branch review fix-wave closed"
        );

        // SCALING assertion (t3 final review, v1 -> v2 -> v3 -> v4) — the
        // gate that actually matters, machine-speed-independent unlike the
        // magnitude bounds above (see COMPUTE_ALL_SCALING_FACTOR's own doc
        // for the full history of what each prior version got wrong). Uses
        // its OWN, INDEPENDENT, INTERLEAVED pair of corpus-size measurements
        // (500 vs. 2000 — NOT reusing `m1000` above; see
        // `measure_compute_all_interleaved`'s doc for why interleaving,
        // specifically, was necessary).
        let (msmall, mbig, samplessmall, samplesbig) = measure_compute_all_interleaved(500, 2000);
        let ratio = mbig.as_secs_f64() / msmall.as_secs_f64();
        println!(
            "[perf_bounds] compute_all scaling: t(500)={msmall:?} t(2000)={mbig:?} ratio={ratio:.3} \
             samples500={samplessmall:?} samples2000={samplesbig:?}"
        );
        assert!(
            mbig < msmall * COMPUTE_ALL_SCALING_FACTOR,
            "compute_all's 2000-file cost ({mbig:?}) must be under {COMPUTE_ALL_SCALING_FACTOR}x \
             its 500-file cost ({msmall:?}, ratio={ratio:.3}) — a ratio at/above \
             {COMPUTE_ALL_SCALING_FACTOR}x is the complexity-class signature of the \
             O(decls * event_edges) quadratic this fix-wave closed, and unlike the magnitude \
             bounds above, this check does not get weaker on a faster machine"
        );
    }

    #[test]
    fn rung1_body_edit_apply_batch_within_bound() {
        let (dir, base, parsed) = build_snapshot_with_parsed(1000);
        let mut updater = Updater::new(dir.path().to_path_buf(), parsed);
        let target = dir.path().join(perf_support::file_name(1));

        // Body-only edit, on disk once — outside the timed region. No
        // routine identity/signature change, so this must stay rung 1 on
        // every apply below (see `body_only_comment_edit`'s doc).
        perf_support::body_only_comment_edit(dir.path(), 1000, 1);
        let batch = vec![ChangeEvent::FileSaved(target)];

        // Warm-up (also proves the rung-1 path works before timing it).
        let (warm_snap, warm_rung) = updater
            .apply_batch(&base, &batch)
            .expect("apply_batch (warm-up)");
        assert_eq!(
            warm_rung,
            Rung::One,
            "a comment-only body edit must stay rung 1"
        );

        // Every subsequent call re-saves the SAME (already-applied) content:
        // the fingerprint stays unchanged relative to whatever was just
        // published, which is exactly rung 1's own gate condition — so
        // repeating the identical batch keeps exercising the genuine rung-1
        // path without needing a fresh edit each iteration.
        let mut cur = warm_snap;
        let mut samples = Vec::with_capacity(3);
        for _ in 0..3 {
            let start = Instant::now();
            let (next, rung) = updater
                .apply_batch(&cur, &batch)
                .expect("apply_batch must succeed");
            samples.push(start.elapsed());
            assert_eq!(rung, Rung::One, "a comment-only body edit must stay rung 1");
            cur = next;
        }
        let m = median(samples.clone());
        println!(
            "[perf_bounds] rung1_body_edit: median={m:?} absolute_bound={RUNG1_BOUND:?} \
             synthetic_bound={RUNG1_SYNTHETIC_BOUND:?} samples={samples:?}"
        );
        // Both bounds are asserted: the absolute (CDO-anchored) bound guards
        // CDO-scale behavior; the synthetic (corpus-relative) bound catches
        // an order-of-magnitude regression ON THIS CORPUS that the absolute
        // bound's headroom would otherwise miss — see both constants' doc.
        assert!(
            m <= RUNG1_BOUND,
            "rung-1 body-edit median {m:?} exceeds CDO-anchored bound {RUNG1_BOUND:?} (samples: {samples:?})"
        );
        assert!(
            m <= RUNG1_SYNTHETIC_BOUND,
            "rung-1 body-edit median {m:?} exceeds corpus-relative bound {RUNG1_SYNTHETIC_BOUND:?} \
             (samples: {samples:?}) — this is a real regression on THIS corpus even though it \
             may still be under the looser CDO-anchored bound"
        );
    }

    #[test]
    fn rung1_body_edit_scoped_within_bound() {
        let (dir, base, parsed) = build_snapshot_with_parsed(1000);
        let mut updater = Updater::new(dir.path().to_path_buf(), parsed);
        let target = dir.path().join(perf_support::file_name(1));

        // Body-only edit, on disk once — outside the timed region. No
        // routine identity/signature change, so this must stay rung 1 on
        // every apply below (see `body_only_comment_edit`'s doc).
        perf_support::body_only_comment_edit(dir.path(), 1000, 1);
        let batch = vec![ChangeEvent::FileSaved(target)];

        // The PRODUCTION path: `Rung1Context` built ONCE (like
        // `spawn_updater`'s scoped-context loop) and reused across every
        // sample below — see F6's finding for why `apply_batch` alone
        // (rebuilding this context per call) over-measures what a real
        // keystroke-save actually costs.
        let ctx = Rung1Context::build(&base, updater.workspace());

        // Warm-up (also proves the rung-1 path works before timing it).
        let (warm, _delta) = updater
            .apply_batch_scoped(&base, &batch, &ctx)
            .expect("a comment-only body edit must stay rung 1");

        // Every subsequent call re-saves the SAME (already-applied) content:
        // the fingerprint stays unchanged relative to whatever was just
        // published, which is exactly rung 1's own gate condition — so
        // repeating the identical batch keeps exercising the genuine rung-1
        // path without needing a fresh edit each iteration.
        let mut cur = warm;
        let mut samples = Vec::with_capacity(3);
        for _ in 0..3 {
            let start = Instant::now();
            let (next, _delta) = updater
                .apply_batch_scoped(&cur, &batch, &ctx)
                .expect("must stay rung 1");
            samples.push(start.elapsed());
            cur = next;
        }
        let m = median(samples.clone());
        println!(
            "[perf_bounds] rung1_body_edit_scoped: median={m:?} absolute_bound={RUNG1_BOUND:?} \
             synthetic_bound={RUNG1_SCOPED_SYNTHETIC_BOUND:?} samples={samples:?}"
        );
        // Both bounds are asserted, mirroring the `apply_batch` test above.
        assert!(
            m <= RUNG1_BOUND,
            "rung-1 scoped body-edit median {m:?} exceeds CDO-anchored bound {RUNG1_BOUND:?} \
             (samples: {samples:?})"
        );
        assert!(
            m <= RUNG1_SCOPED_SYNTHETIC_BOUND,
            "rung-1 scoped body-edit median {m:?} exceeds corpus-relative bound \
             {RUNG1_SCOPED_SYNTHETIC_BOUND:?} (samples: {samples:?}) — this is a real \
             regression on THIS corpus even though it may still be under the looser \
             CDO-anchored bound"
        );
    }

    #[test]
    fn rung2_signature_edit_apply_batch_within_bound() {
        // Warm-up: a full cycle outside the timed samples below, both to
        // page the corpus into the OS file cache and to prove the rung-2
        // path works before timing it.
        {
            let (dir, base, parsed) = build_snapshot_with_parsed(1000);
            let mut updater = Updater::new(dir.path().to_path_buf(), parsed);
            perf_support::rewrite_with_extra_procedure(dir.path(), 1000, 1);
            let batch = vec![ChangeEvent::FileSaved(
                dir.path().join(perf_support::file_name(1)),
            )];
            let (_new_snap, rung) = updater
                .apply_batch(&base, &batch)
                .expect("apply_batch (warm-up)");
            assert_eq!(rung, Rung::Two, "a new-procedure edit must take rung 2");
        }

        // Each timed sample needs its OWN fresh baseline: `apply_batch`'s
        // rung-2 gate compares the fresh parse's fingerprint against the
        // CURRENTLY PUBLISHED one, so re-using one already-escalated
        // snapshot across iterations would silently degrade to rung 1 on the
        // 2nd/3rd call (nothing left to detect as changed, since the
        // "new procedure" is already part of the published state). A fresh
        // corpus + fresh `Updater` per sample keeps every iteration a
        // genuine rung-2 escalation, at the cost of the (untimed) setup
        // running 3 times instead of once — cheap at this corpus scale (see
        // `.superpowers/sdd/t3-stage-split.md`'s synthetic-corpus numbers).
        let mut samples = Vec::with_capacity(3);
        for _ in 0..3 {
            let (dir, base, parsed) = build_snapshot_with_parsed(1000);
            let mut updater = Updater::new(dir.path().to_path_buf(), parsed);
            perf_support::rewrite_with_extra_procedure(dir.path(), 1000, 1);
            let batch = vec![ChangeEvent::FileSaved(
                dir.path().join(perf_support::file_name(1)),
            )];

            let start = Instant::now();
            let (_new_snap, rung) = updater
                .apply_batch(&base, &batch)
                .expect("apply_batch must succeed");
            samples.push(start.elapsed());
            assert_eq!(rung, Rung::Two, "a new-procedure edit must take rung 2");
        }
        let m = median(samples.clone());
        println!(
            "[perf_bounds] rung2_signature_edit: median={m:?} absolute_bound={RUNG2_BOUND:?} \
             synthetic_bound={RUNG2_SYNTHETIC_BOUND:?} samples={samples:?}"
        );
        // Both bounds are asserted — see `RUNG2_SYNTHETIC_BOUND`'s doc.
        assert!(
            m <= RUNG2_BOUND,
            "rung-2 signature-edit median {m:?} exceeds CDO-anchored bound {RUNG2_BOUND:?} (samples: {samples:?})"
        );
        assert!(
            m <= RUNG2_SYNTHETIC_BOUND,
            "rung-2 signature-edit median {m:?} exceeds corpus-relative bound {RUNG2_SYNTHETIC_BOUND:?} \
             (samples: {samples:?}) — this is a real regression on THIS corpus even though it \
             may still be under the looser CDO-anchored bound"
        );
    }
}
