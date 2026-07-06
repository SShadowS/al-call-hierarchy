# R0 differential goldens

These files are the **committed safety net** for the al-sem → Rust engine
migration. The default `cargo test` runs the `differential` harness
(`tests/differential.rs`) entirely **offline**: it parses the in-repo source
fixtures under `tests/r0-corpus/` with the Rust `snapshot_workspace()` and
asserts the resulting identity subset matches the goldens here, field for field.

No Bun and no al-sem checkout are required for the default test. Goldens are
Rust-owned baselines (the al-sem TS oracle is retired).

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

The harness asserts a direct, strict field-for-field match against these
goldens — no allowlist/tolerance mechanism; any divergence fails the test.

## Provenance (this snapshot)

| field | value |
| --- | --- |
| al-sem git sha | `f0ae38cc1a80a1c72e4a1a2eb0443f522ea08ded` |
| tree-sitter-al tag | `v2.5.2-shim` |
| snapshot schema | `3` (`return-type-aware` fingerprints) |
| modelInstanceId | `r0` |

The authoritative values live in `manifest.json` — this table is a convenience
copy. Scope for R0/Task 5: **ws-d2 only**. Task 7 widens to the full corpus.

## Refreshing the goldens (regen mode)

The al-sem TS oracle is retired; goldens are Rust-owned baselines. Rebaseline
by setting `REGEN_TEMP_GOLDENS=1`, which makes the harness write the ENGINE's
own output over the golden files instead of asserting against them:

```bash
REGEN_TEMP_GOLDENS=1 cargo test --test differential
```

This does **not** auto-commit — inspect the diff and commit deliberately.
