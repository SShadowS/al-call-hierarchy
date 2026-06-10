//! `alsem` — the future al-sem production CLI (the Rust port). Stage 1 ships the
//! `analyze` GATE path: `analyze <ws> --preset <name> --format sarif` byte-matches
//! the al-sem TS CLI's SARIF gate goldens.
//!
//! This is a NEW bin, separate from `aldump` (the R0 differential producer). It does
//! NOT touch the LSP shipping code.
//!
//! Output discipline: SARIF goes to stdout; errors go to stderr. The SARIF is written
//! followed by a trailing newline, matching al-sem's
//! `process.stdout.write(`${formatSarif(...)}\n`)`.

use std::io::IsTerminal;
use std::process::ExitCode;

use al_call_hierarchy::engine::gate::exit_code::{exit, parse_fail_on};
use al_call_hierarchy::engine::gate::filter::Scope;
use al_call_hierarchy::engine::gate::presets::PRESET_NAMES_LIST;
use al_call_hierarchy::engine::gate::run::{run_analyze_with_exit, AnalyzeArgs, OutputFormat};
use al_call_hierarchy::engine::gate::version::DEFAULT_ALSEM_VERSION;
use al_call_hierarchy::engine::l5::digest_cli::{
    auto_detect_changed, run_digest_pipeline, ChangedAutoDetect,
};
use al_call_hierarchy::engine::l5::prove::{parse_question, question_ids, run_prove_pipeline};
use clap::{Parser, Subcommand};

/// The engine's default (unpinned) SARIF `driver.version`. The differential always
/// pins via `--sarif-version-override gate-sarif-v1`; this is only the fallback for a
/// real, unpinned invocation.
const DEFAULT_SARIF_VERSION: &str = DEFAULT_ALSEM_VERSION;

const SEVERITY_VALUES: &[&str] = &["critical", "high", "medium", "low", "info"];

#[derive(Parser)]
#[command(
    name = "al-sem",
    about = "Static semantic analysis engine for Microsoft Business Central AL code (Rust port)",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Analyze an AL workspace and emit findings (Stage 1: --format sarif).
    Analyze(AnalyzeCli),
    /// Digest an AL workspace: changed-root effect summary (cli-b/b1).
    Digest(DigestCli),
    /// Prove an absence-safety question about a single routine (cli-b/b2).
    Prove(ProveCli),
}

#[derive(Parser)]
struct AnalyzeCli {
    /// Path to the AL workspace root.
    workspace: String,

    /// Path to the .alpackages directory (reserved; source-only Stage 1).
    #[arg(long)]
    alpackages: Option<String>,

    /// Pin timestamps / version for byte-stable output (reserved; SARIF is already stable).
    #[arg(long, default_value_t = false)]
    deterministic: bool,

    /// Drop findings below this severity: critical|high|medium|low|info.
    #[arg(long = "min-severity")]
    min_severity: Option<String>,

    /// Comma-separated allow-list of detector ids. Mutually exclusive with --preset.
    #[arg(long = "detector")]
    detector: Option<String>,

    /// Run a named detector bundle (e.g. transaction-integrity).
    #[arg(long = "preset")]
    preset: Option<String>,

    /// primary (default) drops findings anchored in a dependency; all keeps them.
    #[arg(long = "scope", default_value = "primary")]
    scope: String,

    /// Cap output at the first N findings (after filtering).
    #[arg(long = "limit")]
    limit: Option<usize>,

    /// Output format: sarif | pr-summary | terminal | json | html | auto.
    /// `auto` resolves to `terminal` when stdout is a TTY, else `json`.
    #[arg(long = "format", default_value = "auto")]
    format: String,

    /// Pin the SARIF driver.version (e.g. gate-sarif-v1) for byte-stable output.
    #[arg(long = "sarif-version-override")]
    sarif_version_override: Option<String>,

    /// Exit 1 if any kept finding is at/above this severity: critical|high|medium|low|info.
    #[arg(long = "fail-on")]
    fail_on: Option<String>,

