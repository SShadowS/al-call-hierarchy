//! Deterministic synthetic AL corpus generator for performance benchmarks
//! (`benches/lsp_pipeline.rs`) and the CI perf-bounds gate (`tests/perf_bounds.rs`,
//! Task T0.5). Every file's content is a pure function of its index — no RNG, no
//! seed state to keep in sync across runs — so two runs (or two machines)
//! produce byte-identical corpora for the same `file_count`.
//!
//! Shape: `file_count` codeunits, each with [`PROCS_PER_FILE`] procedures
//! (`Proc0..Proc{PROCS_PER_FILE-1}`). Every non-hub file's `Proc0` makes 3
//! calls: one QUALIFIED call into the "hub" codeunit (index [`HUB_INDEX`]),
//! via a locally-declared `Hub: Codeunit "..."` variable, plus 2 local calls
//! (`Proc1`, `Proc2`); `Proc1..Proc{N-2}` form a local call chain, and the
//! last procedure is a leaf. This gives:
//! - `incomingCalls` on the hub's `Proc0` real fan-in: `file_count - 1` distinct
//!   callers, one per other file.
//! - `outgoingCalls` on any non-hub file's `Proc0` real fan-out: 3 callees
//!   (1 cross-file qualified + 2 local).
//!
//! so call-hierarchy queries exercise real hash-map fan-out rather than an
//! all-isolated, degenerate corpus.
//!
//! **The hub call MUST go through a declared variable, never a bare
//! `HubObjectName.Proc0()`** — real AL has no syntax for invoking another
//! object's procedure by its bare display name with no declared receiver
//! (confirmed empirically against the T3 program-engine resolver: a bare
//! object-name "call" like that classifies as `Unknown`/`UntrackedReceiver`,
//! 0% resolved). The legacy LSP pipeline's naive text-matching resolution
//! (`callee_object` is whatever raw source text sits left of the dot,
//! resolved directly against object display names when no variable binding
//! exists for that text — see `src/indexer.rs`'s `add_variable_binding`/
//! `callee_object` handling) tolerated the bare form, which is how this
//! generator originally read; the engine-backed LSP surface (T3) does not.
//!
//! # Event-bearing (t3 whole-branch review — closes a real coverage hole)
//!
//! Every file ALSO declares [`EVENT_ROUTINES_PER_FILE`] event-related
//! routines: two publishers (`OnEventA` — `[IntegrationEvent]`; `OnEventB` —
//! `[InternalEvent]`) and two subscribers (`HandleEventA`/`HandleEventB`,
//! each `[EventSubscriber(...)]`-attributed against the PREVIOUS file's
//! (index `i-1`, wrapping around) matching publisher) — 2 real
//! `event_edges` entries per file, each with exactly one real resolved
//! route. Before this addition, the corpus had ZERO events at all, so
//! `LspSnapshot::event_edges` was always empty and `effective_incoming_count`'s
//! (`src/lsp/lens.rs`) per-declaration publisher-fan-out term read as
//! literally free — the exact condition that let a genuine O(decls ×
//! event_edges) quadratic in `compute_all` (`src/lsp/diagnostics.rs`) go
//! undetected through 17 prior tasks and every review: neither
//! `tests/perf_bounds.rs` nor `benches/lsp_pipeline.rs` ever called
//! `compute_all` at all, and the one thing that WOULD have caught the cost
//! (a non-trivial `event_edges` population) never existed in this shared
//! fixture. See `tests/perf_bounds.rs`'s `compute_all_*` rows for the
//! measurement this now enables.

use std::fs;
use std::path::Path;

/// Procedures generated per codeunit.
pub const PROCS_PER_FILE: usize = 6;

/// Event-related routines generated per codeunit, in ADDITION to
/// [`PROCS_PER_FILE`] — see the module doc's "Event-bearing" section. Always
/// appended AFTER the `Proc*` routines in source order, so `Proc0` stays the
/// first procedure in the file (preserving [`body_only_comment_edit`]'s
/// "first `begin\n` in the file is `Proc0`'s" assumption). Half of these
/// ([`PUBLISHERS_PER_FILE`]) are publishers; the other half are subscribers.
pub const EVENT_ROUTINES_PER_FILE: usize = 4;

