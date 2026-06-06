# Engine migration: grammar provenance & convergence

> Scratch/provenance note created in R0 Task 6. Task 7 finalizes the full migration doc.

## Oracle grammar provenance

al-sem's committed goldens were produced with the **`tree-sitter-al` `v2.5.2-shim`**
grammar (native DLL build):

- al-sem `GRAMMAR_VERSION = "tree-sitter-al-v2.5.2-native"`, package version `2.5.2`.
- The `v2.5.2-shim` tag == commit **`89b1d055214d95bcf9596e168b240df313bd1a36`** on
  `github.com/SShadowS/tree-sitter-al` (the canonical repo). That commit carries the
  committed `src/parser.c` (~24 MB) + `src/scanner.c`.
- The canonical local checkout `U:\Git\tree-sitter-al` is already AT `89b1d05`
  (package `2.5.2`).

This grammar is the **oracle**: any differential divergence between the Rust engine and
the goldens is a real bug, not a grammar-version artifact.

## Convergence = advance the submodule pin

The engine's bundled submodule `tree-sitter-al/` uses the **same remote**
(`.gitmodules` url = `https://github.com/SShadowS/tree-sitter-al.git`) but was pinned to
the STALE **`v2.0.0`** commit `a9dc044ea07e773d974c9f772b1a8cae7001d5ab`.

Convergence is therefore simply **advancing the submodule gitlink pin**:

| revision | grammar | role |
| --- | --- | --- |
| `a9dc044` | `v2.0.0` | stale engine-submodule pin (pre-convergence) |
| `89b1d05` | `v2.5.2-shim` | oracle grammar (al-sem goldens) — convergence target |

No `build.rs` rewrite is needed: `build.rs` already compiles
`tree-sitter-al/src/parser.c` from the submodule (default
`TREE_SITTER_AL_PATH="tree-sitter-al"`), so once the submodule is at `89b1d05` the
engine compiles the 2.5.2 grammar automatically.

The bundled `tree-sitter-al/` was never a *separate fork* — it is the canonical remote at
a stale pin. So the plan's "remove the bundled fork" step is moot; `.gitmodules` is left
unchanged. The grammar swap is done LAST (after the harness exists) so the differential
harness catches any AST-shape delta the swap introduces.
