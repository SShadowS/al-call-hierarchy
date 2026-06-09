//! `run_analyze` — the gate pipeline lib entry the `alsem analyze` bin wraps and the
//! differential tests call in-process. Mirrors the `analyze` action in al-sem
//! `src/cli/index.ts`.
//!
//! Stage 1 (SARIF + projection + filters): NO baseline, NO inline suppression.
//! Stage 2b (this layer): adds `--format pr-summary`, `--fail-on`, `--require-dependencies`,
//! the dependency-coverage preflight, and the CI exit-code contract.
//!
//! Pipeline:
//!   assemble_and_resolve_workspace(ws)  (L0→L3, source-only)
//!   → resolve the detector set (preset | --detector | default)
//!   → run_detectors (L4 inside the DetectorContext + L5) — pre-sorted Finding[]
//!   → project_finding per finding (display names + 1-based location)
//!   → filter_findings (min-severity, detector allow-list)
//!   → scope filter (primary drops dependency-anchored findings)
//!   → limit
//!   → format (sarif | pr-summary)
//!   → preflight + exit-code (--fail-on / --require-dependencies)
//!
//! Source-only: the transaction-integrity preset is intra-app, so this drives the
//! source-only `run_detectors` (not the cross-app variant). Every workspace object is
//! "primary", so the scope=primary filter keeps everything.

use std::path::Path;

use crate::engine::gate::app_attribution::App;
use crate::engine::gate::baseline::{apply_baseline, load_baseline, save_baseline};
use crate::engine::gate::exit_code::{compute_finding_exit, exit};
use crate::engine::gate::filter::{filter_findings, scope_filter, FilterOptions, Scope};
use crate::engine::gate::format_json::{build_analyze_json, JsonFormatInputs};
use crate::engine::gate::format_pr_summary::format_pr_summary;
use crate::engine::gate::format_sarif::format_sarif;
use crate::engine::gate::format_terminal::{format_terminal, format_terminal_grouped, GroupBy};
use crate::engine::gate::inline_suppression::{apply_inline_suppressions, build_suppression_map};
use crate::engine::gate::model_instance_id::compute_gate_model_instance_id;
use crate::engine::gate::preflight::evaluate_preflight;
use crate::engine::gate::presets::resolve_analyze_detectors;
use crate::engine::gate::projection::{project_finding, ProjectionIndex};
use crate::engine::gate::version::alsem_version;
use crate::engine::l3::l3_workspace::assemble_and_resolve_workspace;
use crate::engine::l5::registry::run_detectors;

/// Output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Sarif,
    PrSummary,
    /// Rich terminal output (colour, grouping). The future Stage A1 formatter.
    Terminal,
    /// Machine-readable JSON envelope (the future Stage A2 formatter).
    Json,
    /// Self-contained HTML report (the future Stage A3 formatter).
    Html,
}

/// The "not yet implemented" `Err` string for a stub formatter. Referenced from BOTH
/// the main format match and the `empty_output_result` fallback so the wording cannot
/// drift, and so A1–A3 only delete the arm in one conceptual place. Returns `None` for
/// the already-implemented formats (Sarif / PrSummary / Json).
fn stub_not_implemented(fmt: OutputFormat) -> Option<String> {
    let (name, stage) = match fmt {
        OutputFormat::Html => ("html", "A3"),
        OutputFormat::Sarif
        | OutputFormat::PrSummary
        | OutputFormat::Json
        | OutputFormat::Terminal => return None,
    };
    Some(format!(
        "format '{name}' not yet implemented (stage {stage})"
    ))
}

