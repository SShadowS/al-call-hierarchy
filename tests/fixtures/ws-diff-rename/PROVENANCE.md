# Provenance

This fixture tree is a byte-for-byte copy, not a live oracle.

- **Source path:** `U:\Git\al-sem-OBOLETE\test\fixtures\ws-diff-rename`
- **al-sem HEAD:** `cfea6149c1ed912f1a10fa45eb4a755302327c60`
- **Copy date:** 2026-07-05
- **Verification:** each file's SHA-256 matches
  `.superpowers/sdd/alsem-witness/fixture-listings/ws-diff-rename+removed-field.sha256.txt`

`al-sem-OBOLETE` is a frozen, read-only archive checkout — it is never a live
oracle for this repo. This tree is vendored so `tests/cli_b_diff_differential.rs`
is self-contained and requires no sibling al-sem checkout.
