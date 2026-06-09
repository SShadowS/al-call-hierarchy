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

use std::process::ExitCode;

use al_call_hierarchy::engine::gate::filter::Scope;
use al_call_hierarchy::engine::gate::presets::PRESET_NAMES_LIST;
use al_call_hierarchy::engine::gate::run::{run_analyze, AnalyzeArgs, OutputFormat};
use clap::{Parser, Subcommand};

/// The engine's default (unpinned) SARIF `driver.version`. The differential always
/// pins via `--sarif-version-override gate-sarif-v1`; this is only the fallback for a
/// real, unpinned invocation.
const DEFAULT_SARIF_VERSION: &str = env!("CARGO_PKG_VERSION");

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

    /// Output format (Stage 1: only `sarif`).
    #[arg(long = "format", default_value = "sarif")]
    format: String,

    /// Pin the SARIF driver.version (e.g. gate-sarif-v1) for byte-stable output.
    #[arg(long = "sarif-version-override")]
    sarif_version_override: Option<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Commands::Analyze(a) => run_analyze_cmd(a),
    }
}

fn run_analyze_cmd(a: AnalyzeCli) -> ExitCode {
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

    let format = match a.format.as_str() {
        "sarif" => OutputFormat::Sarif,
        other => {
            eprintln!(
                "al-sem: unsupported --format '{other}'. Stage 1 supports: sarif (known presets: {})",
                PRESET_NAMES_LIST.join(", ")
            );
            return ExitCode::from(3);
        }
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
    };

    match run_analyze(&args, DEFAULT_SARIF_VERSION) {
        Ok(sarif) => {
            // al-sem appends a trailing newline to the SARIF string.
            println!("{sarif}");
            ExitCode::SUCCESS
        }
        Err(msg) => {
            eprintln!("al-sem: {msg}");
            ExitCode::from(3)
        }
    }
}
