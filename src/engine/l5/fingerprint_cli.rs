//! cli-b/b3 — FINGERPRINT CLI pipeline.
//!
//! `run_fingerprint_pipeline` assembles the workspace, composes the consumed-core
//! snapshot, runs the query, and format-dispatches per the contract from
//! `src/cli/fingerprint.ts`:
//!
//! ## Format dispatch (fingerprint.ts:207-298)
//!
//! 1. `--shard`                              → B0 `serialize_sharded` (PRECEDES query)
//! 2. `--format cbor`                        → B0 `serialize_cbor`
//! 3. `--format cbor.gz`                     → B0 `serialize_cbor_gz`
//! 4. `--format json` + NO query flag        → B0 `serialize_envelope`
//! 5. `--format json` + query flag           → `fingerprint_query` → `project_fingerprint_query_full`
//! 6. `--format human`                       → `fingerprint_query` → `format_fingerprint_human`
//!
//! `isQueryRequested` = any of `--witness`, `--roots`, `--routine`, `--include-inherited`
//! specified (even with default values).
//!
//! Error path:
//!   - Selector unresolved / ambiguous → exit 2.
//!   - json: valid doc in payload.diagnostics, no stderr.
//!   - human: message to stderr, exit 2, no stdout.
//!   - `.app` input: rejected with exit 1 (no workspace).
//!
//! Illegal combos:
//!   - `--shard` + query flag     → exit 1 (error).
//!   - `--format cbor` + query    → exit 1 (error).
//!   - `--format cbor.gz` + query → exit 1 (error).

use crate::engine::gate::model_instance_id::compute_gate_model_instance_id;
use crate::engine::gate::run::compute_analyzer_diagnostics;
use crate::engine::l3::l3_workspace::assemble_and_resolve_workspace;
use crate::engine::l5::detectors::registered_detectors;
use crate::engine::l5::digest_cli::{build_envelope_diagnostics_json, DEFAULT_DETECTOR_NAMES};
use crate::engine::l5::fingerprint_query::{
    fingerprint_query, format_fingerprint_human_verbosity, project_fingerprint_query_full,
    FingerprintFilters, FingerprintPipelineInput, FingerprintQueryDiagnostic, WitnessLimit,
};
use crate::engine::l5::snapshot::compose_snapshot;
use crate::engine::l5::snapshot_full::{
    compose_full_snapshot, serialize_cbor, serialize_cbor_gz, serialize_envelope, serialize_json,
    serialize_sharded, EnvelopeDiagnostic, FullSnapshotOptions,
};

/// The format the fingerprint command outputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FingerprintFormat {
    Json,
    Human,
    Cbor,
    CborGz,
}

impl FingerprintFormat {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "json" => Some(Self::Json),
            "human" => Some(Self::Human),
            "cbor" => Some(Self::Cbor),
            "cbor.gz" => Some(Self::CborGz),
            _ => None,
        }
    }
}

/// Options for `run_fingerprint_pipeline`.
pub struct FingerprintOptions<'a> {
    /// Workspace path.
    pub workspace: &'a std::path::Path,
    /// al-sem version string (e.g. `"cli-b-v1"`).
    pub alsem_version: &'a str,
    /// Output format.
    pub format: FingerprintFormat,
    /// Output file (stdout when None).
    pub out: Option<&'a str>,
    /// Emit sharded output instead of a single file.
    pub shard: bool,
    /// Witness limit: None = use default (3 = golden default); Some(WitnessLimit) = explicit.
    pub witness_limit: Option<WitnessLimit>,
    /// Root-kind filter (None = all roots).
    pub roots: Option<std::collections::BTreeSet<String>>,
    /// Routine selectors (empty = all routines).
    pub routine_selectors: Vec<String>,
    /// When true, include inherited (transitive) facts.
    pub include_inherited: bool,
    /// When true, any of witness/roots/routine/include-inherited was explicitly specified.
    pub is_query_requested: bool,
    /// Pin timestamps for byte-stable output.
    pub deterministic: bool,
    /// Verbosity for human output: "compact" | "full".
    pub verbosity: &'a str,
}

/// Result returned from `run_fingerprint_pipeline`.
pub enum FingerprintOutput {
    /// Text output (JSON or human).
    Text(String),
    /// Binary output (CBOR or CBOR.gz).
    Binary(Vec<u8>),
    /// Multiple shard files (filename, text content).
    Shards(Vec<(String, Vec<u8>)>),
}