/// The publisher half of [`EVENT_ROUTINES_PER_FILE`] (`OnEventA`/`OnEventB`)
/// — exposed separately since `LspSnapshot::event_edges` carries one entry
/// PER PUBLISHER declaration (`emit_event_flow_edges`'s own contract), not
/// per event-bearing routine, so `file_count * PUBLISHERS_PER_FILE` is the
/// corpus's exact expected `event_edges.len()`.
pub const PUBLISHERS_PER_FILE: usize = 2;

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
/// exercising the incremental updater's rung-2 (definition-surface-change)
/// path: a brand-new routine identity always changes the file's `DefSurface`
/// fingerprint. Deterministic — always produces the same "changed" content
/// for a given `i`.
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

/// Rewrite file index `i`'s content with one extra COMMENT line inserted as
/// `Proc0`'s first statement, for exercising the incremental updater's
/// rung-1 (body-only-edit) path: no routine identity, signature, or
/// call-site is added/removed/changed, so the file's `DefSurface`
/// fingerprint stays byte-identical to the unedited content — the exact
/// condition rung 1 requires. Deterministic — always produces the same
/// "changed" content for a given `i`.
pub fn body_only_comment_edit(dir: &Path, file_count: usize, i: usize) {
    let content = codeunit_source(i, file_count);
    // `Proc0` is always the first procedure emitted, so its `begin` is the
    // first one in the file — insert right after it.
    let insert_at = content
        .find("begin\n")
        .expect("codeunit source has a begin block")
        + "begin\n".len();
    let mut new_content = content;
    new_content.insert_str(insert_at, "        // rung-1 perf probe: body-only edit\n");
    fs::write(dir.join(file_name(i)), new_content)
        .expect("rewrite generated AL corpus file (body-only edit)");
}

fn codeunit_source(i: usize, file_count: usize) -> String {
    let name = object_name(i);
    let id = OBJECT_ID_BASE + i as u32;
    let mut body = String::new();
    let calls_hub = i != HUB_INDEX && file_count > 1;

    // Proc0: hub call (qualified, via a locally-declared `Hub` variable — real
    // AL has no syntax for calling another object by its bare display name
    // with no declared receiver, so a var declaration is required for the
    // call to be genuinely resolvable, not just parseable — see this
    // module's doc) + 2 local calls.
    body.push_str("    procedure Proc0()\n");
    if calls_hub {
        body.push_str(&format!(
            "    var\n        Hub: Codeunit \"{}\";\n",
            object_name(HUB_INDEX)
        ));
    }
    body.push_str("    begin\n");
    if calls_hub {
        body.push_str("        Hub.Proc0();\n");
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

    // Event routines (see the module doc's "Event-bearing" section): this
    // file's own 2 publishers, plus 2 subscribers targeting the PREVIOUS
    // file's publishers (wrapping around — a lone file self-subscribes,
    // still exercising the publisher side, which is what drives
    // `event_edges` scale).
    let target = object_name((i + file_count - 1) % file_count);
    body.push_str("\n    [IntegrationEvent(false, false)]\n");
    body.push_str("    procedure OnEventA()\n    begin\n    end;\n\n");
    body.push_str("    [InternalEvent(false)]\n");
    body.push_str("    procedure OnEventB()\n    begin\n    end;\n\n");
    body.push_str(&format!(
        "    [EventSubscriber(ObjectType::Codeunit, Codeunit::\"{target}\", 'OnEventA', '', false, false)]\n"
    ));
    body.push_str("    local procedure HandleEventA()\n    begin\n    end;\n\n");
    body.push_str(&format!(
        "    [EventSubscriber(ObjectType::Codeunit, Codeunit::\"{target}\", 'OnEventB', '', false, false)]\n"
    ));
    body.push_str("    local procedure HandleEventB()\n    begin\n    end;\n");

    format!("codeunit {id} \"{name}\"\n{{\n{body}}}\n")
}

// No `#[cfg(test)]` self-tests live in this file: it is `#[path]`-included
// unconditionally by `benches/lsp_pipeline.rs` (a `harness = false` bench,
// where `#[test]`-annotated functions would compile as plain unreachable
// functions and trip `dead_code`/`unused_imports` — verified empirically).
// See `tests/perf_support_smoke.rs` for the generator's correctness checks.