/// Parsed `analyze` arguments.
#[derive(Debug, Clone)]
pub struct AnalyzeArgs {
    pub workspace: String,
    /// `--min-severity` (validated by the caller / CLI).
    pub min_severity: Option<String>,
    /// `--detector <ids>` (comma-separated). Mutually exclusive with `preset`.
    pub detector: Option<String>,
    /// `--preset <name>`.
    pub preset: Option<String>,
    /// `--scope` (default Primary).
    pub scope: Scope,
    /// `--limit`.
    pub limit: Option<usize>,
    /// `--format`.
    pub format: OutputFormat,
    /// `--sarif-version-override` — pins `driver.version` for byte-stable output.
    /// When `None`, `default_version` is used. (SARIF only; PR-summary embeds no version.)
    pub sarif_version_override: Option<String>,
    /// `--fail-on <sev>` — when `Some`, exit `FINDINGS` (1) if any kept finding is
    /// at/above this severity. Already-validated severity string (`None` ⇒ never gate).
    pub fail_on: Option<String>,
    /// `--require-dependencies` — make a degraded preflight FAIL (exit 4).
    pub require_dependencies: bool,
    /// `--baseline <path>` — when `Some`, load the baseline fingerprint set and drop any
    /// finding whose fingerprint is in it BEFORE inline suppression (Stage 3b).
    pub baseline: Option<String>,
    /// `--update-baseline` — when set together with `baseline`, write the current
    /// post-filter/scope/limit finding set to the baseline file (the new floor).
    pub update_baseline: bool,
    /// Disable inline `// al-sem-ignore` suppression. al-sem applies inline suppression
    /// unconditionally (default-ON, like a compiler pragma); this flag exists ONLY so the
    /// differential can capture the UN-suppressed SARIF (the +1 finding) — it has no CLI
    /// surface. Default `false` (suppression ON).
    pub disable_inline_suppression: bool,
    /// `--group-by <object|routine|table|detector|file>` — controls how the
    /// `terminal` formatter groups findings. Validated by the CLI before
    /// entering the pipeline. `None` means no explicit grouping was requested
    /// (the terminal formatter will use its default). Ignored by sarif/pr-summary/json/html.
    pub group_by: Option<String>,
    /// `--deterministic` — pins timestamps and version for byte-stable output.
    /// Used by the `json` formatter to pin `generatedAt` to the UNIX epoch.
    pub deterministic: bool,
}

/// Read the workspace root `app.json` identity (`id` / `publisher` / `name` / `version`)
/// into the gate `App` registry. Mirrors al-sem `WorkspaceProvider.collect`:
///   - defaults: `publisher = "unknown"`, `name = "unknown"`, `version = "0.0.0.0"`.
///   - `appGuid` from the `id` (already validated by `compute_gate_model_instance_id`).
/// SOURCE-ONLY: exactly one app per run. Returns an empty registry if `app.json` is
/// unreadable (engine-never-throws; attribution then falls back to "(unknown app)").
fn read_workspace_apps(ws: &Path) -> Vec<App> {
    let Ok(text) = std::fs::read_to_string(ws.join("app.json")) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let Some(app_guid) = v
        .get("id")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
    else {
        return Vec::new();
    };
    let publisher = v
        .get("publisher")
        .and_then(|x| x.as_str())
        .unwrap_or("unknown")
        .to_string();
    let name = v
        .get("name")
        .and_then(|x| x.as_str())
        .unwrap_or("unknown")
        .to_string();
    let version = v
        .get("version")
        .and_then(|x| x.as_str())
        .unwrap_or("0.0.0.0")
        .to_string();
    vec![App {
        app_guid: app_guid.to_string(),
        publisher,
        name,
        version,
    }]
}

/// Run the gate `analyze` pipeline and return the formatted output string WITHOUT the
/// trailing newline (the CLI / caller appends `"\n"`, matching al-sem's
/// `process.stdout.write(`${format(...)}\n`)`).
///
/// Backwards-compatible Stage-1 entry: SARIF only, no exit code. Prefer
/// `run_analyze_with_exit` for the full gate (pr-summary + exit code).
pub fn run_analyze(args: &AnalyzeArgs, default_version: &str) -> Result<String, String> {
    run_analyze_with_exit(args, default_version).map(|(out, _exit, _warn)| out)
}

