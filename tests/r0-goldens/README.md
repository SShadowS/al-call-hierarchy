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
  `modelInstanceId` (`r0`), and per-fixture counts. It documents exactly which
  al-sem state these goldens were originally generated from. `differential_
  identity_subset_matches_goldens` asserts the discovered golden count is at
  least the manifest's `fixtureCount` (Task T0.6) — a floor, not an exact
  match, since `fixtureCount` is a frozen al-sem-era snapshot and this corpus
  may only grow from here.
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
by setting `REGEN_TEMP_GOLDENS=1` (the exact string `"1"` — `"0"`, an empty
value, or any other value does **not** trigger regen; see Task T0.6 /
`tests/common/regen.rs`), which makes the harness write the ENGINE's own
output over the golden files instead of asserting against them:

```bash
REGEN_TEMP_GOLDENS=1 cargo test --test differential
```

This does **not** auto-commit — inspect the diff and commit deliberately.
`tests/differential.rs` bundles five sub-harnesses behind this one binary (R0
identity, R1a L2 features, R2a L3 record-types, R2b L3 call-graph, R2d L3
coverage all regenerate this way); R2c L3 event-graph and R0 identity gained
their regen path in Task T0.6 (previously neither had one, despite this
section's claim — regenerating them silently no-opped).

**Known regen≠assert divergence (NOT rebaselined — needs its own investigation):**
`ws-interface-dispatch` carries two `Interface` objects (`IEmpty`, `IProcessor`)
that collide on the same `stableObjectId` — AL interfaces have no object
number, so `engine::snapshot`'s `object_number = obj.id.unwrap_or(0)` assigns
every interface in a workspace `0`. The default field-for-field comparison
doesn't catch the collision because it keys both sides by `stableObjectId`
into a map, silently dropping one side.

That alone would just mean "one golden went stale, regen fixes it" — but it's
worse: **the regen output for this fixture is not reproducible.** Running
`differential_identity_subset_matches_goldens` ALONE (`cargo test --test
differential differential_identity_subset_matches_goldens -- --exact`) writes
`IEmpty.signatureFingerprint = db72b899…`, which is the mathematically correct
`sha256("Interface|0|IEmpty")`. Running the SAME test as part of the full
`differential.rs` binary (all 15 tests, the normal `cargo test` path) writes
`IEmpty.signatureFingerprint = 1a5af2b8…` instead — `IProcessor`'s hash, not
IEmpty's — reproducing the value currently committed. Both outcomes are 100%
stable across repeated trials within their own mode; the mode alone (isolated
vs. batched) decides which value comes out. `object_signature_fingerprint`
(`sha256("{type}|{number}|{name}")`) is a pure function and `snapshot_workspace`
processes files strictly sequentially over an already-sorted list, so neither
explains the split — the cause is upstream (`al_syntax::parse` /
`extract_from_ir`'s object extraction for `Interface` kind) and unidentified.
Regenerating this golden in EITHER mode would just pick one arbitrary side of
a live nondeterminism bug, not fix anything — left untouched pending a
dedicated investigation of the two open questions: the id-collision itself
(a real identity gap for any multi-interface workspace) and, more urgently,
why this fixture's parse output depends on unrelated prior test execution in
the same process.
