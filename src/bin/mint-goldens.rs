//! 1B.3b Task 1: the DEV-ONLY committed-golden minting tool.
//!
//! The LAST sanctioned L3 oracle use after this task lands: `src/program/resolve`
//! (the gate module) calls L3 only through [`mint_l3_validated_golden`] /
//! [`mint_l3_trigger_golden`] (and `differential::project_l3_event_rows`),
//! and this binary is the ONLY caller of those — the runtime audits
//! (`run_cdo_semantic_audit`/`run_cdo_trigger_audit`/`run_cdo_event_audit`)
//! LOAD the committed output instead.
//!
//! Mints + ANONYMIZES (via [`anon::anon`] — see that module's docs for the
//! domain-separation + HMAC-governance writeup) the three committed goldens
//! under `tests/goldens/semantic-edges/`:
//!   - `cdo-anon.json`         — Member/Interface ([`mint_l3_validated_golden`])
//!   - `cdo-trigger-anon.json` — ImplicitTrigger ([`mint_l3_trigger_golden`])
//!   - `cdo-event-anon.json`   — EventFlow (`project_l3_event_rows`)
//!
//! All three are written MINIFIED (single-line JSON; the ~13k-site CDO golden
//! is a large committed artifact and pretty-printing roughly doubles it for
//! no review benefit — a diff tool handles structural JSON diffs either way).
//!
//! ALSO writes/merges the GITIGNORED local de-anonymization map
//! (`cdo-deanon-map.json`, `AnonId -> human-readable plaintext`) so a
//! developer with CDO access can reverse a failing anonymized diff back to
//! the exact broken AL code (the committed goldens themselves never carry
//! plaintext).
//!
//! Usage: `CDO_WS=<workspace> CDO_ANON_KEY=<secret> cargo run --release --bin
//! mint-goldens` (workspace defaults to `$CDO_WS`; an explicit positional arg
//! overrides it). Output discipline: progress/summary goes to stderr; the
//! tool writes files directly, nothing meaningful goes to stdout.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use al_call_hierarchy::program::resolve::anon::{self, ANON_KEY_ENV};
use al_call_hierarchy::program::resolve::differential::project_l3_event_rows;
use al_call_hierarchy::program::resolve::semantic_golden::{
    anonymize_event_rows_with_deanon, anonymize_golden_with_deanon, cdo_anon_golden_path,
    cdo_deanon_map_path, cdo_event_anon_golden_path, cdo_trigger_anon_golden_path,
    merge_deanon_map, mint_l3_trigger_golden, mint_l3_validated_golden,
};

