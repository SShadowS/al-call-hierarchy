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
//!
//! # A SECOND, separate corpus: recursive SCCs ([`generate_recursive_corpus`])
//!
//! The corpus described above has **no recursive SCC at all** — `Proc0`'s calls
//! all point "downward" (`Proc1`/`Proc2`/the hub's `Proc0`) and
//! `Proc1..Proc{N-2}` form a straight-line chain, so every Tarjan SCC over it
//! is a singleton and `Scc::recursive` is `false` everywhere. That was fine
//! while this corpus only had to serve the LSP surface, but it left the L4
//! db-effect solver's RECURSIVE path — the exact path the l4-summary /
//! db-effect-store redesign rewrote (517s -> 11.3s, 24 GB -> 0.47 GB) — with
//! no automated regression guard at all; `tests/perf_bounds.rs`'s
//! `compute_summaries_v2_within_bound` said so in its own doc
//! (final-branch-review finding **M-6**).
//!
//! [`generate_recursive_corpus`] is a SEPARATE generator writing into its own
//! directory. It leaves everything above byte-identical, so no existing bound's
//! baseline moves. Its shape stresses the two axes the redesign attacked, and
//! nothing else:
//!
//! - **Dense recursive cycles.** Each codeunit declares
//!   [`RECURSIVE_CYCLE_PROCS`] procedures `Cyc0..Cyc{M-1}`, where `Cyc{k}` calls
//!   `Cyc{(k+1)%M}`, `Cyc{(k+3)%M}` and `Cyc{(k+5)%M}` — three intra-cycle
//!   out-edges per member, so the set is one strongly-connected component with
//!   ~3M edges rather than a bare M-cycle.
//! - **Cycles that SPAN files**, so an SCC is genuinely large rather than
//!   file-sized: files are partitioned into rings of `ring_files`, and every
//!   file's `Cyc0` additionally calls the NEXT file in its ring through a
//!   declared `Codeunit` variable (the same declared-receiver requirement the
//!   hub call above documents). That fuses a ring's per-file cycles into ONE
//!   recursive SCC of `ring_files * RECURSIVE_CYCLE_PROCS` members.
//! - **Real db effects on every member.** Each `Cyc{k}` performs two distinct
//!   db-touching record operations ([`RECURSIVE_DB_OPS`], indexed so adjacent
//!   members get different ops) against a workspace-local table, so an SCC's
//!   terminal db-effect union is `2 * members` DISTINCT effects rather than a
//!   handful. That is the dimension the retired Jacobi solver multiplied by
//!   (members x effects x passes, re-materialized every pass); the closed-form
//!   store interns ONE shared terminal set per SCC instead.
//!
//! The corpus stays a pure function of `(file_count, ring_files)` — no RNG.
//! `tests/perf_bounds.rs`'s `recursive_scc_db_effects_within_bound` asserts an
//! absolute bound over it AND — first, before timing anything — that the corpus
//! really does produce recursive SCCs of the expected size, since an
//! all-singleton corpus would make that gate measure nothing while still
//! passing.

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

// ---------------------------------------------------------------------------
// Recursive-SCC corpus (final-branch-review M-6) — see the module doc's
// "A SECOND, separate corpus" section for the shape and why it exists.
// ---------------------------------------------------------------------------

/// Procedures per codeunit in the recursive corpus. `Cyc0..Cyc{M-1}` form ONE
/// dense strongly-connected component per file (each member calls `+1`, `+3`
/// and `+5` mod `M`), which the cross-file ring below then fuses with its
/// neighbours' components.
///
/// 8 is the smallest value for which the three strides `+1`/`+3`/`+5` are all
/// distinct and none is the identity, so every member has three real, distinct
/// intra-cycle out-edges.
///
/// ⟨fix wave FIX 3⟩ Unused in `tests/lsp/perf_support_smoke.rs`'s compilation of
/// this `#[path]`-included module (that crate has no recursive-corpus test) —
/// per-item, not blanket, so the EXISTING corpus below keeps its dead-code
/// signal. See that file's module-doc comment.
#[allow(dead_code)]
pub const RECURSIVE_CYCLE_PROCS: usize = 8;