/// Run the gate `analyze` pipeline, returning `(stdout, exit_code, stderr_warning)`.
///
/// The exit code follows the al-sem precedence:
///   CONFIG_ERROR (3) and ANALYSIS_FAILURE (2) are the CALLER's responsibility (bad
///   flags / a thrown pipeline — neither occurs here: detector/preset resolution errors
///   return `Err`, which the bin maps to 3). This fn computes:
///   PREFLIGHT_FAILED (4) > FINDINGS (1) > CLEAN (0).
///
/// The third tuple field is the preflight degraded warning message (F2: "no silent
/// clean" contract). `Some(msg)` when `pf.degraded`, `None` when coverage is complete.
/// The bin emits `al-sem: warning: {msg}` to stderr; tests may inspect it directly.
/// The warning is emitted REGARDLESS of `--require-dependencies` (only the
/// FAILED→exit-4 path needs that flag).
///
/// `default_version` is the engine's real version, used when no SARIF override is given.
///
/// Errors: detector/preset resolution failures (e.g. unknown preset, `--preset` +
/// `--detector` together). A workspace that fails to assemble (fail-closed / unreadable)
/// yields EMPTY findings (engine-never-throws) — empty SARIF / "no findings" PR-summary,
/// a clean preflight, and exit CLEAN.
pub fn run_analyze_with_exit(
    args: &AnalyzeArgs,
    default_version: &str,
) -> Result<(String, u8, Option<String>), String> {
    let detectors = resolve_analyze_detectors(args.preset.as_deref(), args.detector.as_deref())?;

    let version = args
        .sarif_version_override
        .clone()
        .unwrap_or_else(|| default_version.to_string());

    // Assemble with the al-sem GATE modelInstanceId (content-derived, UNPINNED) so the
    // internal RoutineIds embedded in each finding's rootCauseKey — and therefore the
    // SARIF fingerprint hashed over them — byte-match the al-sem `analyze` CLI goldens.
    let ws_path = Path::new(&args.workspace);
    let model_instance_id = match compute_gate_model_instance_id(ws_path) {
        Some(id) => id,
        // Fail-closed layout → empty output, clean preflight, clean exit.
        None => return empty_output_result(args, &version),
    };
    let resolved = match assemble_and_resolve_workspace(ws_path, &model_instance_id) {
        Some(r) => r,
        // Fail-closed / unreadable workspace → empty output.
        None => return empty_output_result(args, &version),
    };

    // L4 + L5: run the selected detectors. Findings come pre-sorted by
    // (detector, primaryLocationKey, rootCauseKey) with dep-anchored findings already
    // role-scoped out (source-only ⇒ no-op).
    let run = run_detectors(&resolved, &detectors);
    // Capture diagnostics + detector stats for the Json formatter (consumed after filtering).
    //
    // al-sem `analyzeWorkspace` (src/index.ts:287-297) concatenates SIX diagnostic
    // sources, IN THIS ORDER, into the flat `result.diagnostics` the JSON envelope
    // serializes:
    //   1. workspace.diagnostics  — provider (remapped "discover") + index/parse
    //   2. depArtifacts.diagnostics — dependency-artifact resolution
    //   3. summarizeDiagnostics   — L4 `computeSummaries`
    //   4. loadedRootsConfig.diagnostics — roots.config.json LOADER (parse/schema)
    //   5. overlayDiagnostics     — roots.config.json OVERLAY (kinds-mismatch)
    //   6. detectDiagnostics      — L5 detector-emitted (e.g. d43 substrate guard)
    //
    // Each source preserves a deterministic (insertion / sorted-file) order — no
    // HashMap/HashSet iteration leaks into this concatenation (the determinism
    // contract). `infra_diagnostics` is the OVERLAY source (5); `run.diagnostics`
    // is DETECT (6). The TS-order #1 (workspace) goes FIRST so it precedes overlay.
    let run_diagnostics: Vec<crate::engine::l5::registry::Diagnostic> = {
        let mut all: Vec<crate::engine::l5::registry::Diagnostic> = Vec::new();
        // (1) workspace.diagnostics — provider (discover) + index, computed from disk.
        all.extend(
            crate::engine::gate::workspace_diagnostics::compute_workspace_diagnostics(ws_path),
        );
        // (2) depArtifacts.diagnostics — TRACKED GAP: the gate's source-only pipeline
        //     does not resolve `.app` dependency artifacts, so this source is always
        //     empty here. (When dep resolution is wired into the gate, emit it in this
        //     slot so a dep diagnostic lands in TS order before summarize.)
        // (3) summarizeDiagnostics — TRACKED GAP: L4 `computeSummaries` runs inside
        //     `run_detectors`'s DetectorContext and currently surfaces no diagnostics
        //     to the gate. Slotted here for TS order when it does.
        // (4) loadedRootsConfig.diagnostics — TRACKED GAP: the roots.config.json LOADER
        //     diagnostics (parse/schema errors) are not yet surfaced separately from the
        //     OVERLAY diagnostics by `compute_root_classifications`. The overlay
        //     diagnostics (5) below cover the kinds-mismatch case the corpus exercises.
        // (5) overlayDiagnostics — roots.config.json overlay (kinds-mismatch warnings).
        all.extend(resolved.infra_diagnostics.iter().map(|d| {
            crate::engine::l5::registry::Diagnostic {
                severity: d.severity.clone(),
                stage: d.stage.clone(),
                message: d.message.clone(),
            }
        }));
        // (6) detectDiagnostics — L5 detector-emitted (d43 substrate guard, ...).
        all.extend(run.diagnostics.iter().cloned());
        all
    };
    let run_detector_stats = run.detector_stats.clone();

    // Project each finding (display names + 1-based location), preserving order.
    let idx = ProjectionIndex::build(&resolved.workspace.objects, &resolved.workspace.routines);
    let mut paired: Vec<(
        crate::engine::gate::projection::FindingSummary,
        &crate::engine::l5::finding::Finding,
    )> = run
        .findings
        .iter()
        .map(|f| (project_finding(f, &idx), f))
        .collect();

    // --- filter: min-severity, then detector allow-list (only when `--detector` set). ---
    let detector_allow = args.detector.as_ref().map(|d| {
        d.split(',')
            .map(|s| s.trim().to_string())
            .collect::<Vec<_>>()
    });
    let opts = FilterOptions {
        min_severity: args.min_severity.clone(),
        detectors: detector_allow,
    };
    {
        let summaries: Vec<_> = paired.iter().map(|(s, _)| s.clone()).collect();
        let kept_ids: std::collections::HashSet<String> = filter_findings(summaries, &opts)
            .into_iter()
            .map(|s| s.id)
            .collect();
        paired.retain(|(s, _)| kept_ids.contains(&s.id));
    }

    // --- scope: primary drops dependency-anchored findings. Source-only ⇒ keep all. ---
    {
        let summaries: Vec<_> = paired.iter().map(|(s, _)| s.clone()).collect();
        let kept_ids: std::collections::HashSet<String> =
            scope_filter(summaries, args.scope, |_obj_id| false)
                .into_iter()
                .map(|s| s.id)
                .collect();
        paired.retain(|(s, _)| kept_ids.contains(&s.id));
    }

    // --- limit: first N (after scope). Order-preserving prefix. ---
    if let Some(n) = args.limit {
        paired.truncate(n);
    }

    // --- baseline suppression (al-sem index.ts:296-302) ---
    // Load the baseline fingerprint set (empty when no --baseline). Drop any finding
    // whose fingerprint is in it. --update-baseline saves the CURRENT (post-limit) set —
    // the new floor — BEFORE the drop, matching al-sem (`saveBaseline(path, limited)`).
    if let Some(path) = &args.baseline {
        if args.update_baseline {
            let summaries: Vec<_> = paired.iter().map(|(s, _)| s.clone()).collect();
            // Engine-never-throws: a write failure is surfaced as Err to the caller,
            // which the bin maps to a config/IO error message.
            save_baseline(Path::new(path), &summaries)
                .map_err(|e| format!("failed to write baseline '{path}': {e}"))?;
        }
        // A malformed baseline (exists but not valid JSON / non-array fingerprints) is
        // an analysis-failure, not a config error: al-sem's loadBaseline throws and the
        // CLI catch emits "al-sem: analysis failure — <msg>" + exits 2.  We surface
        // this as an Err tagged with the "analysis failure — " prefix so the bin can
        // distinguish it from config errors (exit 3) and use exit 2 instead.
        let baseline =
            load_baseline(Path::new(path)).map_err(|e| format!("analysis failure — {e}"))?;
        let summaries: Vec<_> = paired.iter().map(|(s, _)| s.clone()).collect();
        let kept_ids: std::collections::HashSet<String> = apply_baseline(&summaries, &baseline)
            .into_iter()
            .map(|s| s.id)
            .collect();
        paired.retain(|(s, _)| kept_ids.contains(&s.id));
    }

    // --- inline al-sem-ignore suppression (default-ON, al-sem index.ts:304-319) ---
    // Parsed from workspace source files on disk; only ws: units are scanned. The kept
    // set after this is `newFindings` — the set the exit gate and all formats use.
    if !args.disable_inline_suppression {
        let unit_ids: std::collections::HashSet<String> = paired
            .iter()
            .map(|(s, _)| s.primary_location.file.clone())
            .collect();
        let suppression_map = build_suppression_map(ws_path, unit_ids.iter().map(|s| s.as_str()));
        let summaries: Vec<_> = paired.iter().map(|(s, _)| s.clone()).collect();
        let outcome = apply_inline_suppressions(&summaries, &suppression_map);
        // outcome.kept holds indices into `summaries` (= into `paired`); retain them.
        let keep: std::collections::HashSet<usize> = outcome.kept.into_iter().collect();
        let mut i = 0usize;
        paired.retain(|_| {
            let k = keep.contains(&i);
            i += 1;
            k
        });
    }

    // --- dependency-coverage preflight (al-sem Task 2) ---
    // NOTE: coverage is computed HERE (before the format switch) so it is available
    // to the Json formatter. The preflight evaluation + exit-code gate follow below.
    let coverage = resolved.project_coverage_disk(ws_path);

    // --- format ---
    let output = match args.format {
        OutputFormat::Sarif => {
            let summaries: Vec<_> = paired.iter().map(|(s, _)| s.clone()).collect();
            let raws: Vec<&crate::engine::l5::finding::Finding> =
                paired.iter().map(|(_, r)| *r).collect();
            format_sarif(&summaries, &raws, &version)
        }
        OutputFormat::PrSummary => {
            let apps = read_workspace_apps(ws_path);
            format_pr_summary(&paired, &resolved.workspace.routines, &apps)
        }
        OutputFormat::Json => {
            let summaries: Vec<_> = paired.iter().map(|(s, _)| s.clone()).collect();
            build_analyze_json(&JsonFormatInputs {
                findings: &summaries,
                diagnostics: &run_diagnostics,
                detector_stats: &run_detector_stats,
                coverage: &coverage,
                deterministic: args.deterministic,
                alsem_version: alsem_version(),
            })
        }
        OutputFormat::Terminal => {
            let summaries: Vec<_> = paired.iter().map(|(s, _)| s.clone()).collect();
            // group-by path: only when format==terminal AND group_by is set.
            if let Some(ref by_str) = args.group_by {
                if let Some(by) = GroupBy::from_str(by_str) {
                    format_terminal_grouped(&summaries, &coverage, by)
                } else {
                    // Invalid group_by — the CLI validates this, so treat as plain.
                    format_terminal(&summaries, &coverage, &run_diagnostics)
                }
            } else {
                format_terminal(&summaries, &coverage, &run_diagnostics)
            }
        }
        // Stubs — the actual formatters are wired in Stages A1–A3.
        // The resolved model + raw findings are available on `resolved` / `paired`
        // when those stages implement them; nothing is emitted here.
        fmt @ OutputFormat::Html => {
            return Err(stub_not_implemented(fmt).expect("stub format has a message"))
        }
    };

    // --- dependency-coverage preflight (al-sem Task 2) ---
    // F2 FIX: always evaluate AND surface pf.degraded as a stderr warning (the
    // "no silent clean" contract — al-sem index.ts:263-264). The warn is INDEPENDENT
    // of --require-dependencies; only the FAILED→exit-4 path needs that flag.
    // We return the warning message as the 3rd tuple field; the bin emits it.
    // NOTE: `coverage` was already computed above for the Json formatter.
    let pf = evaluate_preflight(
        coverage.unresolved_callsites.len(),
        &coverage.opaque_apps,
        args.require_dependencies,
    );

    // The degraded warning message — None when coverage is complete (pf.degraded false).
    // Matches al-sem: `if (pf.degraded) process.stderr.write(`al-sem: warning: ${pf.message}\n`)`.
    let stderr_warning: Option<String> = if pf.degraded {
        Some(pf.message.clone())
    } else {
        None
    };

    // --- exit-code gate (precedence: PREFLIGHT_FAILED (4) > FINDINGS (1) > CLEAN (0)). ---
    let exit_code = if pf.failed {
        exit::PREFLIGHT_FAILED
    } else {
        // computeFindingExit over the KEPT findings' severities (no fail-on ⇒ CLEAN).
        let severities: Vec<&str> = paired.iter().map(|(s, _)| s.severity.as_str()).collect();
        compute_finding_exit(&severities, args.fail_on.as_deref())
    };

    Ok((output, exit_code, stderr_warning))
}