    /// Make a degraded dependency-coverage preflight FAIL (exit 4).
    #[arg(long = "require-dependencies", default_value_t = false)]
    require_dependencies: bool,

    /// Path to a baseline file: findings whose fingerprint is listed are suppressed.
    #[arg(long = "baseline")]
    baseline: Option<String>,

    /// Rewrite the --baseline file from the current findings (the new floor), then apply it.
    #[arg(long = "update-baseline", default_value_t = false)]
    update_baseline: bool,

    /// Group terminal output by: object | routine | table | detector | file.
    /// Only acted on when the resolved format is `terminal`.
    #[arg(long = "group-by")]
    group_by: Option<String>,

    /// Not supported: the full-model JSON dump is not ported to the Rust engine.
    /// Use the TS CLI (`al-sem analyze ... --dump-model`) for full-model debug dumps.
    #[arg(long = "dump-model", default_value_t = false, hide = true)]
    dump_model: bool,
}

/// `ProveCli` — arguments for `alsem prove <ws> <routine> <question>`.
#[derive(Parser)]
struct ProveCli {
    /// Path to the AL workspace root.
    workspace: String,

    /// Routine selector: display name or StableRoutineId.
    routine: String,

    /// Prove question: may-commit | commits-on-success-path | writes-table:<name>
    ///   | publishes-event:<name> | reaches-ui | throws-error.
    question: String,

    /// Output format: json | human. Defaults to json.
    #[arg(long = "format", default_value = "json")]
    format: String,

    /// Write output to a file instead of stdout.
    #[arg(long = "out")]
    out: Option<String>,

    /// Pin timestamps / version for byte-stable output.
    #[arg(long, default_value_t = false)]
    deterministic: bool,

    /// Skip loading roots.config.json (pass the workspace as-is).
    #[arg(long = "no-roots-config", default_value_t = false)]
    no_roots_config: bool,

    /// Path to the .alpackages directory.
    #[arg(long = "alpackages")]
    alpackages: Option<String>,
}

/// `DigestCli` — arguments for `alsem digest <ws>`.
#[derive(Parser)]
struct DigestCli {
    /// Path to the AL workspace root.
    workspace: String,

    /// Workspace-relative source file(s) to treat as changed.
    /// May be repeated. Mutually exclusive with --order (rejected).
    #[arg(long = "file", action = clap::ArgAction::Append)]
    file: Vec<String>,

    /// Routine stable IDs or display names to treat as changed roots.
    #[arg(long = "routine", action = clap::ArgAction::Append)]
    routine: Vec<String>,

    /// Read a unified diff from a file (or `-` for stdin) to resolve changed roots.
    #[arg(long = "diff")]
    diff: Option<String>,

    /// Convenience alias — auto-detect: existing file path → --diff; comma-list with
    /// `.al` entries → --changed-files; else → --changed-routines.
    #[arg(long = "changed")]
    changed: Option<String>,

    /// Maximum number of via-paths per effect (reserved; wired but not yet honoured).
    #[arg(long = "max-paths")]
    max_paths: Option<usize>,

    /// Output format: json | human. Defaults to json.
    #[arg(long = "format", default_value = "json")]
    format: String,

    /// Write output to a file instead of stdout.
    #[arg(long = "out")]
    out: Option<String>,

    /// Pin timestamps / version for byte-stable output.
    #[arg(long, default_value_t = false)]
    deterministic: bool,

    /// NOT SUPPORTED: the digest command does not support ordering output.
    #[arg(long, default_value_t = false, hide = true)]
    order: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Commands::Analyze(a) => run_analyze_cmd(a),
        Commands::Digest(d) => run_digest_cmd(d),
        Commands::Prove(p) => run_prove_cmd(p),
    }
}

const GROUP_BY_VALUES: &[&str] = &["object", "routine", "table", "detector", "file"];

// ── digest command ──────────────────────────────────────────────────────────

