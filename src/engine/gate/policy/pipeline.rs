//! Policy CLI pipeline — ports al-sem `src/cli/policy.ts` (`runPolicyCheck` /
//! `runPolicyExplain`) and the `runPolicy` wrapper in `policy-engine.ts`.
//!
//! Drives the workspace analysis (same path as `events`: assemble → resolve →
//! detector context), builds the policy model-input view, runs the engine, and
//! formats the result. Policy resolution mirrors the CLI:
//!   1. `--no-policy` → "disabled" (policyVersion 0).
//!   2. `--policy <path>` → "explicit:<abs>".
//!   3. auto-detect `al-sem.policy.yaml` / `.yml` in the workspace → "auto:<abs>".
//!   4. bundled default → "default".

use std::path::{Path, PathBuf};

use crate::engine::gate::model_instance_id::compute_gate_model_instance_id;
use crate::engine::gate::policy::format_policy::{
    format_policy_human, format_policy_json, format_policy_sarif,
};
use crate::engine::gate::policy::policy_engine::{PolicyModel, PolicyRunResult, run_policy_engine};
use crate::engine::gate::policy::policy_loader::{
    BUNDLED_DEFAULT_POLICY_YAML, LoadResult, load_policy_from_file, load_policy_from_string,
};
use crate::engine::gate::policy::policy_types::{PolicyDoc, predicate_to_json};
use crate::engine::gate::run::compute_analyzer_diagnostics;
use crate::engine::l3::l3_workspace::assemble_and_resolve_workspace;
use crate::engine::l5::detector_context::build_detector_context;
use crate::engine::l5::detectors::registered_detectors;
use crate::engine::l5::digest_cli::DEFAULT_DETECTOR_NAMES;
use crate::engine::root_classification::classify_roots;

/// How the effective policy was resolved.
enum ResolvedPolicy {
    /// A loaded policy with a `policySource` string.
    Loaded { policy: PolicyDoc, source: String },
    /// `--no-policy`: no policy, source "disabled", version 0.
    Disabled,
    /// A load error (stderr lines already prefixed).
    Error(Vec<String>),
}

/// Auto-detect / explicit / default policy resolution for `check`. Mirrors
/// `runPolicyCheck`'s resolution block.
fn resolve_policy_check(
    workspace: &Path,
    policy_path: Option<&str>,
    no_policy: bool,
) -> ResolvedPolicy {
    if no_policy {
        return ResolvedPolicy::Disabled;
    }
    if let Some(p) = policy_path {
        let abs = absolutize(p);
        return match load_policy_from_file(&abs) {
            LoadResult::Ok { policy, .. } => ResolvedPolicy::Loaded {
                policy,
                source: format!("explicit:{}", abs.display()),
            },
            LoadResult::Err { errors, .. } => ResolvedPolicy::Error(
                errors
                    .into_iter()
                    .map(|e| format!("policy load error: {e}"))
                    .collect(),
            ),
        };
    }
    // Auto-detect.
    for name in ["al-sem.policy.yaml", "al-sem.policy.yml"] {
        let candidate = workspace.join(name);
        if candidate.exists() {
            let abs = absolutize(candidate.to_string_lossy().as_ref());
            return match load_policy_from_file(&abs) {
                LoadResult::Ok { policy, .. } => ResolvedPolicy::Loaded {
                    policy,
                    source: format!("auto:{}", abs.display()),
                },
                LoadResult::Err { errors, .. } => ResolvedPolicy::Error(
                    errors
                        .into_iter()
                        .map(|e| format!("policy load error: {e}"))
                        .collect(),
                ),
            };
        }
    }
    // Bundled default (embedded — byte-identical to the vendored yaml).
    match load_policy_from_string(BUNDLED_DEFAULT_POLICY_YAML) {
        LoadResult::Ok { policy, .. } => ResolvedPolicy::Loaded {
            policy,
            source: "default".to_string(),
        },
        LoadResult::Err { errors, .. } => ResolvedPolicy::Error(
            errors
                .into_iter()
                .map(|e| format!("bundled default policy load error: {e}"))
                .collect(),
        ),
    }
}

