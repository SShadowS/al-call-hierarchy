//! CST → IR lowering — the ONLY grammar-aware logic above the raw layer.
//!
//! Skeleton: Phase 1 fills the per-construct lowering (objects → routines → blocks
//! → statements → expressions), descending transparent wrappers via the typed
//! nodes, dropping `Trivia`, recording `Recovery`/`Unknown` as issues (never silent
//! drops), and stamping `Origin` on every node — then drives it to byte-parity with
//! the legacy walk under dual-run (spec §5 Phase 1).

use crate::ir::{AlFile, Ir, Origin, ParseStatus, Point};
use crate::raw::RawNode;

/// Lower a parsed file root into the owned IR.
pub fn lower_file(root: RawNode, _source: &str) -> AlFile {
    let parse_status = if root.has_error() {
        ParseStatus::Recovered
    } else {
        ParseStatus::Clean
    };

    // Phase 1 fills this: iterate the root's object declarations and lower each.
    AlFile {
        objects: Vec::new(),
        ir: Ir::new(),
        issues: Vec::new(),
        parse_status,
    }
}

/// Build an [`Origin`] from a raw node (used pervasively by Phase 1 lowering).
#[allow(dead_code)] // wired in by per-construct lowering (Phase 1)
pub(crate) fn origin_of(n: RawNode) -> Origin {
    let s = n.start_position();
    let e = n.end_position();
    Origin {
        kind_text: n.kind_str(),
        ts_id: n.id(),
        byte: n.byte_range(),
        start: Point { row: s.row as u32, column: s.column as u32 },
        end: Point { row: e.row as u32, column: e.column as u32 },
    }
}