/// The exact stderr message emitted when `--order` is used with `digest`.
const ORDER_REJECTION: &str =
    "al-sem: digest --order is not supported by the Rust engine; use the TS CLI for ordered digests";

fn run_digest_cmd(d: DigestCli) -> ExitCode {
    // Reject --order (CONFIG_ERROR, matching --dump-model pattern)
    if d.order {
        eprintln!("{ORDER_REJECTION}");
        return ExitCode::from(exit::CONFIG_ERROR);
    }

    // Validate --format. al-sem (cli/index.ts:578) writes this exact message and
    // exits 1 (FINDINGS), NOT CONFIG_ERROR (3) — the digest format check is at the
    // CLI layer, distinct from analyze's enum-flag CONFIG_ERROR exits (#4).
    const VALID_DIGEST_FORMATS: &[&str] = &["json", "human"];
    if !VALID_DIGEST_FORMATS.contains(&d.format.as_str()) {
        eprintln!(
            "al-sem digest: invalid --format '{}'. Expected: json | human",
            d.format
        );
        return ExitCode::from(1);
    }

    // Resolve the --changed alias FIRST (auto-detect into diff / files / routines),
    // mirroring cli/index.ts. The detected values are merged with explicit flags.
    let mut file_inputs: Vec<String> = d.file.clone();
    let mut routine_inputs: Vec<String> = d.routine.clone();
    let mut diff_arg: Option<String> = d.diff.clone();
    if let Some(changed) = d.changed.as_deref() {
        if !changed.trim().is_empty() {
            match auto_detect_changed(changed) {
                ChangedAutoDetect::Diff(p) => diff_arg = Some(p),
                ChangedAutoDetect::Files(fs) => file_inputs.extend(fs),
                ChangedAutoDetect::Routines(rs) => routine_inputs.extend(rs),
            }
        }
    }

    // No-input check (cli/digest.ts:138): emit the exact CLI message + exit 1.
    // Done at the CLI layer BEFORE the pipeline so the user sees the --changed-files
    // wording (the pipeline's own message is the internal run-digest-pipeline one).
    let has_files = !file_inputs.is_empty();
    let has_routines = !routine_inputs.is_empty();
    let has_diff = diff_arg
        .as_deref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    if !has_files && !has_routines && !has_diff {
        eprintln!(
            "digest: at least one of --changed-files, --changed-routines, --diff, or --changed is required"
        );
        return ExitCode::from(1);
    }

    // Read diff input
    let diff_text: Option<String> = match diff_arg.as_deref() {
        None => None,
        Some("-") => {
            // stdin: buffer all of it
            use std::io::Read;
            let mut buf = String::new();
            if std::io::stdin().read_to_string(&mut buf).is_err() {
                eprintln!("al-sem: digest: failed to read diff from stdin");
                return ExitCode::from(exit::CONFIG_ERROR);
            }
            Some(buf)
        }
        Some(path) => match std::fs::read_to_string(path) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("al-sem: digest: could not read diff file '{path}': {e}");
                return ExitCode::from(exit::CONFIG_ERROR);
            }
        },
    };

    let changed_files = if file_inputs.is_empty() {
        None
    } else {
        Some(file_inputs)
    };
    let changed_routines = if routine_inputs.is_empty() {
        None
    } else {
        Some(routine_inputs)
    };

    let workspace = std::path::Path::new(&d.workspace);

    match run_digest_pipeline(
        workspace,
        changed_files,
        changed_routines,
        diff_text,
        DEFAULT_ALSEM_VERSION,
        d.deterministic,
        d.max_paths,
    ) {
        Err(msg) => {
            // A DigestPipelineError (input/analyze phase) → write the message verbatim
            // and exit 1, mirroring cli/digest.ts's catch. The message already starts
            // with "digest:" so no prefix is added (#3).
            eprintln!("{msg}");
            ExitCode::from(1)
        }
        Ok(result) => {
            let output = if d.format == "human" {
                result.human_text
            } else {
                result.json_text
            };

            // Write output
            let write_result = if let Some(ref out_path) = d.out {
                std::fs::write(out_path, &output).map_err(|e| format!("{e}"))
            } else {
                use std::io::Write;
                std::io::stdout()
                    .write_all(output.as_bytes())
                    .map_err(|e| format!("{e}"))
            };

            if let Err(e) = write_result {
                eprintln!("al-sem: digest: write error: {e}");
                return ExitCode::from(1);
            }

            ExitCode::from(result.exit_code)
        }
    }
}