pub struct FingerprintRunResult {
    pub output: FingerprintOutput,
    /// Exit code: 0 = ok, 2 = selector error.
    pub exit_code: u8,
    /// Selector error message for human format (emit to stderr).
    pub selector_error_message: Option<String>,
}

/// Run the full fingerprint pipeline.
pub fn run_fingerprint_pipeline(opts: &FingerprintOptions) -> Result<FingerprintRunResult, String> {
    // Validate illegal combos.
    if opts.shard && opts.is_query_requested {
        return Err(
            "fingerprint: --shard is not compatible with query flags (--witness, --roots, --routine, --include-inherited)".to_string(),
        );
    }
    if (opts.format == FingerprintFormat::Cbor || opts.format == FingerprintFormat::CborGz)
        && opts.is_query_requested
    {
        return Err(format!(
            "fingerprint: --format {} is not compatible with query flags",
            if opts.format == FingerprintFormat::Cbor {
                "cbor"
            } else {
                "cbor.gz"
            }
        ));
    }

    // Assemble workspace.
    let model_id = compute_gate_model_instance_id(opts.workspace)
        .ok_or_else(|| "fingerprint: could not compute modelInstanceId".to_string())?;
    let resolved = assemble_and_resolve_workspace(opts.workspace, &model_id)
        .ok_or_else(|| "fingerprint: workspace did not resolve".to_string())?;

    // B0 path: shard / cbor / cbor.gz / json-no-query.
    let go_b0 = opts.shard
        || opts.format == FingerprintFormat::Cbor
        || opts.format == FingerprintFormat::CborGz
        || (opts.format == FingerprintFormat::Json && !opts.is_query_requested);

    if go_b0 {
        // Build envelope diagnostics (shared 34-detector set).
        let default_detectors: Vec<_> = registered_detectors()
            .into_iter()
            .filter(|d| DEFAULT_DETECTOR_NAMES.contains(&d.name.as_str()))
            .collect();
        let diags_vec = compute_analyzer_diagnostics(opts.workspace, &resolved, &default_detectors);
        let envelope_diags: Vec<EnvelopeDiagnostic> = diags_vec
            .into_iter()
            .map(|d| EnvelopeDiagnostic {
                code: format!("DIAG-{}", d.stage),
                severity: d.severity,
                message: d.message,
            })
            .collect();

        let full_opts = FullSnapshotOptions {
            workspace_dir: opts.workspace,
            alsem_version: opts.alsem_version,
            deterministic: opts.deterministic,
            roots_config_ignored: false,
        };
        let tree = compose_full_snapshot(&resolved, &full_opts);

        let output = if opts.shard {
            // Sharded output: returns (filename, bytes) pairs.
            // serialize_sharded: primary_only = opts.deterministic (or false = all shards).
            // The manifest says --shard = primaryOnly:false for the golden fixture.
            let shards = serialize_sharded(&tree, opts.alsem_version, false);
            FingerprintOutput::Shards(shards.into_iter().map(|s| (s.name, s.bytes)).collect())
        } else if opts.format == FingerprintFormat::Cbor {
            FingerprintOutput::Binary(serialize_cbor(&tree))
        } else if opts.format == FingerprintFormat::CborGz {
            FingerprintOutput::Binary(serialize_cbor_gz(&tree))
        } else {
            // json, no query
            let text = serialize_envelope(
                &tree,
                opts.alsem_version,
                opts.deterministic,
                &envelope_diags,
            );
            FingerprintOutput::Text(text)
        };

        return Ok(FingerprintRunResult {
            output,
            exit_code: 0,
            selector_error_message: None,
        });
    }

    // Query path: json-with-query or human.
    // Compose the consumed-core snapshot (not the full CBOR tree).
    let snap = compose_snapshot(&resolved);
    let workspace_fp = crate::engine::l5::snapshot_full::workspace_fingerprint_of(
        opts.workspace,
        opts.alsem_version,
    );

    // Build envelope diagnostics (for the JSON envelope; human omits them).
    let diags_json = build_envelope_diagnostics_json(opts.workspace, &resolved);
    let analyzer_diagnostics: Vec<(String, String, String)> =
        if let serde_json::Value::Array(arr) = diags_json {
            arr.into_iter()
                .filter_map(|v| {
                    let obj = v.as_object()?;
                    let code = obj.get("code")?.as_str()?.to_string();
                    let severity = obj.get("severity")?.as_str()?.to_string();
                    let message = obj.get("message")?.as_str()?.to_string();
                    Some((code, severity, message))
                })
                .collect()
        } else {
            Vec::new()
        };

    // Witness limit: default to 3 (the golden capturePoint default).
    let witness_limit = opts.witness_limit.unwrap_or(WitnessLimit::Capped(3));

    let filters = FingerprintFilters {
        roots: opts.roots.clone(),
        routine_selectors: opts.routine_selectors.clone(),
        include_inherited: opts.include_inherited,
        witness_limit,
    };

    let result = fingerprint_query(&snap, &filters);

    let has_selector_errors = result.diagnostics.iter().any(|d| {
        matches!(
            d,
            crate::engine::l5::fingerprint_query::FingerprintQueryDiagnostic::SelectorUnresolved { .. }
                | crate::engine::l5::fingerprint_query::FingerprintQueryDiagnostic::SelectorAmbiguous { .. }
        )
    });
    let exit_code: u8 = if has_selector_errors { 2 } else { 0 };

    if opts.format == FingerprintFormat::Human {
        if has_selector_errors {
            // Human format: selector errors go to stderr, no stdout.
            let msgs: Vec<String> = result
                .diagnostics
                .iter()
                .map(|d| match d {
                    FingerprintQueryDiagnostic::SelectorUnresolved { selector } => {
                        format!("al-sem fingerprint: selector not found: {selector}")
                    }
                    FingerprintQueryDiagnostic::SelectorAmbiguous {
                        selector,
                        matched_form,
                        ..
                    } => {
                        format!("al-sem fingerprint: selector '{selector}' is ambiguous (matchedForm={matched_form})")
                    }
                })
                .collect();
            return Ok(FingerprintRunResult {
                output: FingerprintOutput::Text(String::new()),
                exit_code: 2,
                selector_error_message: Some(msgs.join("\n")),
            });
        }
        let human = format_fingerprint_human_verbosity(&result, opts.verbosity);
        return Ok(FingerprintRunResult {
            output: FingerprintOutput::Text(human),
            exit_code: 0,
            selector_error_message: None,
        });
    }

    // json + query.
    let json_text = project_fingerprint_query_full(
        &result,
        &snap,
        &opts.workspace.to_string_lossy(),
        &filters,
        opts.deterministic,
        &analyzer_diagnostics,
        opts.alsem_version,
        &workspace_fp,
    );

    Ok(FingerprintRunResult {
        output: FingerprintOutput::Text(json_text),
        exit_code,
        selector_error_message: None,
    })
}

