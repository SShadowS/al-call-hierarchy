//! `aldump <workspace>` — R0 differential-harness producer.
//!
//! Parses an AL workspace and emits the OBJECT/ROUTINE IDENTITY SUBSET as JSON
//! on stdout, in the EXACT shape of al-sem's committed "golden" files. R0 Task 5
//! diffs this output against those goldens; the extraction logic
//! (`engine::snapshot`) reproduces al-sem's identity derivation precisely so the
//! diff can pass.
//!
//! DESIGN DEVIATION (deliberate, decided by the R0 controller): the R0 plan's
//! Task 4 wording says to "emit a v3-shaped CapabilitySnapshot with L1+ arrays
//! empty." We do NOT do that. R0 compares the identity subset (plan REVIEW #9:
//! "compare parsed structures, not byte-identical JSON"), and that subset carries
//! fields the production v3 snapshot does not have at all (routine sub-kind,
//! `canonicalSignatureText`). A v3 envelope could not carry them, and building
//! the full v3 serde type-zoo just to leave it empty is work for the final
//! byte-identical-snapshot phase. So `aldump` emits the identity-subset JSON
//! directly (see `engine::snapshot::IdentitySnapshot`).
//!
//! Output discipline: ONLY JSON goes to stdout; all logs/warnings go to stderr.
//! No absolute paths appear anywhere in the output.

use std::path::PathBuf;
use std::process::ExitCode;

use al_call_hierarchy::engine::snapshot::snapshot_workspace;

fn main() -> ExitCode {
    // Plain positional arg — keep the surface minimal and dependency-light.
    let mut args = std::env::args_os().skip(1);
    let Some(workspace_arg) = args.next() else {
        eprintln!("usage: aldump <workspace>");
        return ExitCode::FAILURE;
    };
    if args.next().is_some() {
        eprintln!("usage: aldump <workspace>  (exactly one argument expected)");
        return ExitCode::FAILURE;
    }

    let workspace = PathBuf::from(workspace_arg);

    let snapshot = match snapshot_workspace(&workspace) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("aldump: error: {e:#}");
            return ExitCode::FAILURE;
        }
    };

    // Pretty-print with 2-space indent to mirror the goldens (the differ parses
    // structurally, so pretty-printing is a convenience, not a requirement).
    match serde_json::to_string_pretty(&snapshot) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("aldump: error: failed to serialize snapshot: {e}");
            ExitCode::FAILURE
        }
    }
}