/// The empty-output path for a fail-closed / unreadable workspace: empty findings ⇒
/// empty SARIF or the "no findings" PR-summary or a zero-findings Json envelope;
/// a clean preflight ⇒ exit CLEAN.
///
/// `ws` is the workspace root: the JSON envelope's `diagnostics` array is populated
/// with the real PROVIDER (fail-closed, remapped to `stage:"discover"`) + index
/// diagnostics for this path — al-sem's `workspace.diagnostics` (index.ts:157-173).
/// This is load-bearing: fail-closed (multi-app / id-less / unreadable `app.json`)
/// is a documented core behavior, and dropping its diagnostics would silently hide
/// WHY the model is empty. SARIF / PR-summary carry no diagnostics array, so they
/// are unaffected (parity with al-sem, whose SARIF/pr-summary likewise omit them).
///
/// `Err` is returned for stub formats (Terminal/Html) so `run_analyze` /
/// `run_analyze_with_exit` surface an obvious error — no silent pass.
pub(crate) fn empty_output_result(
    args: &AnalyzeArgs,
    version: &str,
) -> Result<(String, u8, Option<String>), String> {
    let out = match args.format {
        OutputFormat::Sarif => format_sarif(&[], &[], version),
        OutputFormat::PrSummary => format_pr_summary(&[], &[], &[]),
        OutputFormat::Json => {
            // Empty envelope: zero findings, zero stats, zero coverage — but the
            // real provider/index diagnostics (fail-closed reasons) are threaded.
            let empty_coverage = crate::engine::l3::coverage::AnalysisCoverage {
                source_units_total: 0,
                source_units_parsed: 0,
                routines_total: 0,
                routines_body_available: 0,
                routines_parse_incomplete: vec![],
                opaque_apps: vec![],
                unresolved_callsites: vec![],
                dynamic_dispatch_sites: vec![],
            };
            let ws_path = Path::new(&args.workspace);
            let diagnostics =
                crate::engine::gate::workspace_diagnostics::compute_workspace_diagnostics(ws_path);
            build_analyze_json(&JsonFormatInputs {
                findings: &[],
                diagnostics: &diagnostics,
                detector_stats: &[],
                coverage: &empty_coverage,
                deterministic: args.deterministic,
                alsem_version: alsem_version(),
            })
        }
        OutputFormat::Terminal => {
            // Empty workspace → "No findings." terminal output.
            let empty_coverage = crate::engine::l3::coverage::AnalysisCoverage {
                source_units_total: 0,
                source_units_parsed: 0,
                routines_total: 0,
                routines_body_available: 0,
                routines_parse_incomplete: vec![],
                opaque_apps: vec![],
                unresolved_callsites: vec![],
                dynamic_dispatch_sites: vec![],
            };
            let ws_path = Path::new(&args.workspace);
            let diagnostics =
                crate::engine::gate::workspace_diagnostics::compute_workspace_diagnostics(ws_path);
            format_terminal(&[], &empty_coverage, &diagnostics)
        }
        fmt @ OutputFormat::Html => {
            return Err(stub_not_implemented(fmt).expect("stub format has a message"))
        }
    };
    Ok((out, exit::CLEAN, None))
}
