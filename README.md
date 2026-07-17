# AL Call Hierarchy

Whole-program semantic analysis engine for AL (Microsoft Dynamics 365 Business Central): a precise call-graph resolver with an LSP server and a code-quality analyzer built on top of it.

[![Rust](https://img.shields.io/badge/rust-1.75+-orange)](https://rust-lang.org)
[![GitHub release](https://img.shields.io/github/v/release/SShadowS/al-call-hierarchy)](https://github.com/SShadowS/al-call-hierarchy/releases)
[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0-blue.svg)](LICENSE)

## Overview

| Metric | Value |
|--------|-------|
| Language | Rust (1.75+, grammar via [`SShadowS/tree-sitter-al`](https://github.com/SShadowS/tree-sitter-al)) |
| Resolution precision | 0.0000% unresolved call edges on a real ~18k-edge BC workspace ([honest taxonomy](#resolution-coverage) — the residual is provably dynamic, never "unknown") |
| Analyzer | 43 default + 11 opt-in detectors, false-positive-triaged against real Microsoft Base/System Application source |
| Query latency | prepare <8µs · outgoing <7µs · incoming ~4ms warm (999-way fan-in) · body-edit save ~13ms |
| Binaries | `al-call-hierarchy` (LSP server) · `alsem` (analyzer CLI) · `aldump` (engine inspection) |

## Features

| Feature | Description |
|---------|-------------|
| **Whole-program call graph** | Every call edge resolved with app-qualified, overload-aware identity across the workspace and its `.alpackages` dependencies (embedded source and symbol-only ABI) |
| **Honest resolution taxonomy** | Edges classify as resolved (source/catalog/ABI), conditionally resolved, provably dynamic, or provably empty — an `unknown` bucket that is actually zero, not defined away |
| **Call hierarchy LSP** | prepare/incoming/outgoing over the resolved graph; multi-root workspaces; incremental two-rung updates on save; UTF-8/UTF-16 position negotiation |
| **Code lens + diagnostics** | Reference counts, unused-procedure detection, code-quality thresholds, published live on every snapshot swap |
| **`alsem analyze`** | 54-detector code-quality analyzer (db-ops-in-loop, commit discipline, event hygiene, TryFunction misuse, …) with SARIF/JSON/HTML/terminal output, baseline diffing, inline suppression, presets |
| **Preflight coverage gate** | Every analyze run verifies resolution coverage with the fresh resolver — degraded analysis is warned loudly, never silent (`--require-dependencies` gates CI) |
| **Unicode-correct identity** | Identifier folding is simple-Unicode (`Løbenr` ≡ `LØBENR`), matching the AL compiler's case-insensitivity beyond ASCII |
| **Event-flow modeling** | IntegrationEvent/BusinessEvent publishers, subscribers, and cross-extension fan-out are first-class graph edges |

## Installation

Prebuilt binaries for each release are on the [releases page](https://github.com/SShadowS/al-call-hierarchy/releases).

From source (clones the grammar submodule):

```bash
git clone --recurse-submodules https://github.com/SShadowS/al-call-hierarchy
cd al-call-hierarchy
cargo build --release
```

## Quick Start

```bash
# LSP server (stdio) — point any LSP client at it
al-call-hierarchy

# Code-quality analysis of an AL workspace
alsem analyze path/to/workspace --format terminal
alsem analyze path/to/workspace --format sarif > findings.sarif
alsem analyze path/to/workspace --preset transaction-integrity

# Engine inspection: the north-star resolution metric
aldump --program-call-graph-stats path/to/workspace
```

The workspace root is the directory containing `app.json`; dependencies are read from `.alpackages/` (embedded source preferred, `SymbolReference.json` ABI otherwise, highest compatible version wins).

The LSP server handles `textDocument/prepareCallHierarchy`, `callHierarchy/incomingCalls`, `callHierarchy/outgoingCalls`, `textDocument/codeLens`, and pushes `textDocument/publishDiagnostics`; see [LSP.md](LSP.md) for wrapper integration.

## Architecture

One engine, two consumers:

```
AL source + .alpackages
        |
        v
  snapshot (app-set ingestion, identity-verified roots)
        |
        v
  program graph (app-qualified nodes, overload-aware identity)
        |
        v
  fresh resolver ────────────► Histogram + per-edge routes   (aldump / alsem gate)
        |
        v
  LspSnapshot (O(1) query surface, Arc-swapped, incremental updater)
        |
        v
  LSP server: call hierarchy · code lens · diagnostics · custom requests
```

## Resolution Coverage

`aldump --program-call-graph-stats` emits the full honest taxonomy per workspace — both whole-program and workspace-scoped:

| Bucket | Meaning |
|--------|---------|
| `resolvedSource` | Target found in workspace/first-party source |
| `resolvedCatalog` | Platform intrinsic (cataloged builtin) |
| `resolvedAbiExternal` | Target found via a dependency's ABI |
| `conditionalResolved` | Resolved under a stated precondition (e.g. interface dispatch) |
| `honestDynamic` | Provably runtime-typed — no static target exists |
| `honestEmpty` | Provably no callee (e.g. unsubscribed event) |
| `unknown` | A true resolution failure — held at **0** on the reference workspace |

A genuinely dynamic call target is reported as such rather than guessed — never silently dropped or misclassified as resolved.

## Performance

| Operation | Target | Enforced |
|-----------|--------|----------|
| Initial index (1000 files) | < 2s | release-mode CI gate, 3× tolerance, every PR |
| prepareCallHierarchy / outgoingCalls | < 1ms | same |
| incomingCalls (999-way fan-in) | ~25ms budget (~4ms warm measured) | same |
| Incremental save (body edit / signature change) | 100ms / ~1.5s budget | same |

## Configuration

| Location | Purpose |
|----------|---------|
| `~/.al-call-hierarchy/config.json` | Global diagnostic thresholds, telemetry opt-out |
| `<workspace>/.al-call-hierarchy.json` | Per-workspace overrides |
| `--no-watcher`, `--no-telemetry`, `--verbose` | Runtime flags (see `--help`) |

## Telemetry

Anonymous, opt-out failure-diagnostics telemetry helps find resolution gaps hit by real projects. **No raw identifiers, paths, or source leave your machine** — identifier names are salted-hashed per installation. Off by default in debug builds, tests, and CI. Disable via `AL_CH_TELEMETRY=0` / `DO_NOT_TRACK=1`, `--no-telemetry`, or the config file. Details: [docs/telemetry.md](docs/telemetry.md); auditable source: [src/telemetry/](src/telemetry/).

## Key Files

| File | Purpose |
|------|---------|
| `src/program/resolve/` | The fresh call/behaviour-edge resolver (the core) |
| `src/snapshot/` | App-set ingestion and dependency identity |
| `src/lsp/` | LSP query surface, incremental updater, diagnostics |
| `src/engine/l5/detectors/` | The analyzer's detector suite |
| `src/bin/alsem.rs` | Analyzer CLI |
| `src/bin/aldump.rs` | Engine inspection CLI |
| `crates/al-syntax/` | Grammar binding, CST→IR lowering, owned AL syntax IR |
| `CHANGELOG.md` | Full history (Keep a Changelog) |

---

**Author**: Torben Leth
**License**: GPL-3.0 (see [LICENSE](LICENSE))
