//! Dual-run parity harness (owned-syntax-IR migration, Phase 1b).
//!
//! Compares the LEGACY CST walk against the NEW al-syntax IR lowerer on real `.al`
//! corpus files, one feature stream at a time, driving the lowerer to parity. This
//! first stage compares the **routine inventory** (object + procedure/trigger
//! names) — proves the harness + the lowerer's outer-structure fidelity. Deeper
//! streams (call sites, ops, refs) are added as the IR-side extractor grows.
//!
//! Run `cargo test --test ir_dual_run -- --nocapture` to see the parity report.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use al_call_hierarchy::dual_run_support::legacy_routine_names;

/// Case-insensitive, quote-stripped normalization (AL identifiers are
/// case-insensitive; the IR strips quotes, so normalize both sides the same way).
fn norm(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(s);
    s.to_ascii_lowercase()
}

fn legacy_routines(source: &str) -> BTreeSet<String> {
    legacy_routine_names(source).iter().map(|n| norm(n)).collect()
}

fn ir_routines(source: &str) -> BTreeSet<String> {
    let file = al_syntax::parse(source);
    file.objects
        .iter()
        .flat_map(|o| o.routines.iter())
        .map(|r| norm(&r.name))
        .collect()
}

/// IR callee method/function names: for each `Call`, the function expr's name
/// (`Identifier` / `Member.member`). Mirrors legacy CALLS `@call.simple` /
/// `@call.method`.
fn ir_call_methods(source: &str) -> Vec<String> {
    use al_syntax::ir::ExprKind;
    let f = al_syntax::parse(source);
    let mut out = Vec::new();
    for e in f.ir.iter_exprs() {
        if let ExprKind::Call { function, .. } = &e.kind {
            match &f.ir.expr(*function).kind {
                ExprKind::Identifier(n) | ExprKind::QuotedIdentifier(n) => out.push(norm(n)),
                ExprKind::Member { member, .. } => out.push(norm(member)),
                _ => {}
            }
        }
    }
    out
}

fn collect_al_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root).into_iter().flatten() {
        let p = entry.path();
        if p.extension().map(|e| e == "al").unwrap_or(false) {
            out.push(p.to_path_buf());
        }
    }
    out.sort();
    out
}

#[test]
fn routine_inventory_parity() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        eprintln!("r0-corpus absent; skipping");
        return;
    }
    let files = collect_al_files(&root);
    assert!(!files.is_empty(), "no .al fixtures found under {}", root.display());

    let mut total = 0usize;
    let mut matching = 0usize;
    let mut divergences: Vec<(String, Vec<String>, Vec<String>)> = Vec::new();

    for f in &files {
        let Ok(source) = std::fs::read_to_string(f) else { continue };
        total += 1;
        let legacy = legacy_routines(&source);
        let ir = ir_routines(&source);
        if legacy == ir {
            matching += 1;
        } else {
            let only_legacy: Vec<String> = legacy.difference(&ir).cloned().collect();
            let only_ir: Vec<String> = ir.difference(&legacy).cloned().collect();
            let rel = f.strip_prefix(&root).unwrap_or(f).display().to_string();
            divergences.push((rel, only_legacy, only_ir));
        }
    }

    let pct = if total > 0 { matching as f64 * 100.0 / total as f64 } else { 0.0 };
    eprintln!(
        "\n=== IR dual-run: routine inventory ===\n{matching}/{total} files match ({pct:.1}%), {} diverge",
        divergences.len()
    );
    for (file, only_legacy, only_ir) in divergences.iter().take(25) {
        eprintln!("  {file}");
        if !only_legacy.is_empty() {
            eprintln!("    legacy-only: {only_legacy:?}");
        }
        if !only_ir.is_empty() {
            eprintln!("    ir-only:     {only_ir:?}");
        }
    }
    if divergences.len() > 25 {
        eprintln!("  ... {} more", divergences.len() - 25);
    }

    // Hard parity gate: the IR routine inventory must match legacy on every file.
    assert_eq!(matching, total, "{} files diverge — see report above", divergences.len());
}

#[test]
fn call_inventory_parity() {
    use al_call_hierarchy::dual_run_support::legacy_call_methods;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        return;
    }
    let files = collect_al_files(&root);
    let mut total = 0usize;
    let mut matching = 0usize;
    let mut divergences: Vec<(String, Vec<String>, Vec<String>)> = Vec::new();

    for f in &files {
        let Ok(source) = std::fs::read_to_string(f) else { continue };
        total += 1;
        let mut legacy: Vec<String> = legacy_call_methods(&source).iter().map(|n| norm(n)).collect();
        let mut ir = ir_call_methods(&source);
        legacy.sort();
        ir.sort();
        if legacy == ir {
            matching += 1;
        } else {
            // report multiset difference
            let lset: BTreeSet<_> = legacy.iter().cloned().collect();
            let iset: BTreeSet<_> = ir.iter().cloned().collect();
            let rel = f.strip_prefix(&root).unwrap_or(f).display().to_string();
            divergences.push((
                rel,
                lset.difference(&iset).cloned().collect(),
                iset.difference(&lset).cloned().collect(),
            ));
        }
    }

    let pct = if total > 0 { matching as f64 * 100.0 / total as f64 } else { 0.0 };
    eprintln!(
        "\n=== IR dual-run: call inventory ===\n{matching}/{total} files match ({pct:.1}%), {} diverge",
        divergences.len()
    );
    for (file, only_legacy, only_ir) in divergences.iter().take(25) {
        eprintln!("  {file}\n    legacy-only: {only_legacy:?}\n    ir-only:     {only_ir:?}");
    }
    if divergences.len() > 25 {
        eprintln!("  ... {} more", divergences.len() - 25);
    }
    // Hard parity gate: IR call inventory must match legacy on every file.
    assert_eq!(matching, total, "{} files diverge — see report above", divergences.len());
}