/// Resolve an absolute path (al-sem `resolve(path)` semantics — relative to cwd).
fn absolutize(p: &str) -> PathBuf {
    let path = Path::new(p);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

/// The outcome of a `policy check` run (stdout text + exit + stderr lines).
pub struct PolicyCheckOutcome {
    pub text: Option<String>,
    pub exit_code: u8,
    pub stderr_lines: Vec<String>,
    /// When set, the text should be written to this `--out` path instead of stdout.
    pub out_path: Option<String>,
}

/// Options for `run_policy_check`.
pub struct PolicyCheckOptions<'a> {
    pub workspace: &'a Path,
    pub policy_path: Option<&'a str>,
    pub no_policy: bool,
    pub format: &'a str, // "human" | "json" | "sarif"
    pub out: Option<&'a str>,
    pub deterministic: bool,
    pub strict: bool,
    pub alsem_version: &'a str,
}

/// `runPolicyCheck`. Exit: 0 on success ALWAYS (no fail-on gate); non-zero only on
/// invalid format / strict analyzer error / policy-load error / write failure.
pub fn run_policy_check(opts: &PolicyCheckOptions) -> PolicyCheckOutcome {
    // Build the model (assemble → resolve → detector context).
    let model_id = match compute_gate_model_instance_id(opts.workspace) {
        Some(id) => id,
        None => {
            return PolicyCheckOutcome {
                text: None,
                exit_code: 1,
                stderr_lines: vec![
                    "al-sem policy check: could not compute modelInstanceId".to_string(),
                ],
                out_path: None,
            };
        }
    };
    let resolved = match assemble_and_resolve_workspace(opts.workspace, &model_id) {
        Some(r) => r,
        None => {
            return PolicyCheckOutcome {
                text: None,
                exit_code: 1,
                stderr_lines: vec!["al-sem policy check: workspace did not resolve".to_string()],
                out_path: None,
            };
        }
    };

    // analyzeWorkspace diagnostics (run the default detector set so detector-stage
    // diagnostics surface exactly as al-sem does).
    let diag_lines = analyze_workspace_diagnostic_lines(opts.workspace, &resolved);

    // --strict: any error-severity diagnostic → exit 1 (stderr the diagnostics).
    if opts.strict && diag_lines.iter().any(|l| l.starts_with("error:")) {
        return PolicyCheckOutcome {
            text: None,
            exit_code: 1,
            stderr_lines: diag_lines,
            out_path: None,
        };
    }

    // Resolve the effective policy.
    let (policy_opt, source): (Option<PolicyDoc>, String) =
        match resolve_policy_check(opts.workspace, opts.policy_path, opts.no_policy) {
            ResolvedPolicy::Loaded { policy, source } => (Some(policy), source),
            ResolvedPolicy::Disabled => (None, "disabled".to_string()),
            ResolvedPolicy::Error(lines) => {
                return PolicyCheckOutcome {
                    text: None,
                    exit_code: 1,
                    stderr_lines: lines,
                    out_path: None,
                };
            }
        };

    // Run the engine (runPolicy wrapper: disabled → empty result, version 0).
    let result: PolicyRunResult = match &policy_opt {
        None => PolicyRunResult {
            policy_source: source,
            policy_version: 0,
            rule_summaries: Vec::new(),
            findings: Vec::new(),
            diagnostics: Vec::new(),
        },
        Some(policy) => {
            let ctx = build_detector_context(&resolved);
            let root_classifications = classify_roots(&resolved.workspace);
            let ws = &resolved.workspace;
            let model = PolicyModel::new(
                &ws.routines,
                &ws.objects,
                &ws.tables,
                &ctx.event_graph.events,
                &root_classifications,
                &ctx.summaries,
            );
            let out = run_policy_engine(&model, policy);
            PolicyRunResult {
                policy_source: source,
                policy_version: policy.version,
                rule_summaries: out.rule_summaries,
                findings: out.findings,
                diagnostics: out.diagnostics,
            }
        }
    };

    let text = match opts.format {
        "json" => format_policy_json(&result, opts.alsem_version, opts.deterministic),
        "sarif" => format_policy_sarif(&result, opts.alsem_version),
        _ => format_policy_human(&result),
    };

    // Exit 0 always; stderr = analyzer diagnostics (after writing output).
    PolicyCheckOutcome {
        text: Some(text),
        exit_code: 0,
        stderr_lines: diag_lines,
        out_path: opts.out.map(|s| s.to_string()),
    }
}

/// `analyzeWorkspace`-equivalent diagnostic lines (mirrors events.rs).
fn analyze_workspace_diagnostic_lines(
    workspace: &Path,
    resolved: &crate::engine::l3::l3_workspace::L3Resolved,
) -> Vec<String> {
    let default_detectors: Vec<_> = registered_detectors()
        .into_iter()
        .filter(|d| DEFAULT_DETECTOR_NAMES.contains(&d.name.as_str()))
        .collect();
    compute_analyzer_diagnostics(workspace, resolved, &default_detectors)
        .iter()
        .map(|d| format!("{}: {}", d.severity, d.message))
        .collect()
}

