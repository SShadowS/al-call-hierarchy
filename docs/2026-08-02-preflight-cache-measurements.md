# Preflight verdict cache — measurement ledger

Implements `docs/superpowers/specs/2026-08-01-preflight-verdict-cache.md` against
the attribution in `docs/2026-07-31-preflight-census.md`.

## MEASURED — paired, alternating, 3 pairs per corpus

A = master (`2e041c6`, cache-disabled), B = master + this change with a warm cache.

| corpus | `analyze.total` A | `analyze.total` B | per-pair B/A | median |
|---|---|---|---|---:|
| **DO** (real customer workspace) | 3,155 / 3,133 / 3,386 ms | 1,003 / 1,031 / 1,052 | 0.318 / 0.329 / 0.311 | **−68.2 %** |
| 8020 (BC Base App) | 39,708 / 39,739 / 40,825 ms | 37,057 / 36,474 / 37,937 | 0.933 / 0.918 / 0.929 | **−7.1 %** |

`preflight.*` total, which is what the change actually targets:

| corpus | A | B | Δ |
|---|---|---|---:|
| DO | 2,614 / 2,602 / 2,772 ms | 474 / 476 / 510 | **−82 %** |
| 8020 | 3,719 / 3,669 / 3,661 ms | 487 / 483 / 466 | **−87 %** |

`parse_snapshot`, `dep_layer`, `assemble_graph`, `resolve_full` and `ctx_drop` all
go to **0 ms** on a hit; the residual is `snapshot_build`, the accepted floor.

**Why the two whole-run numbers differ so much is the point, not a caveat.** The
preflight is 83.4 % of a DO run and 10.8 % of an 8020 run, so an ~85 % cut to it
lands as −68 % on the corpus that represents a real customer and −7 % on the
synthetic one. Every previous arc in this track was tuned against 8020; this is
the first lever that is worth ~10× more on the shape the product actually ships
against.

## What it does NOT do

**It does not help the edit loop.** The key covers primary source content, so any
edit to the workspace is a guaranteed miss and a full recompute — verified
executably by `editing_the_workspace_never_serves_the_stale_verdict`. This is an
identical-input rerun win: CI re-runs, a second invocation for another `--format`,
a re-run after a no-op. The edit-loop lever is the dependency parse-artifact cache
(spec §6), which is a separate and larger design.

## Byte identity — cold, warm, AND disabled

The merge gate gained a new required form here, because a cached verdict is
formatter-visible output (`opaque_apps` flows into the JSON coverage block):

| corpus | cold | warm | `ALSEM_NO_PREFLIGHT_CACHE=1` |
|---|---|---|---|
| 8020 | `36151bf6…` | `36151bf6…` | `36151bf6…` |
| DO | `f022f677…` | `f022f677…` | `f022f677…` |

All six exact, and equal to the frozen gate hashes. A cold run and a warm run are
byte-identical, which is the property the whole design rests on.

## The discrimination proofs — and the three tests they killed

Every proof: break the thing, run the named test, expect FAIL; revert, expect
PASS; assert the source is restored byte-for-byte. Each scripted break asserts its
own match count first — an unasserted break proves nothing, and its green run
reads exactly like a pass.

| broken | test | broken | restored |
|---|---|---|---|
| length prefix in `put` | `key_changes_when_a_path_boundary_shifts_into_the_text` | FAIL | PASS |
| `virtual_path` in the walked fold | `key_changes_when_a_file_is_renamed` | FAIL | PASS |
| self-hash verification in `lookup` | `self_hash_mismatch_recomputes` | FAIL | PASS |
| binary-identity check in `lookup` | `entry_from_a_different_binary_is_rejected` | FAIL | PASS |
| body bytes in the walked fold | `key_changes_when_a_file_body_changes` | FAIL | PASS |

**The first run of these proofs came back GREEN-WHEN-BROKEN on three of the five.**
A passing proof is evidence about the TEST, not the code, so each was diagnosed
rather than accepted. All three were test defects, and all three are now fixed:

1. **The re-split row did not pin the length prefix.** The fold interleaves path
   and text, so `a.al`+head+`b.al`+tail and `a.al`+whole+`b.al` already differ in
   where `b.al` lands — the PATH carried that row. The doc comment claiming
   otherwise was an over-claim and is corrected in place; a new row
   (`key_changes_when_a_path_boundary_shifts_into_the_text`) hand-states the
   collision that does isolate the prefix: one file named `a.alb.al` versus two
   empty files `a.al`/`b.al`, which without length prefixes both fold to the bytes
   `a.alb.al`.
2. **The self-hash row's corruption destroyed its own oracle.** It rewrote
   `"unknown":999` → `7`, so with the self-hash check disabled the SERVED payload
   also failed `!= TRACER` and the row passed for the wrong reason — it could not
   tell "recomputed" from "served a corrupted entry". It now flips
   `coverage_holds` and leaves the tracer intact.
3. **The binary row was subsumed by the self-hash check.** Overwriting `binary`
   in place also invalidated the self-hash, so the entry was rejected one branch
   earlier and the row stayed green with the binary check disabled. The binary
   check's real job is not tamper-detection — it is rejecting a WELL-FORMED entry
   from another engine version. The replacement constructs exactly that: an entry
   whose `self_hash` is correctly computed over a foreign `binary`, plus an
   executable precondition that the same shape under THIS binary IS served.

This is the fourth arc in this repo where the discrimination requirement caught
something the suite could not. It cost three tests and bought three real ones.

## Gates

`scripts/check-goldens` green (9 targets, 0 failed, **zero files under `tests/`
moved**), **1,732** lib tests green (16 new), `cargo clippy --all-targets
--all-features` clean.

`scripts/cdo-gate` now exports `ALSEM_NO_PREFLIGHT_CACHE=1`: those are the
north-star zero-ratchets, and a warm replay would measure a previous run's verdict
and report it as this run's — precisely the laundering the cache's own rules exist
to prevent.
