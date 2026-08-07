# Architecture

For people working on the engine. If you just want to use the tools, the
[README](../README.md) is the place.

One engine, two consumers. The whole-program semantic graph is the only resolution
engine in the tree; both the editor surface and the command-line tools read its resolved
output.

```
AL source + .alpackages
        |
        v
  snapshot            app-set ingestion, identity-verified source roots per app
        |
        v
  al-syntax           parse each file, lower it to the owned AL syntax IR
        |
        v
  program graph       app-qualified nodes, overload-aware routine identity
        |
        v
  resolver ─────────────────────► Histogram + per-edge routes    (aldump, alsem)
        |
        v
  LspSnapshot         immutable Arc-shareable query surface, swapped atomically
        |
        v
  LSP server          call hierarchy · code lens · diagnostics · custom requests
```

## Where things live

The library crate is `al-sem` (imported as `al_sem`). The call-hierarchy LSP server's
binary is still called `al-call-hierarchy`, along with several wire strings — CLAUDE.md's
Project Overview has the table of what deliberately kept the old name and why.

| Path | What it is |
|------|------------|
| `crates/al-syntax/` | The grammar boundary. The **only** crate that links tree-sitter or reads a raw CST: FFI binding, generated raw vocabulary, the CST→IR lowerer, and the owned AL syntax IR every other consumer reads |
| `src/snapshot/` | Turns a workspace plus its dependency tables into identity-verified per-app source roots |
| `src/program/` | The whole-program semantic graph: node identity, topology index, graph assembly, signature fingerprints |
| `src/program/resolve/` | The call/behaviour-edge resolver — the core of the product. `full.rs` is the entry point; `builtins.rs`/`member_catalog.rs` hold the platform intrinsic catalogs; `edge.rs` holds the outcome taxonomy |
| `src/lsp/` | The editor query surface: `LspSnapshot`, the incremental updater, position encoding, and the request handlers |
| `src/state_paths.rs` | The only place that knows both the current (`~/.al-sem/`) and pre-rename (`~/.al-call-hierarchy/`) per-user state locations, and the read-fallback between them |
| `src/engine/deps/` | `.app` ingestion — manifest plus `SymbolReference.json` into a dependency ABI |
| `src/engine/l2/` | Structural body walk and feature projection over the IR |
| `src/engine/l3/` | An older workspace symbol table and call resolver. Advisory only |
| `src/engine/l4/` | Per-routine effect summaries over the call graph, and the database-effect query store behind `alsem query` |
| `src/engine/l5/` | Detectors, findings, event flow, digests, fingerprints, `prove` |
| `src/engine/gate/` | The `alsem analyze` path: report formats, baseline diffing, suppression, policy |
| `src/bin/alsem.rs` | The analyzer and query CLI |
| `src/bin/aldump.rs` | Engine inspection dumps — see its own `usage()` |

## The grammar boundary

`crates/al-syntax` exists so that nothing else has to know what tree-sitter's parse tree
looks like. The lowerer (`crates/al-syntax/src/lower/mod.rs`) is the one place that reads
raw grammar shapes; everything downstream sees a flattened, owned IR. A grammar change
therefore lands in one file, and the differential goldens tell you whether it changed
behaviour.

If you're touching the lowerer, read CLAUDE.md's Grammar section first — it lists the
specific node shapes that have caused real bugs, and why you must verify a claim against
`tree-sitter parse` output rather than a reading of `grammar.js`.

## Build, test, and golden workflow

See [CLAUDE.md](../CLAUDE.md). The short version: format per-file with `rustfmt`, lint
with `scripts/ci-steps clippy`, and never regenerate one golden family on its own —
`scripts/check-goldens` covers all of them together, and the pre-commit hook enforces it.

## Resolution coverage

See [resolution.md](resolution.md) for the outcome taxonomy, the current numbers, and how
to reproduce them.
