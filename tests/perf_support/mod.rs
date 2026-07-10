//! Deterministic synthetic AL corpus generator for performance benchmarks
//! (`benches/lsp_pipeline.rs`) and the CI perf-bounds gate (`tests/perf_bounds.rs`,
//! Task T0.5). Every file's content is a pure function of its index — no RNG, no
//! seed state to keep in sync across runs — so two runs (or two machines)
//! produce byte-identical corpora for the same `file_count`.
//!
//! Shape: `file_count` codeunits, each with [`PROCS_PER_FILE`] procedures
//! (`Proc0..Proc{PROCS_PER_FILE-1}`). Every non-hub file's `Proc0` makes 3
//! calls: one QUALIFIED call into the "hub" codeunit (index [`HUB_INDEX`])
//! plus 2 local calls (`Proc1`, `Proc2`); `Proc1..Proc{N-2}` form a local
//! call chain, and the last procedure is a leaf. This gives:
//! - `incomingCalls` on the hub's `Proc0` real fan-in: `file_count - 1` distinct
//!   callers, one per other file.
//! - `outgoingCalls` on any non-hub file's `Proc0` real fan-out: 3 callees
//!   (1 cross-file qualified + 2 local).
//!
//! so call-hierarchy queries exercise real hash-map fan-out rather than an
//! all-isolated, degenerate corpus.

use std::fs;
use std::path::Path;

/// Procedures generated per codeunit.
pub const PROCS_PER_FILE: usize = 6;

/// Object ID base for generated codeunits — a high custom-range ID that
/// won't collide with any real AL object.
const OBJECT_ID_BASE: u32 = 50100;

/// The 0-indexed file that every other file's `Proc0` calls into, giving it
/// real (and scaling) incoming-call fan-in.
pub const HUB_INDEX: usize = 0;

/// Deterministic object name for file index `i` (fixed-width so names sort
/// and format predictably: `GenCU00000`, `GenCU00001`, ...).
pub fn object_name(i: usize) -> String {
    format!("GenCU{i:05}")
}

/// Deterministic file name (without directory) for file index `i`.
pub fn file_name(i: usize) -> String {
    format!("{}.al", object_name(i))
}

/// Write a synthetic AL corpus of `file_count` codeunits into `dir` (which
/// must already exist). Returns `file_count` for convenience.
pub fn generate_corpus(dir: &Path, file_count: usize) -> usize {
    for i in 0..file_count {
        let content = codeunit_source(i, file_count);
        fs::write(dir.join(file_name(i)), content).expect("write generated AL corpus file");
    }
    file_count
}

/// Rewrite file index `i`'s content with one extra trailing procedure, for
/// exercising the single-file reindex path. Deterministic — always produces
/// the same "changed" content for a given `i`.
pub fn rewrite_with_extra_procedure(dir: &Path, file_count: usize, i: usize) {
    let mut content = codeunit_source(i, file_count);
    // Splice an extra procedure in before the final closing brace.
    let insert_at = content
        .rfind('}')
        .expect("codeunit source has closing brace");
    content.insert_str(
        insert_at,
        "    procedure ProcExtra()\n    begin\n        Proc0();\n    end;\n",
    );
    fs::write(dir.join(file_name(i)), content).expect("rewrite generated AL corpus file");
}

fn codeunit_source(i: usize, file_count: usize) -> String {
    let name = object_name(i);
    let id = OBJECT_ID_BASE + i as u32;
    let mut body = String::new();

    // Proc0: hub call (qualified, skipped for the hub file itself) + 2 local calls.
    body.push_str("    procedure Proc0()\n    begin\n");
    if i != HUB_INDEX && file_count > 1 {
        body.push_str(&format!("        {}.Proc0();\n", object_name(HUB_INDEX)));
    }
    if PROCS_PER_FILE > 1 {
        body.push_str("        Proc1();\n");
    }
    if PROCS_PER_FILE > 2 {
        body.push_str("        Proc2();\n");
    }
    body.push_str("    end;\n\n");

    // Proc1..Proc(PROCS_PER_FILE-2): a local chain, ProcK calls ProcK+1.
    for k in 1..PROCS_PER_FILE.saturating_sub(1) {
        body.push_str(&format!(
            "    procedure Proc{k}()\n    begin\n        Proc{}();\n    end;\n\n",
            k + 1
        ));
    }

    // Last procedure (if more than one exists): a leaf, no calls.
    if PROCS_PER_FILE > 1 {
        let last = PROCS_PER_FILE - 1;
        body.push_str(&format!(
            "    procedure Proc{last}()\n    begin\n    end;\n"
        ));
    }

    format!("codeunit {id} \"{name}\"\n{{\n{body}}}\n")
}

// No `#[cfg(test)]` self-tests live in this file: it is `#[path]`-included
// unconditionally by `benches/lsp_pipeline.rs` (a `harness = false` bench,
// where `#[test]`-annotated functions would compile as plain unreachable
// functions and trip `dead_code`/`unused_imports` — verified empirically).
// See `tests/perf_support_smoke.rs` for the generator's correctness checks.
