//! The production `analyze` GATE path — port of al-sem's `analyze` CLI command
//! (`src/cli/index.ts`) Stage 1 (projection + filters + SARIF; NO baseline / inline
//! suppression — those are Stage 3).
//!
//! Layout mirrors al-sem:
//!   - `presets`      — `src/detectors/presets.ts` + the CLI detector-selection logic.
//!   - `projection`   — `src/projection/finding-summary.ts` (`project_finding`).
//!   - `filter`       — `src/projection/finding-filters.ts` + scope/limit.
//!   - `format_sarif` — `src/cli/format-sarif.ts` (SARIF 2.1.0 + RULES + codeFlows).
//!   - `run`          — the `analyze` pipeline lib entry (`run_analyze`).
//!
//! The `alsem` bin (`src/bin/alsem.rs`) is a thin clap wrapper over `run::run_analyze`.

pub mod app_attribution;
pub mod exit_code;
pub mod filter;
pub mod format_pr_summary;
pub mod format_sarif;
pub mod model_instance_id;
pub mod preflight;
pub mod presets;
pub mod projection;
pub mod run;
