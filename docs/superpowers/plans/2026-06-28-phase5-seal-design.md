# Phase 5 design: drop the engine's tree-sitter dependency

## Where we are

Owned-syntax-IR migration. `al-syntax` is the only crate that SHOULD link tree-sitter
(`al_syntax::parse(source) -> AlFile`, an owned IR). Phases 2 (L2), 3 (L3), 4 (LSP
front-end) are DONE: the engine's production analysis + the LSP call-graph front-end all
consume the IR. The root crate still has `tree-sitter = "0.26"` in `[dependencies]`.

## Remaining direct `tree_sitter::` users in the engine crate

1. **`src/engine/snapshot.rs` (PRODUCTION).** Builds the R0 "identity snapshot"
   (objects + routines + stableIds + signature fingerprints + normalizedSignatureHash +
   canonicalSignatureText) via its OWN tree-sitter walk (`extract_from_tree`,
   `extract_object_number/name`, `classify_kind` via prev-sibling attributes,
   `extract_parameters` with al-sem "GAP 1" quoted-name handling, `object_type_for` that
   deliberately SKIPS `permissionsetextension` and maps `xmlport`→"XMLport"). Golden-
   tested by the R0 snapshot goldens. The engine already has IR-based
   `compute_routine_id` + `normalized_signature_hash` (used by L2/L3) that this could reuse.

2. **The L2 "dual-run oracle" (TEST-ONLY in practice).** `src/dual_run_support.rs`
   exposes `pub fn legacy_*` (legacy_l2_features / legacy_call_methods / … ) that re-run
   the LEGACY tree-sitter body-walk and are consumed ONLY by the dual-run gate tests.
   They pull in the legacy L2 path: `engine/l2/{body_walk, cfn, classify,
   control_context, operation_order, node_util, scope, l2_workspace, mod}`. PROBLEM:
   these modules are compiled into the lib (NOT `#[cfg(test)]`), so they make the
   production lib link tree-sitter. AND some are SHARED: production L3 uses
   `scope::compute_routine_id` and `node_util::{strip_quotes, Utf16Cols}` (which do NOT
   need tree-sitter), while the tree-sitter walk lives in OTHER fns of the same modules.

3. `engine/l3/event_graph.rs` — false positive: `Evidence::tree_sitter()` is just a
   provenance STRING label ("tree-sitter"), no real parse. (Could rename for accuracy;
   golden-affecting, so leave or rebaseline.)

## The dual-run tension

There is an OPEN task: "add an L4/L5-summary dual-run gate to catch serde-skip drift."
That gate is IR-INTERNAL (compare IR serde round-trip vs PartialEq) — it does NOT need
tree-sitter and is orthogonal to the LEGACY-vs-IR L2 dual-run. The L2 dual-run oracle's
job (prove IR L2 == legacy tree-sitter L2) is DONE — validated, and the IR L2 is now the
source of truth (Rust-owned goldens). So retiring the L2 oracle does not block task #3.

## Options for dropping the dep

- **(A) Delete** the legacy L2 oracle + `dual_run_support` + the dual-run gate tests;
  port snapshot.rs to IR; remove `tree-sitter` from `[dependencies]`. Loses the
  legacy-vs-IR L2 dual-run permanently (its job is done; goldens remain).
- **(B) Quarantine to tests**: `#[cfg(test)]`-gate the legacy tree-sitter walk (split the
  shared modules so production helpers like `scope::compute_routine_id` stay, but the
  tree-sitter fns compile only under test), port snapshot.rs, and move `tree-sitter` to
  `[dev-dependencies]`. PRODUCTION lib/bin stop linking tree-sitter; the dual-run gate
  survives as a test. Costs module surgery to separate prod helpers from tree-sitter fns.
- **(C)** Keep the dep (Phase 5 deferred).

## Proposed plan

1. Port snapshot.rs to the IR (reuse `compute_routine_id` / `normalized_signature_hash`;
   reproduce the al-sem identity quirks: skip permissionsetextension, XMLport label,
   classify_kind from `RoutineDecl.attributes`, GAP-1 quoted-name param keep). Validate
   byte-identical via the R0 snapshot goldens.
2. For the dual-run oracle: lean **(B)** — quarantine the tree-sitter walk behind
   `#[cfg(test)]` and move the dep to `[dev-dependencies]`, so production is sealed but
   the L2 dual-run + serde-skip gate (task #3) both still run in `cargo test`. Only if the
   module surgery proves disproportionate, fall back to **(A)**.
3. Remove `tree-sitter` (and `streaming-iterator`) from `[dependencies]`.

## Questions for reviewers

1. snapshot.rs port: is byte-identical R0-golden output realistic reusing the IR +
   existing `compute_routine_id`/`normalized_signature_hash`, or are the snapshot's
   al-sem-mirroring quirks (classify_kind prev-sibling scan, GAP-1 param skip,
   permissionsetextension omission) likely to diverge from the IR's view? Which is the
   subtle trap?
2. Dual-run oracle: **(A) delete vs (B) quarantine-to-dev-dep**? Does "drop the engine's
   tree-sitter dependency" mean production-only (so dev-dep is fine and keeps the safety
   net), or fully gone (delete)? Given the open serde-skip dual-run task, is keeping the
   L2 oracle as a test worth the module-surgery cost, or is it dead weight now that the
   IR is the source of truth?
3. Is there any RISK in retiring the legacy-vs-IR L2 dual-run now — i.e. is the IR L2
   genuinely covered by the Rust-owned goldens, or does the dual-run catch a class of
   regression the goldens don't?
