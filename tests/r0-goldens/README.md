# R0 differential goldens

These files are the **committed safety net** for the al-sem → Rust engine
migration. The default `cargo test` runs the `differential` harness
(`tests/differential.rs`) entirely **offline**: it parses the in-repo source
fixtures under `tests/r0-corpus/` with the Rust `snapshot_workspace()` and
asserts the resulting identity subset matches the goldens here, field for field.

No Bun, no al-sem checkout, and no `AL_SEM_DIR` are required for the default
test.

## Layout

- `tests/r0-goldens/<fixture>.golden.json` — the al-sem-produced identity subset
  (objects + routines: stable ids, names, kinds, signature fingerprints,
  normalized signature hashes, canonical signature text). The Rust harness
  deserializes these into the same `IdentitySnapshot` structs it produces.
- `tests/r0-goldens/manifest.json` — provenance pin. Records the al-sem git sha,
  tree-sitter-al tag + native sha256, snapshot schema version,
  `signatureFingerprintSemantics`, the cache-version tuple,
  `modelInstanceId` (`r0`), and per-fixture counts. The harness does not gate on
  it yet (Task 7 widens the corpus and may), but it documents exactly which
  al-sem state these goldens were generated from.
- `tests/r0-corpus/<fixture>/` — the **source-only** AL workspace
  (`app.json` + `src/**/*.al`) the goldens were derived from, so the harness has
  AL to parse offline.
- `KNOWN_DIVERGENCES.json` (repo root) — machine-readable allowlist. Empty for
  R0 (ws-d2 fully matches). See the harness header for gating semantics.

## Provenance (this snapshot)

| field | value |
| --- | --- |
| al-sem git sha | `f0ae38cc1a80a1c72e4a1a2eb0443f522ea08ded` |
| tree-sitter-al tag | `v2.5.2-shim` |
| snapshot schema | `3` (`return-type-aware` fingerprints) |
| modelInstanceId | `r0` |

The authoritative values live in `manifest.json` — this table is a convenience
copy. Scope for R0/Task 5: **ws-d2 only**. Task 7 widens to the full corpus.

## Refreshing the goldens (live mode)

Refresh is a separate, `#[ignore]`d test that runs ONLY when `AL_SEM_DIR` points
at an al-sem checkout. It regenerates the goldens from al-sem and copies the
source-only fixtures + goldens + manifest back into this repo. It does **not**
auto-commit — it leaves a reviewable diff.

```bash
AL_SEM_DIR=/u/Git/al-sem cargo test --test differential -- --ignored refresh_goldens_from_al_sem --nocapture
```

What it does:

1. `bun run scripts/dump-goldens.ts` inside `$AL_SEM_DIR` (regenerates
   `scripts/r0-goldens/*.golden.json` + `manifest.json`).
2. Copies each source-only `ws-*` fixture (`app.json` + `src/**`) into
   `tests/r0-corpus/`, and the matching `*.golden.json` + `manifest.json` into
   `tests/r0-goldens/`.
3. Prints the al-sem git sha, tree-sitter-al grammar sha, and this engine's
   commit sha for provenance.

After running, review the diff and commit deliberately.