/// Files per cross-file ring in the recursive corpus — the default
/// `tests/perf_bounds.rs`'s gate uses. Each ring is one recursive SCC of
/// `RECURSIVE_RING_FILES * RECURSIVE_CYCLE_PROCS` members, so a
/// `file_count`-file corpus yields `file_count / RECURSIVE_RING_FILES` such
/// SCCs.
///
/// ⟨fix wave finding 3⟩ Only the SCC-SIZE axis is genuinely stressed at the
/// gated corpus size (400 files / ring 100): a per-member re-materialization
/// multiplies against the 800-member/1,600-effect terminal union, which is
/// exactly what the redesign rewrote. The SCC-COUNT axis is NOT — 400 files x
/// 8 procs = 3,200 routines land in just 4 SCCs, so a reintroduced per-SCC
/// workspace-wide rebuild costs ~4 x 3,200 map probes, microseconds against a
/// ~190ms solve and far below this gate's 3.44x detection threshold. The
/// COUNT axis is instead covered by the EXISTING non-recursive corpus (1,000
/// files x 10 routines ≈ 10,000 singleton SCCs) — the two corpora are
/// complementary, not redundant; see `tests/perf_bounds.rs`'s
/// `RECURSIVE_SCC_BOUND` for the measured size-axis separation this value buys.
#[allow(dead_code)]
pub const RECURSIVE_RING_FILES: usize = 100;

/// The db-touching record operations the recursive corpus emits, in the order
/// `Cyc{k}` draws from (ops `2k` and `2k+1`, mod this slice's length). Every
/// entry is (a) classified db-touching by
/// `engine::l4::summary_runner::is_db_touching` — so each call site becomes a
/// real `DbEffect` — and (b) valid AL as a bare, argument-free statement, so
/// the corpus parses and resolves without needing per-op argument shapes.
#[allow(dead_code)]
pub const RECURSIVE_DB_OPS: &[&str] = &[
    "FindSet",
    "FindFirst",
    "FindLast",
    "Insert",
    "Modify",
    "Delete",
    "DeleteAll",
    "LockTable",
];

/// Object ID base for the recursive corpus's generated codeunits — a distinct
/// range from [`OBJECT_ID_BASE`] so the two corpora could coexist in one
/// directory without an ID collision (they never do today; each generator
/// writes its own dir).
#[allow(dead_code)]
const RECURSIVE_OBJECT_ID_BASE: u32 = 51000;

/// Object ID and name of the single table every recursive-corpus routine
/// operates on. One shared table keeps each `DbEffect`'s `table_id` resolved
/// (rather than the `"unknown"` fallback), while the per-call-site operation id
/// still makes every one of the `2 * members` effects distinct.
#[allow(dead_code)]
const RECURSIVE_TABLE_ID: u32 = 50990;
#[allow(dead_code)]
const RECURSIVE_TABLE_NAME: &str = "PerfRecTable";

/// Deterministic object name for recursive-corpus file index `i`.
#[allow(dead_code)]
pub fn recursive_object_name(i: usize) -> String {
    format!("RecCU{i:05}")
}

/// Deterministic file name (without directory) for recursive-corpus file
/// index `i`.
#[allow(dead_code)]
pub fn recursive_file_name(i: usize) -> String {
    format!("{}.al", recursive_object_name(i))
}

/// Write the recursive-SCC corpus (see the module doc) into `dir`, which must
/// already exist: one shared table plus `file_count` codeunits, partitioned
/// into cross-file rings of `ring_files`. Returns the number of `.al` files
/// written (`file_count` + 1 for the table).
///
/// `ring_files` must be >= 1. A ring that ends up with exactly one file emits
/// no cross-file call at all (a codeunit calling its own `Cyc0` through a
/// declared variable pointing at ITSELF is a different, needlessly exotic
/// shape) — that ring's SCC is then just the file's own
/// [`RECURSIVE_CYCLE_PROCS`]-member cycle, still recursive.
#[allow(dead_code)]
pub fn generate_recursive_corpus(dir: &Path, file_count: usize, ring_files: usize) -> usize {
    assert!(ring_files >= 1, "ring_files must be at least 1");
    fs::write(dir.join("PerfRecTable.al"), recursive_table_source())
        .expect("write recursive-corpus table");
    for i in 0..file_count {
        let content = recursive_codeunit_source(i, file_count, ring_files);
        fs::write(dir.join(recursive_file_name(i)), content)
            .expect("write recursive-corpus AL file");
    }
    file_count + 1
}