// ---------------------------------------------------------------------------
// policy explain
// ---------------------------------------------------------------------------

/// The outcome of `policy explain`.
pub struct PolicyExplainOutcome {
    pub stdout: Option<String>,
    pub exit_code: u8,
    pub stderr_lines: Vec<String>,
}

/// Options for `run_policy_explain`.
pub struct PolicyExplainOptions<'a> {
    pub workspace: &'a Path,
    pub rule_id: &'a str,
    pub policy_path: Option<&'a str>,
}

/// `runPolicyExplain`. `--format json` is IGNORED (always human). Resolution mirrors
/// `runPolicyCheck` MINUS `--no-policy` (explain has no disabled mode).
pub fn run_policy_explain(opts: &PolicyExplainOptions) -> PolicyExplainOutcome {
    // Resolve policy (explicit → auto → default).
    let (policy, source): (PolicyDoc, String) = if let Some(p) = opts.policy_path {
        let abs = absolutize(p);
        match load_policy_from_file(&abs) {
            LoadResult::Ok { policy, .. } => (policy, format!("explicit:{}", abs.display())),
            LoadResult::Err { errors, .. } => {
                return PolicyExplainOutcome {
                    stdout: None,
                    exit_code: 1,
                    stderr_lines: errors
                        .into_iter()
                        .map(|e| format!("policy load error: {e}"))
                        .collect(),
                };
            }
        }
    } else {
        // Auto-detect.
        let mut found: Option<(PolicyDoc, String)> = None;
        for name in ["al-sem.policy.yaml", "al-sem.policy.yml"] {
            let candidate = opts.workspace.join(name);
            if candidate.exists() {
                let abs = absolutize(candidate.to_string_lossy().as_ref());
                match load_policy_from_file(&abs) {
                    LoadResult::Ok { policy, .. } => {
                        found = Some((policy, format!("auto:{}", abs.display())));
                        break;
                    }
                    LoadResult::Err { errors, .. } => {
                        return PolicyExplainOutcome {
                            stdout: None,
                            exit_code: 1,
                            stderr_lines: errors
                                .into_iter()
                                .map(|e| format!("policy load error: {e}"))
                                .collect(),
                        };
                    }
                }
            }
        }
        match found {
            Some(x) => x,
            None => match load_policy_from_string(BUNDLED_DEFAULT_POLICY_YAML) {
                LoadResult::Ok { policy, .. } => (policy, "default".to_string()),
                LoadResult::Err { errors, .. } => {
                    return PolicyExplainOutcome {
                        stdout: None,
                        exit_code: 1,
                        stderr_lines: errors
                            .into_iter()
                            .map(|e| format!("bundled default policy load error: {e}"))
                            .collect(),
                    };
                }
            },
        }
    };

    // Find the rule.
    let Some(rule) = policy.rules.iter().find(|r| r.id == opts.rule_id) else {
        return PolicyExplainOutcome {
            stdout: None,
            exit_code: 1,
            stderr_lines: vec![format!(
                "al-sem policy explain: rule '{}' not found in effective policy ({})",
                opts.rule_id, source
            )],
        };
    };

    // Render the rule-level summary + Normalized AST block.
    let coverage = rule
        .require_coverage
        .as_deref()
        .or_else(|| {
            policy
                .defaults
                .as_ref()
                .and_then(|d| d.require_coverage.as_deref())
        })
        .unwrap_or("any");
    let on_unknown = rule
        .on_unknown
        .as_deref()
        .or_else(|| {
            policy
                .defaults
                .as_ref()
                .and_then(|d| d.on_unknown.as_deref())
        })
        .unwrap_or("fail-closed");

    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("Rule: {}", rule.id));
    if let Some(t) = &rule.title {
        lines.push(format!("Title: {t}"));
    }
    lines.push(format!("Severity: {}", rule.severity));
    lines.push(format!("Coverage gate: {coverage}"));
    lines.push(format!("On unknown: {on_unknown}"));
    lines.push(format!("Effective policy: {source}"));
    lines.push(String::new());
    lines.push("Normalized AST:".to_string());
    lines.push(predicate_to_json(&rule.when));
    if let Some(except) = &rule.except {
        lines.push("Except:".to_string());
        lines.push(predicate_to_json(except));
    }

    PolicyExplainOutcome {
        stdout: Some(format!("{}\n", lines.join("\n"))),
        exit_code: 0,
        stderr_lines: Vec::new(),
    }
}