// ── prove command ───────────────────────────────────────────────────────────

fn run_prove_cmd(p: ProveCli) -> ExitCode {
    // Validate --format
    const VALID_PROVE_FORMATS: &[&str] = &["json", "human"];
    if !VALID_PROVE_FORMATS.contains(&p.format.as_str()) {
        eprintln!(
            "al-sem prove: invalid --format '{}'. Expected: json | human",
            p.format
        );
        return ExitCode::from(1);
    }

    // Validate question early (mirrors prove.ts exit 1 for unknown question)
    if parse_question(&p.question).is_none() {
        let valid: Vec<&str> = question_ids().to_vec();
        eprintln!(
            "prove: unknown question '{}'\nValid questions:\n{}",
            p.question,
            valid
                .iter()
                .map(|q| format!("  {q}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        return ExitCode::from(1);
    }

    let workspace = std::path::Path::new(&p.workspace);

    match run_prove_pipeline(
        workspace,
        &p.routine,
        &p.question,
        DEFAULT_ALSEM_VERSION,
        p.deterministic,
    ) {
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::from(1)
        }
        Ok(result) => {
            let output = if p.format == "human" {
                result.human_text
            } else {
                result.json_text
            };

            let write_result = if let Some(ref out_path) = p.out {
                std::fs::write(out_path, &output).map_err(|e| format!("{e}"))
            } else {
                use std::io::Write;
                std::io::stdout()
                    .write_all(output.as_bytes())
                    .map_err(|e| format!("{e}"))
            };

            if let Err(e) = write_result {
                eprintln!("al-sem: prove: write error: {e}");
                return ExitCode::from(1);
            }

            ExitCode::from(result.exit_code)
        }
    }
}

/// The exact stderr message emitted when `--dump-model` is used. The full-model JSON
/// dump (>500MB) is an intentional, documented out-of-scope divergence — the Rust
/// engine rejects it rather than porting it.
const DUMP_MODEL_REJECTION: &str =
    "al-sem: --dump-model is not supported by the Rust engine; use the TS CLI for full-model debug dumps";

/// Resolve `--format auto` (or omitted) to a concrete `OutputFormat`.
/// Non-TTY stdout → `Json`; TTY stdout → `Terminal`.
/// This is the testable contract (corpus differentials always pipe → non-TTY → json).
fn resolve_auto_format(format_str: &str) -> OutputFormat {
    match format_str {
        "auto" => {
            if std::io::stdout().is_terminal() {
                OutputFormat::Terminal
            } else {
                OutputFormat::Json
            }
        }
        "sarif" => OutputFormat::Sarif,
        "pr-summary" => OutputFormat::PrSummary,
        "terminal" => OutputFormat::Terminal,
        "json" => OutputFormat::Json,
        "html" => OutputFormat::Html,
        // Unknown values are caught before this call; return Json as a safe fallback.
        _ => OutputFormat::Json,
    }
}

/// The `--dump-model` rejection decision: `Some((message, exit_code))` when the flag
/// is set, `None` otherwise. Pure helper so the (message, exit) contract is unit-testable
/// without driving the full `run_analyze_cmd` / capturing stderr.
fn dump_model_rejection(dump_model: bool) -> Option<(&'static str, u8)> {
    if dump_model {
        Some((DUMP_MODEL_REJECTION, exit::CONFIG_ERROR))
    } else {
        None
    }
}

fn run_analyze_cmd(a: AnalyzeCli) -> ExitCode {
    // --- --dump-model: intentional not-ported rejection ---
    if let Some((msg, code)) = dump_model_rejection(a.dump_model) {
        eprintln!("{msg}");
        return ExitCode::from(code);
    }

    // --- enum-flag validation (mirrors the al-sem CLI's CONFIG_ERROR exits) ---
    if let Some(sev) = &a.min_severity {
        if !SEVERITY_VALUES.contains(&sev.as_str()) {
            eprintln!(
                "al-sem: invalid --min-severity '{sev}'. Expected one of: {}",
                SEVERITY_VALUES.join(", ")
            );
            return ExitCode::from(3);
        }
    }

    let scope = match a.scope.as_str() {
        "primary" => Scope::Primary,
        "all" => Scope::All,
        other => {
            eprintln!("al-sem: invalid --scope '{other}'. Expected one of: primary, all");
            return ExitCode::from(3);
        }
    };

    // --- validate --format (accept auto + all concrete values) ---
    const VALID_FORMATS: &[&str] = &["auto", "sarif", "pr-summary", "terminal", "json", "html"];
    if !VALID_FORMATS.contains(&a.format.as_str()) {
        eprintln!(
            "al-sem: unsupported --format '{}'. Supported: {} (known presets: {})",
            a.format,
            VALID_FORMATS.join(", "),
            PRESET_NAMES_LIST.join(", ")
        );
        return ExitCode::from(3);
    }
    let format = resolve_auto_format(&a.format);

    // --- validate --group-by ---
    let group_by = if let Some(ref g) = a.group_by {
        if !GROUP_BY_VALUES.contains(&g.as_str()) {
            eprintln!(
                "al-sem: invalid --group-by '{g}'. Expected one of: {}",
                GROUP_BY_VALUES.join(", ")
            );
            return ExitCode::from(3);
        }
        Some(g.clone())
    } else {
        None
    };

    // --- validate --fail-on (CONFIG_ERROR on a bad value, mirroring al-sem parseFailOn). ---
    let fail_on = match &a.fail_on {
        Some(s) => match parse_fail_on(s) {
            Ok(sev) => Some(sev),
            Err(msg) => {
                eprintln!("al-sem: {msg}");
                return ExitCode::from(3);
            }
        },
        None => None,
    };

    let args = AnalyzeArgs {
        workspace: a.workspace,
        min_severity: a.min_severity,
        detector: a.detector,
        preset: a.preset,
        scope,
        limit: a.limit,
        format,
        sarif_version_override: a.sarif_version_override,
        fail_on,
        require_dependencies: a.require_dependencies,
        baseline: a.baseline,
        update_baseline: a.update_baseline,
        disable_inline_suppression: false,
        group_by,
        deterministic: a.deterministic,
    };

    match run_analyze_with_exit(&args, DEFAULT_SARIF_VERSION) {
        Ok((out, exit_code, stderr_warning)) => {
            // F2: emit the preflight degraded warning to stderr (the "no silent clean"
            // contract). Matches al-sem index.ts:263-264:
            //   `if (pf.degraded) process.stderr.write(`al-sem: warning: ${pf.message}\n`)`.
            if let Some(msg) = stderr_warning {
                eprintln!("al-sem: warning: {msg}");
            }
            // al-sem appends a trailing newline to the formatted output.
            println!("{out}");
            ExitCode::from(exit_code)
        }
        Err(msg) => {
            // "analysis failure — …" is tagged by run.rs for errors that al-sem maps to
            // EXIT.ANALYSIS_FAILURE (2) — e.g. a malformed baseline file that would throw
            // inside al-sem's loadBaseline and be caught by the analyze-action catch block.
            // All other Err strings are config/usage errors → EXIT.CONFIG_ERROR (3).
            if msg.starts_with("analysis failure — ") {
                eprintln!("al-sem: {msg}");
                ExitCode::from(2)
            } else {
                eprintln!("al-sem: {msg}");
                ExitCode::from(3)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Non-TTY stdout (piped) resolves `auto` → `Json`. This is the corpus-differential
    /// contract: differentials always pipe, so they always get JSON under `--format auto`.
    /// The test runner's stdout is non-TTY, so `resolve_auto_format("auto")` exercises the
    /// real `is_terminal() == false` branch here.
    #[test]
    fn auto_format_non_tty_resolves_to_json() {
        // The load-bearing assertion: `auto` under non-TTY stdout (the test harness) → Json.
        assert_eq!(resolve_auto_format("auto"), OutputFormat::Json);
        // The explicit-string passthroughs.
        assert_eq!(resolve_auto_format("json"), OutputFormat::Json);
        assert_eq!(resolve_auto_format("terminal"), OutputFormat::Terminal);
        assert_eq!(resolve_auto_format("sarif"), OutputFormat::Sarif);
        assert_eq!(resolve_auto_format("pr-summary"), OutputFormat::PrSummary);
        assert_eq!(resolve_auto_format("html"), OutputFormat::Html);
    }

    /// `--dump-model` rejection: the exact stderr message AND CONFIG_ERROR (3).
    #[test]
    fn dump_model_is_rejected_with_exact_message_and_config_error() {
        // dump_model = false → no rejection.
        assert_eq!(dump_model_rejection(false), None);
        // dump_model = true → exact message + CONFIG_ERROR (3).
        let (msg, code) = dump_model_rejection(true).expect("dump-model must be rejected");
        assert_eq!(
            msg,
            "al-sem: --dump-model is not supported by the Rust engine; \
             use the TS CLI for full-model debug dumps"
        );
        assert_eq!(code, exit::CONFIG_ERROR);
        assert_eq!(code, 3);
    }

    /// All production formats (Terminal/Html) return `Ok` from the pipeline. We drive a
    /// REAL fixture workspace so the primary format-match arm is exercised (NOT the
    /// fail-closed empty_output path).
    /// Json is implemented (Stage A2), Terminal (Stage A3), Html (Stage A4).
    #[test]
    fn stub_formats_return_err() {
        use al_call_hierarchy::engine::gate::filter::Scope;
        use al_call_hierarchy::engine::gate::run::{run_analyze_with_exit, AnalyzeArgs};
        use std::path::PathBuf;

        // A real, resolvable workspace fixture (the SARIF/pr-summary differentials use it).
        let ws = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("r0-corpus")
            .join("ws-d8-commit-in-tx");
        assert!(
            ws.is_dir(),
            "fixture ws-d8-commit-in-tx missing at {} (offline corpus incomplete)",
            ws.display()
        );

        // Terminal is implemented (Stage A3) — must succeed.
        {
            let args = AnalyzeArgs {
                workspace: ws.to_string_lossy().to_string(),
                min_severity: None,
                detector: None,
                preset: Some("transaction-integrity".to_string()),
                scope: Scope::Primary,
                limit: None,
                format: OutputFormat::Terminal,
                sarif_version_override: None,
                fail_on: None,
                require_dependencies: false,
                baseline: None,
                update_baseline: false,
                disable_inline_suppression: false,
                group_by: None,
                deterministic: false,
            };
            let result = run_analyze_with_exit(&args, "test");
            assert!(
                result.is_ok(),
                "Terminal format must succeed (Stage A3 implemented); got: {result:?}"
            );
        }
        // Html is implemented (Stage A4) — must succeed.
        {
            let args = AnalyzeArgs {
                workspace: ws.to_string_lossy().to_string(),
                min_severity: None,
                detector: None,
                preset: Some("transaction-integrity".to_string()),
                scope: Scope::Primary,
                limit: None,
                format: OutputFormat::Html,
                sarif_version_override: None,
                fail_on: None,
                require_dependencies: false,
                baseline: None,
                update_baseline: false,
                disable_inline_suppression: false,
                group_by: None,
                deterministic: false,
            };
            let result = run_analyze_with_exit(&args, "test");
            assert!(
                result.is_ok(),
                "Html format must succeed (Stage A4 implemented); got: {result:?}"
            );
        }
    }
}
