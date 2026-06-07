//! `aldump <workspace>` — R0 differential-harness producer.
//! `aldump --l2 <workspace>` — R1a L2 features projection producer (Task 3).
//!
//! DEFAULT (no `--l2`): parses an AL workspace and emits the OBJECT/ROUTINE
//! IDENTITY SUBSET as JSON on stdout, in the EXACT shape of al-sem's committed
//! R0 "golden" files. R0 Task 5 diffs this output against those goldens; the
//! extraction logic (`engine::snapshot`) reproduces al-sem's identity derivation
//! precisely so the diff can pass.
//!
//! `--l2`: parses the workspace and emits the ALLOWLISTED L2 FEATURES PROJECTION
//! (`engine::l2::l2_workspace`) — objects + routines with metadata + per-routine
//! `features` (loops/operations/call-sites/record-ops/CFN skeleton/…), matching
//! the R1a goldens (`scripts/r1a-goldens/<fixture>.l2.golden.json`). Forbidden
//! later-gate / L3-resolved fields are structurally absent from the projection
//! types, so they can never appear here.
//!
//! DESIGN DEVIATION (R0, deliberate): the default mode emits the identity-subset
//! JSON directly rather than a v3-shaped CapabilitySnapshot — that subset carries
//! fields (routine sub-kind, `canonicalSignatureText`) a v3 envelope cannot.
//!
//! Output discipline: ONLY JSON goes to stdout; all logs/warnings go to stderr.
//! No absolute paths appear anywhere in the output.

use std::path::PathBuf;
use std::process::ExitCode;

use al_call_hierarchy::engine::l2::l2_workspace::project_workspace;
use al_call_hierarchy::engine::snapshot::snapshot_workspace;

fn usage() -> ExitCode {
    eprintln!("usage: aldump [--l2] <workspace>");
    ExitCode::FAILURE
}

fn main() -> ExitCode {
    let mut l2 = false;
    let mut workspace_arg: Option<std::ffi::OsString> = None;

    for arg in std::env::args_os().skip(1) {
        // `--l2` flag (anywhere); everything else is the single positional.
        if arg == "--l2" {
            l2 = true;
            continue;
        }
        if workspace_arg.is_some() {
            eprintln!("aldump: error: more than one workspace argument");
            return usage();
        }
        workspace_arg = Some(arg);
    }

    let Some(workspace_arg) = workspace_arg else {
        return usage();
    };
    let workspace = PathBuf::from(workspace_arg);

    if l2 {
        let projection = match project_workspace(&workspace) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("aldump: error: {e:#}");
                return ExitCode::FAILURE;
            }
        };
        match serde_json::to_string_pretty(&projection) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("aldump: error: failed to serialize L2 projection: {e}");
                ExitCode::FAILURE
            }
        }
    } else {
        let snapshot = match snapshot_workspace(&workspace) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("aldump: error: {e:#}");
                return ExitCode::FAILURE;
            }
        };
        // Pretty-print with 2-space indent to mirror the goldens (the differ
        // parses structurally, so pretty-printing is a convenience).
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
}