#[allow(dead_code)]
fn recursive_table_source() -> String {
    let mut s = format!("table {RECURSIVE_TABLE_ID} \"{RECURSIVE_TABLE_NAME}\"\n{{\n");
    s.push_str("    fields\n");
    s.push_str("    {\n");
    s.push_str("        field(1; \"Entry No.\"; Integer) { }\n");
    s.push_str("        field(2; Name; Text[100]) { }\n");
    s.push_str("        field(3; Amount; Decimal) { }\n");
    s.push_str("    }\n");
    s.push_str("    keys { key(PK; \"Entry No.\") { } }\n");
    s.push_str("}\n");
    s
}

/// The next file in `i`'s cross-file ring, or `None` when `i`'s ring holds
/// exactly one file. Rings are contiguous blocks of `ring_files` files; the
/// final block may be shorter when `file_count` is not a multiple.
#[allow(dead_code)]
fn recursive_ring_successor(i: usize, file_count: usize, ring_files: usize) -> Option<usize> {
    let ring_start = (i / ring_files) * ring_files;
    let ring_len = ring_files.min(file_count - ring_start);
    if ring_len <= 1 {
        return None;
    }
    Some(ring_start + (i - ring_start + 1) % ring_len)
}

#[allow(dead_code)]
fn recursive_codeunit_source(i: usize, file_count: usize, ring_files: usize) -> String {
    let name = recursive_object_name(i);
    let id = RECURSIVE_OBJECT_ID_BASE + i as u32;
    let successor = recursive_ring_successor(i, file_count, ring_files);
    let m = RECURSIVE_CYCLE_PROCS;
    let mut body = String::new();

    for k in 0..m {
        // Only `Cyc0` carries the cross-file ring call, so exactly one member
        // per file reaches out — enough to fuse the ring into one SCC without
        // inflating the edge count with `m` redundant copies.
        let ring_call = if k == 0 { successor } else { None };

        body.push_str(&format!("    procedure Cyc{k}()\n    var\n"));
        body.push_str(&format!(
            "        Rec: Record \"{RECURSIVE_TABLE_NAME}\";\n"
        ));
        if let Some(next) = ring_call {
            body.push_str(&format!(
                "        Next: Codeunit \"{}\";\n",
                recursive_object_name(next)
            ));
        }
        body.push_str("    begin\n");
        // Two DISTINCT db-touching ops per member (see RECURSIVE_DB_OPS).
        let ops = RECURSIVE_DB_OPS;
        body.push_str(&format!("        Rec.{}();\n", ops[(2 * k) % ops.len()]));
        body.push_str(&format!(
            "        Rec.{}();\n",
            ops[(2 * k + 1) % ops.len()]
        ));
        // Three intra-file cycle edges: +1, +3, +5 (mod m).
        for stride in [1usize, 3, 5] {
            body.push_str(&format!("        Cyc{}();\n", (k + stride) % m));
        }
        if ring_call.is_some() {
            body.push_str("        Next.Cyc0();\n");
        }
        body.push_str("    end;\n\n");
    }

    format!("codeunit {id} \"{name}\"\n{{\n{body}}}\n")
}

// No `#[cfg(test)]` self-tests live in this file: it is `#[path]`-included
// unconditionally by `benches/lsp_pipeline.rs` (a `harness = false` bench,
// where `#[test]`-annotated functions would compile as plain unreachable
// functions and trip `dead_code`/`unused_imports` — verified empirically).
// See `tests/perf_support_smoke.rs` for the generator's correctness checks.