/// Write the fingerprint output to a file or stdout.
pub fn write_fingerprint_output(
    output: &FingerprintOutput,
    out: Option<&str>,
) -> Result<(), String> {
    match output {
        FingerprintOutput::Text(text) => {
            if let Some(path) = out {
                std::fs::write(path, text).map_err(|e| format!("fingerprint: write error: {e}"))
            } else {
                use std::io::Write;
                std::io::stdout()
                    .write_all(text.as_bytes())
                    .map_err(|e| format!("fingerprint: stdout write error: {e}"))
            }
        }
        FingerprintOutput::Binary(bytes) => {
            if let Some(path) = out {
                std::fs::write(path, bytes).map_err(|e| format!("fingerprint: write error: {e}"))
            } else {
                use std::io::Write;
                std::io::stdout()
                    .write_all(bytes)
                    .map_err(|e| format!("fingerprint: stdout write error: {e}"))
            }
        }
        FingerprintOutput::Shards(shards) => {
            // For shards, `out` is treated as the output directory.
            // When None, use the current directory.
            let dir = out.unwrap_or(".");
            let dir_path = std::path::Path::new(dir);
            if !dir_path.exists() {
                std::fs::create_dir_all(dir_path)
                    .map_err(|e| format!("fingerprint: create dir error: {e}"))?;
            }
            for (filename, content) in shards {
                let file_path = dir_path.join(filename);
                std::fs::write(&file_path, content)
                    .map_err(|e| format!("fingerprint: write shard {filename}: {e}"))?;
            }
            Ok(())
        }
    }
}
