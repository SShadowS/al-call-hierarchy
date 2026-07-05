# Provenance

This directory holds two different kinds of file, vendored/generated at
Task 3.3 (al-sem parity retirement):

- **INPUTS** (fixed test data, never regenerated): `snapshots/*.snap.json`
  and `rename-overlay.json`. Copied byte-for-byte from
  `.superpowers/sdd/alsem-witness/scripts/cli-b-goldens/diff/` (al-sem HEAD
  `cfea6149c1ed912f1a10fa45eb4a755302327c60`, copy date 2026-07-05).
- **OUTPUTS** (Rust-owned baselines): every `.human.txt` / `.json` / `.sarif`
  / `.exitcode.txt` / `ws-mode-*.std{out,err}.txt` file. Regenerated from
  THIS engine (`REGEN_TEMP_GOLDENS=1 cargo test --test
  cli_b_diff_differential`) and witness-diffed — all 24 are byte-identical
  to the witness copy.

`al-sem-OBOLETE` (the source of the witness copies) is a frozen, read-only
archive — it is never a live oracle for this repo.
