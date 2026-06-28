# Remaining Items — post owned-IR migration + case-pattern grammar cleanup

Status snapshot (2026-06-28): the owned-AL-syntax-IR migration (Phases 2–5) and the
case-pattern grammar field-pollution cleanup are **complete and committed locally**:
- submodule `tree-sitter-al` `2bfc7fc` on branch `owned-ir-grammar-fixes`
- engine `9c06717` on `feat/owned-syntax-ir` (pin bump + regen vocab + lowerer tests + CHANGELOG)

Both are **local only — not pushed**. `cargo test --workspace` green incl. al-sem
differential goldens (zero divergence). Reviewed: gpt-5.5 + gemini-3.1-pro.

This file tracks everything still open so nothing is forgotten.

---

## P0 — Blocking / CI integrity (do before this branch builds in CI)

### 1. Push the grammar branch + sync canonical
The grammar change exists in two places that must converge before CI (which checks out
`tree-sitter-al` `main` HEAD, unpinned) sees it.

- [ ] Push submodule branch `owned-ir-grammar-fixes` (`2bfc7fc`, atop `c4d2cb2`
      call_statement, atop `a6128d0` named in/is/as + case_else_branch) to GitHub
      `tree-sitter-al`.
- [ ] Decide merge target: merge `owned-ir-grammar-fixes` → `main` on GitHub (CI builds
      `main` HEAD). Until merged to `main`, CI will NOT see these grammar fixes and will
      diverge from the engine's pinned `node-types.sha256` hash `8f9b7013…` → build.rs
      hash-guard fails.
- [ ] Sync canonical working copy `U:\Git\tree-sitter-al` with the merged grammar
      (`git pull`), so local canonical == submodule == GitHub `main`.
- [ ] Confirm the engine submodule pin (`tree-sitter-al` gitlink in `9c06717`) points at
      the rev that lands on `main`. If the merge produces a new SHA (e.g. squash/rebase),
      update the gitlink + regen vocab + re-verify the hash, then amend/commit.

**Acceptance:** fresh clone `--recurse-submodules` → `cargo run -p xtask -- gen-syntax &&
git diff --exit-code` clean AND `cargo test --workspace` green. CI checking out grammar
`main` HEAD produces hash `8f9b7013…`.

### 2. Merge `feat/owned-syntax-ir` → master (engine)
- [ ] Only on explicit user request (CLAUDE.md: never push/merge to master unasked).
- [ ] Sequence after item 1 (grammar must be on GitHub `main` first, else CI breaks).

---

## P1 — Grammar follow-ups (we own the grammar; tracked in
`[[tree-sitter-al-grammar-issues]]` memory)

### 3. Broader slice of grammar issue #1 — junk `left`/`operator`/`right` fields
The case-pattern slice is fixed. The SAME pollution still leaks onto unrelated nodes:
`assignment` / `if` / `while` / `for` / `with` / `exit` / `asserterror` statements,
`argument_list`, `parenthesized_expression`, `statement_block`. A shared inlined
binary-or-`in` expression rule distributes `left`/`operator`/`right` field labels onto
every node that can contain an expression.

- [ ] Find the inline binary/`in`/`is`/`as` arms still embedded in statement/container
      rules (same pattern as the case fix: replace inline `seq(field('left'…)…)` with the
      named `$.in_expression` / `$.is_expression` / `$.as_expression` / the binary rules).
- [ ] `tree-sitter generate` clean; expect another large `node-types.json` pollution drop.
- [ ] Regen vocab; `cargo test --workspace` ZERO al-sem differential divergence.
- [ ] Add targeted parse/lowerer tests where a real bug was masked.
- [ ] Reviewer pass (gpt-5.5 + gemini-3.1-pro).

### 4. Grammar issue #4 — `trigger_declaration.name` includes the `::` token
`trigger_declaration.name` is `multiple:true` with type set `::,identifier,quoted_identifier`
(member triggers span `Object::Trigger`). An anonymous `::` inside a `name` field's type
set is odd.

- [ ] Consider a structured member-trigger-name node instead of a multi-token `name` field.
- [ ] Validate behavior-preserving via differential goldens.

---

## P2 — Tooling / hooks

### 5. Primary-repo `.claude` hook conflicts with CLAUDE.md rustfmt rule
The primary repo's `.claude` hook runs `cargo fmt` (whole-crate). CLAUDE.md mandates
per-file `rustfmt <file>`, NEVER `cargo fmt` (whole-crate churn). The new in-repo
`.claude/settings.local.json` (gitignored) already does the right thing: PostToolUse
per-file `rustfmt`, Stop-time `clippy` + `test --workspace`.

- [ ] Reconcile: either remove/replace the primary-repo `cargo fmt` hook, or align it to
      per-file `rustfmt`. Confirm which hook config is authoritative (global vs project vs
      local) so they don't fight.

---

## P3 — Deferred / optional (reviewer-noted, no deadline)

### 6. Real-`unknown` ratchet dashboard
The moat metric is the real-`unknown` edge rate on real BC apps. A ratchet dashboard via
`aldump --l3-unknown-breakdown` would track it over time.

- [ ] Needs a real BC workspace to measure against (not available in-repo).
- [ ] See `[[call-graph-resolution-redesign]]` memory for the honest taxonomy.

### 7. Full edition-2024 reformat (298 hunks)
The edition-2024 upgrade applied semantic migrations but NOT the 298 pure-reformat hunks
(per CLAUDE.md never-`cargo fmt` rule). Normalization happens incrementally via the
per-file `rustfmt` PostToolUse hook as files are touched.

- [ ] No action unless we decide to bulk-normalize (would be one big mechanical commit —
      only if the incremental drip is judged too slow).

---

## Quick reference — verification commands
```bash
# Freshness gate (generated vocab matches grammar):
cargo run -p xtask -- gen-syntax && git diff --exit-code crates/al-syntax/src/raw/generated/

# Full validation (incl. al-sem differential goldens):
cargo test --workspace

# Dump a real grammar tree:
cd tree-sitter-al && tree-sitter parse <file.al>

# Regen Rust-owned goldens after an intentional improvement:
REGEN_TEMP_GOLDENS=1 cargo test
```