fn usage() -> ExitCode {
    eprintln!(
        "usage: CDO_WS=<workspace> {ANON_KEY_ENV}=<secret> cargo run --release \
         --bin mint-goldens [-- <workspace-root>] [--insecure-test-key]\n\
         \n\
         Mints + anonymizes the three committed CDO-derived goldens under\n\
         tests/goldens/semantic-edges/ (cdo-anon.json, cdo-trigger-anon.json,\n\
         cdo-event-anon.json) plus the GITIGNORED local de-anon map\n\
         (cdo-deanon-map.json). The LAST sanctioned L3 oracle use (1B.3b Task 1).\n\
         \n\
         <workspace-root> defaults to $CDO_WS when omitted as a positional arg.\n\
         \n\
         {ANON_KEY_ENV} MUST be set to a real, NON-COMMITTED secret when minting\n\
         real CDO data — without it, anon() falls back to a committed test key,\n\
         which would make the committed golden's ids dictionary-attackable (see\n\
         anon.rs's module docs, \"Governance\" section). Pass --insecure-test-key\n\
         to override ONLY for local tool development against synthetic/\n\
         non-proprietary fixtures."
    );
    ExitCode::FAILURE
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let insecure_test_key = args.iter().any(|a| a == "--insecure-test-key");
    let positional = args.iter().find(|a| !a.starts_with("--"));

    let workspace_root: PathBuf = match positional {
        Some(p) => PathBuf::from(p),
        None => match std::env::var_os("CDO_WS") {
            Some(v) if !v.is_empty() => PathBuf::from(v),
            _ => {
                eprintln!("error: no workspace given and CDO_WS is unset");
                return usage();
            }
        },
    };
    if !workspace_root.exists() {
        eprintln!(
            "error: workspace root does not exist: {}",
            workspace_root.display()
        );
        return ExitCode::FAILURE;
    }

    let key_is_set = std::env::var(ANON_KEY_ENV)
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    if !key_is_set && !insecure_test_key {
        eprintln!(
            "error: {ANON_KEY_ENV} is unset/empty. Minting real CDO data with the \
             committed fallback test key would make the result dictionary-attackable."
        );
        return usage();
    }
    if !key_is_set {
        eprintln!(
            "WARNING: --insecure-test-key set, {ANON_KEY_ENV} unset — using the \
             committed fallback key. Output is NOT safe to commit if `workspace_root` \
             contains proprietary data."
        );
    }

    eprintln!(
        "mint-goldens: workspace={} (1B.3b Task 1 — LAST sanctioned L3 use)",
        workspace_root.display()
    );

    let mut deanon: BTreeMap<String, String> = BTreeMap::new();

    // ── (a) Member/Interface ─────────────────────────────────────────────────
    eprintln!("  minting Member/Interface golden (mint_l3_validated_golden / project_l3)...");
    let member_golden = mint_l3_validated_golden(&workspace_root);
    let member_anon =
        anonymize_golden_with_deanon(&member_golden, anon::SITE_DOMAIN_V1, &mut deanon);
    let member_path = cdo_anon_golden_path();
    write_minified(&member_path, &member_anon);
    eprintln!(
        "    {} site(s) -> {}",
        member_anon.entries.len(),
        member_path.display()
    );

    // ── (b) ImplicitTrigger ───────────────────────────────────────────────────
    eprintln!(
        "  minting ImplicitTrigger golden (mint_l3_trigger_golden / project_l3_implicit_trigger_in_scope)..."
    );
    let trigger_golden = mint_l3_trigger_golden(&workspace_root);
    let trigger_anon =
        anonymize_golden_with_deanon(&trigger_golden, anon::TRIGGER_OP_DOMAIN_V1, &mut deanon);
    let trigger_path = cdo_trigger_anon_golden_path();
    write_minified(&trigger_path, &trigger_anon);
    eprintln!(
        "    {} site(s) -> {}",
        trigger_anon.entries.len(),
        trigger_path.display()
    );

    // ── (c) EventFlow ─────────────────────────────────────────────────────────
    eprintln!("  minting EventFlow golden (project_l3_event_rows)...");
    let event_rows = project_l3_event_rows(&workspace_root);
    let event_anon = anonymize_event_rows_with_deanon(&event_rows, &mut deanon);
    let event_path = cdo_event_anon_golden_path();
    write_minified(&event_path, &event_anon);
    eprintln!(
        "    {} pair(s) -> {}",
        event_anon.entries.len(),
        event_path.display()
    );

    // ── de-anon map (gitignored, local-only) ─────────────────────────────────
    let deanon_path = cdo_deanon_map_path();
    let deanon_count = deanon.len();
    merge_deanon_map(&deanon_path, &deanon);
    eprintln!(
        "  merged {deanon_count} de-anon entries -> {} (GITIGNORED, local-only)",
        deanon_path.display()
    );

    eprintln!("mint-goldens: done.");
    ExitCode::SUCCESS
}

fn write_minified<T: serde::Serialize>(path: &Path, value: &T) {
    let json = serde_json::to_string(value).expect("serialize golden to JSON");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create tests/goldens/semantic-edges dir");
    }
    std::fs::write(path, json).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}
