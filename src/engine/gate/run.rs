//! `run_analyze` — the gate pipeline lib entry the `alsem analyze` bin wraps and the
//! differential test calls in-process. Mirrors the `analyze` action in al-sem
//! `src/cli/index.ts` (Stage 1: NO baseline, NO inline suppression — those are Stage 3).
//!
//! Pipeline:
//!   assemble_and_resolve_workspace_default(ws)  (L0→L3, source-only)
//!   → resolve the detector set (preset | --detector | default)
//!   → run_detectors (L4 inside the DetectorContext + L5) — pre-sorted Finding[]
//!   → project_finding per finding (display names + 1-based location)
//!   → filter_findings (min-severity, detector allow-list)
//!   → scope filter (primary drops dependency-anchored findings)
//!   → limit
//!   → format_sarif
//!
//! Source-only: the transaction-integrity preset is intra-app, so this drives the
//! source-only `run_detectors` (not the cross-app variant). Every workspace object is
//! "primary", so the scope=primary filter keeps everything.

use std::path::Path;

use crate::engine::gate::filter::{filter_findings, scope_filter, FilterOptions, Scope};
use crate::engine::gate::format_sarif::format_sarif;
use crate::engine::gate::model_instance_id::compute_gate_model_instance_id;
use crate::engine::gate::presets::resolve_analyze_detectors;
use crate::engine::gate::projection::{project_finding, ProjectionIndex};
use crate::engine::l3::l3_workspace::assemble_and_resolve_workspace;
use crate::engine::l5::registry::run_detectors;

/// Output format. Stage 1: only `Sarif`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Sarif,
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
    /// `--format` (Stage 1: Sarif only).
    pub format: OutputFormat,
    /// `--sarif-version-override` — pins `driver.version` for byte-stable output.
    /// When `None`, `default_version` is used.
    pub sarif_version_override: Option<String>,
}

/// Run the gate `analyze` pipeline and return the SARIF string WITHOUT the trailing
/// newline (the CLI / caller appends `"\n"` to match al-sem's
/// `process.stdout.write(`${formatSarif(...)}\n`)`).
///
/// `default_version` is the engine's real version, used when no override is given.
///
/// Errors: detector/preset resolution failures (e.g. unknown preset, `--preset` +
/// `--detector` together). A workspace that fails to assemble (fail-closed / unreadable)
/// yields an EMPTY-results SARIF — engine-never-throws.
pub fn run_analyze(args: &AnalyzeArgs, default_version: &str) -> Result<String, String> {
    let detectors = resolve_analyze_detectors(args.preset.as_deref(), args.detector.as_deref())?;

    let version = args
        .sarif_version_override
        .clone()
        .unwrap_or_else(|| default_version.to_string());

    // Assemble with the al-sem GATE modelInstanceId (content-derived, UNPINNED) so the
    // internal RoutineIds embedded in each finding's rootCauseKey — and therefore the
    // SARIF fingerprint hashed over them — byte-match the al-sem `analyze` CLI goldens.
    // (The R4 dump pins "r0"; the gate does not — see model_instance_id.rs.)
    let ws_path = Path::new(&args.workspace);
    let model_instance_id = match compute_gate_model_instance_id(ws_path) {
        Some(id) => id,
        // Fail-closed layout → empty SARIF (no findings).
        None => return Ok(format_sarif(&[], &[], &version)),
    };
    let resolved = match assemble_and_resolve_workspace(ws_path, &model_instance_id) {
        Some(r) => r,
        // Fail-closed / unreadable workspace → empty SARIF (no findings).
        None => return Ok(format_sarif(&[], &[], &version)),
    };

    // L4 + L5: run the selected detectors. Findings come pre-sorted by
    // (detector, primaryLocationKey, rootCauseKey) with dep-anchored findings already
    // role-scoped out (source-only ⇒ no-op).
    let run = run_detectors(&resolved, &detectors);

    // Project each finding (display names + 1-based location), preserving order.
    let idx = ProjectionIndex::build(&resolved.workspace.objects, &resolved.workspace.routines);
    // Keep the raw Finding alongside its summary so the SARIF can emit evidence paths.
    let mut paired: Vec<(
        crate::engine::gate::projection::FindingSummary,
        &crate::engine::l5::finding::Finding,
    )> = run
        .findings
        .iter()
        .map(|f| (project_finding(f, &idx), f))
        .collect();

    // The filter / scope / limit steps are defined over `FindingSummary` (mirroring
    // al-sem). We run each over the summaries-only view, then prune `paired` to the
    // surviving prefix/subset by replaying the SAME predicates on the pairs — keeping
    // each `(summary, raw)` pair aligned. Finding `id`s are globally unique (the id
    // embeds the detector + location), so membership tests are unambiguous.

    // --- filter: min-severity, then detector allow-list. The al-sem CLI only passes a
    // detector allow-list to filter_findings when `--detector` is set (the preset bakes
    // the set into analyzeWorkspace instead). Mirror that exactly. ---
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

    // --- scope: primary drops dependency-anchored findings. Source-only ⇒ no object is
    // a dependency, so this keeps everything. ---
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

    let summaries: Vec<crate::engine::gate::projection::FindingSummary> =
        paired.iter().map(|(s, _)| s.clone()).collect();
    let raws: Vec<&crate::engine::l5::finding::Finding> = paired.iter().map(|(_, r)| *r).collect();

    match args.format {
        OutputFormat::Sarif => Ok(format_sarif(&summaries, &raws, &version)),
    }
}
