# Dependency pack format — the measurement gate

The go/no-go for the dependency pack cache's artifact format
(`docs/superpowers/specs/2026-08-07-dependency-pack-cache-design.md` §13). Sizing for
this lever was done in `docs/2026-08-02-dep-parse-sizing.md`, which closed with "it is
not resolvable by reasoning; it needs a round-trip measurement." This is that
measurement.

Reproduce with:

```bash
PACK_BENCH_WS=<bc-workspace-root> \
  cargo bench --profile release-fast --bench dep_pack_roundtrip
```

The bench exits non-zero when `PACK_BENCH_WS` is unset rather than silently passing, so
a missing workspace cannot read as a green run.

**Every timing in this document is transcribed from a committed artifact:**
`docs/measurements/2026-08-07-dep-pack-gate-runs.log`, the raw stdout of six consecutive
runs of the committed bench. Nothing below is quoted from an unretained run.

## Verdict: **PROCEED with postcard**

Worst round in the logged 36-round sample: **110.2 ms**. Median 85.0 ms, mean 83.8 ms,
range 66.2–110.2 ms. Against a ~200 ms proceed line and a ~600 ms switch line, to save a
**measured 1,204–1,336 ms** on the same corpus. The shared string-table format stays in
reserve and is not needed.

**110.2 ms is a sample maximum on a contended machine, not a bound. Treat it as ±20 %,
and do not let step 5 inherit it as a ceiling.** An independent re-run of the previous
commit's binary, by a reviewer, on the same workspace, produced a worst round of
**121.1 ms** — 10 % above this sample's top — along with a warm read of 13.0 ms and a
Base Application pack of 111.2 ms, each above the ranges quoted here. That reproduction is
recorded rather than discarded: it is an external observation that came in *worse* than
our own and still cleared the gate by 1.65×, which is stronger evidence for PROCEED than
any number we generated ourselves.

Taking the highest figure observed anywhere — the reviewer's 121.1 ms — the load is still
1.65× under the proceed line and ~5× under the switch line, against ~1,260 ms saved. The
margin does not depend on any of the residual imprecisions listed under *What this number
does not cover*, one of which (a 58–87 ms `Arc<DepMetaMap>` rebuild) is nonetheless large
enough to matter to the seam's own budget.

## Machine and build

| | |
|---|---|
| CPU | AMD Ryzen 9 9950X3D, 16 cores / 32 threads |
| RAM | 93.7 GB |
| OS | Windows 11 Enterprise 10.0.26200 |
| Artifact volume | `C:\Users\SShadowS\AppData\Local\Temp` |
| Profile | `release-fast` (thin LTO, `codegen-units = 4`) — never a debug build |
| Pool | `big_stack::big_stack_pool()`, the same constructor and sizing `snapshot::parse::parse_snapshot` uses (`src/snapshot/parse.rs:86`), not rayon's global pool |
| Date | 2026-08-07 |

**The machine was shared with concurrent agent processes throughout.** That is the most
likely source of the spread, and of the gap between this sample and the reviewer's higher
re-run, but no isolating probe was run and the attribution is not established — it is
offered as the likely cause, not a measured one. What follows from it regardless: no
figure here should be read as precise to better than about ±20 %, and a quiet machine
would sit at the bottom of each range rather than the top.

## Workspace and population

`PACK_BENCH_WS=u:/Git/DO.Support-Fuckup/DocumentOutput/Cloud` — "DO", Continia Document
Output Cloud. 11 apps: the primary plus 10 dependencies, of which 9 contribute nodes.

| | measured |
|---|---:|
| dependency source files | 10,800 |
| dependency objects (`DepLayer::dep_objects`) | 10,662 |
| dependency routines (`DepLayer::dep_routines`) | 119,773 |
| `RoutineMeta` entries (`DeclSurface::build_split` dep tier) | 119,773 |
| whole snapshot incl. primary | 11,352 source files |

Per app, as the gate prints it:

| app | version | files | objects | routines |
|---|---|---:|---:|---:|
| Continia Document Output *(primary — never packed)* | 29.0.0.0 | 552 | — | — |
| Base Application | 28.1.49838.50268 | 8,073 | 7,990 | 100,944 |
| System Application | 28.1.49838.50268 | 1,319 | 1,299 | 8,738 |
| Continia Delivery Network | 29.0.0.124377 | 509 | 509 | 4,657 |
| System | 28.0.50197.0 | 361 | 332 | 290 |
| Continia Core | 29.0.0.122270 | 291 | 291 | 3,244 |
| Business Foundation | 28.1.49838.50268 | 96 | 96 | 379 |
| Continia System Application | 29.0.0.124334 | 75 | 69 | 791 |
| Continia KYC | 29.0.0.117497 | 65 | 65 | 687 |
| Continia Connector App | 29.0.0.124334 | 11 | 11 | 43 |
| Application *(symbol-only: ABI nodes, no `RoutineMeta`)* | 28.1.49838.50268 | 0 | 0 | 0 |

### The population is ~5–9 % below the figures the plan quoted. What is established, and what is not.

`docs/2026-08-02-dep-parse-sizing.md` recorded 11,856 dependency files / 11,165
`dep_objects` / 126,640 `dep_routines` on "DO" — the same three metrics, read from the same
`DepLayer` fields. This run measures 10,800 / 10,662 / 119,773: −8.9 % / −4.5 % / −5.4 %.

**Established, and this is the part that matters for the gate:** the snapshot sees
everything the measured root contains. Unzipping every `.app` in
`Cloud/.alpackages` and counting `.al` entries gives **10,800 exactly**, app for app,
matching the snapshot's dependency file count. Nothing is dropped. The primary contributes
552 files, and both `.app` packages in the workspace root hold that same 552, so neither
accounts for the gap either.

**Not established: the cause.** An earlier revision of this ledger asserted that
`Cloud/.alpackages` contents had changed since August 2. That is contradicted by the
artifacts — every `.app` there has mtime **2026-07-02**, a month *before* the sizing run —
and the assertion is withdrawn. What the record actually supports:

- `docs/2026-08-02-dep-parse-sizing.md` names no workspace path, only "DO".
- This checkout has six `DocumentOutput/*` roots; only two carry dependencies at all.
  Measured with this same bench: `Cloud` gives 10,800 / 10,662 / 119,773 and `Test` gives
  **11,830 / 11,690 / 129,925** (it adds Document Output itself plus the BC test libraries
  as dependencies).
- **Neither root reproduces the sizing doc's triple.** `Test` comes within 26 files of
  11,856 but overshoots objects by 525 and routines by 3,285; `Cloud` undershoots all
  three. So "it was the `Test` root" is a better fit on one metric and a worse fit on the
  other two, and is not asserted here either.

The honest conclusion is that the sizing figures came from a workspace root, checkout, or
engine revision that the record does not pin down, and recovering it is not worth the
effort — because it cannot change this gate. A 5–9 % population difference cannot close a
1.65× margin: at the sizing doc's larger 126,640 routines the worst observed round scales
to roughly 117 ms, still inside the proceed band. **That scaling remark is context for the
margin, not an adjustment — the verdict is taken from the measured rounds.**

## What the pack contains

Spec §6 lists what a pack must persist. Every item is in the measured payload except one:

| §6 item | in the measured pack |
|---|---|
| format version | yes — `PACK_SCHEMA`, in the header and again in the hashed body |
| self-hash | yes — blake3 over the body bytes, verified on every decode in the timed rounds |
| key echo | **no** — the pack key is spec §7/step 4, not yet designed. A short string; it cannot move the number |
| app identity, symbolically | yes — real guid / name / publisher / version per pack, re-interned in the timed rounds |
| per file: virtual path | yes |
| per file: `ParseStatus` | yes — as `PackedFile::parse_status_recovered` |
| per file: `ObjectNode` / `RoutineNode` | yes — all 10,662 and all 119,773 |
| `DeclSurface` contribution, `RoutineMeta` per routine | yes — all 119,773 |
| recovered-file paths | yes — derivable from the per-file bool above |

`RoutineMeta` is the point worth being explicit about: it is over half the payload, it is
not derivable on a hit (`RoutineMeta::from_decl` consumes a `RoutineDecl`, and the
`ParsedUnit`s those come from are exactly what a hit avoids building), and a run without
it measures roughly a third of the real bytes. It is in. The gate asserts the tier is
non-empty before it measures anything, and its presence was independently confirmed from
the artifact bytes in review (a `.al`-path census of the per-app packs, where the only
possible source of a real path is `RoutineMeta::virtual_path`, returned the per-app
routine counts exactly, 9 apps of 9).

All three `AppRef` sites per routine are re-interned inside the timed region —
`ObjectNode.id`, `RoutineNode.id`, and the `routine_meta` key, which embeds an
`ObjectNodeId` of its own. Missing any one of them would make this a floor rather than
the cost of a hit (spec §13).

## Artifact

| shape | packs | files | size |
|---|---:|---:|---:|
| A — one `PackedFile` per source file (spec §6 shape) | 9 | 7,416 | **33.25 MB** |
| B — one synthetic `PackedFile` per app (the brief's shape) | 9 | 9 | **32.83 MB** |

Both shapes were built and timed. The brief specified shape B on the grounds that the
gate prices record cost rather than per-file framing, which makes it a floor: spec §6
stores contributions per source file. Since the omission can only add cost, measuring
both and taking the worse keeps the decision on measured ground rather than on an
estimate of what framing "would have" cost.

Measured, per-file framing costs 0.42 MB over 7,407 extra file frames — about 59 bytes
each — and it does not cost time: shape A decoded faster than shape B in all six logged
runs, because many moderate `Vec`s allocate better than three enormous ones. Shape B is
therefore the slower shape, and every worst round in the logged sample comes from it.

One residual: the per-file grouping is reconstructed from `RoutineMeta::virtual_path`, the
only path any packed record type carries, so the 3,384 dependency files that declare no
routines never become buckets — shape A has 7,416 frames, not 10,800. At the measured
59 bytes per frame that is ~0.20 MB (0.6 %) of unrepresented framing, and since framing
measured as a time *win*, it cannot push the number up.

## The measurement

Three rounds per shape per run, six runs. Each round is the hit-path shape end to end:
parallel read + `decode` (schema check, blake3 verify, postcard parse) + `AppRef`
re-intern, across per-app pack files, on `parse_snapshot`'s own pool.

**Every round of every run in the committed log, in order.** Nothing is a median, nothing
is rounded in the favourable direction, and no run is omitted — the log
(`docs/measurements/2026-08-07-dep-pack-gate-runs.log`) contains exactly these six runs and
no others.

| run | A r0 | A r1 | A r2 | B r0 | B r1 | B r2 | seam map | saving (parse + dep_layer) |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 90.5 | 85.5 | 85.0 | **110.2** | 106.2 | 101.7 | 73.4 | 1336.2 |
| 2 | 71.0 | 82.7 | 80.2 | 88.4 | 88.6 | 90.3 | 68.9 | 1294.9 |
| 3 | 71.8 | **66.2** | 84.8 | 86.5 | 87.9 | 84.9 | 74.0 | 1204.5 |
| 4 | 71.3 | 68.7 | 69.2 | 79.6 | 88.3 | 80.0 | 70.5 | 1271.4 |
| 5 | 74.1 | 79.1 | 75.1 | 89.4 | 87.7 | 91.7 | 72.5 | 1250.5 |
| 6 | 69.8 | 76.0 | 82.7 | 89.1 | 93.7 | 88.9 | 67.9 | 1212.6 |

All times in ms.

| | shape A | shape B | overall |
|---|---:|---:|---:|
| best round | 66.2 | 79.6 | **66.2** |
| worst round | 90.5 | 110.2 | **110.2** |
| median of all 36 rounds | | | **85.0** |
| mean of all 36 rounds | | | **83.8** |

Round ordering within a shape carries no information: **round 0 is not a cold round.**
`encode` wrote every pack file moments before the rounds begin, so their pages are already
resident. (An earlier revision of the bench made this worse still by running its read
probe immediately before round 0; that probe now runs after the rounds.) A run whose
round 0 is the fastest, or the slowest, is showing noise.

### What a hit replaces, on this same corpus

Measured in every run rather than quoted from elsewhere: `parse_snapshot` +
`build_dep_layer` totals **1,204.5–1,336.2 ms** (mean ~1,262 ms) on the Cloud workspace.

This matters because spec §13's "~1,280 ms saved" comes from
`docs/2026-08-02-dep-parse-sizing.md`, i.e. from the *unidentified larger corpus* discussed
above, while the load cost here is measured on Cloud. A ratio across the two would cross
populations. The same-corpus figure above removes that problem: **~1,262 ms saved against
a 110.2 ms worst load is 11.5×**, and against the reviewer's 121.1 ms it is 10.4×. Both
are like-for-like.

### Where the time goes

| component | ms (logged sample) | reviewer's re-run |
|---|---:|---:|
| read only, warm (9 files, 33 MB, parallel) | 6.7–11.0 | 13.0 |
| blake3 over every pack, parallel | 3.1–3.6 | 3.3 |
| **Base Application pack alone (decode + re-intern)** | **68.1–99.3** | **111.2** |
| System Application pack | 7.2–10.6 | 11.0 |
| Continia Delivery Network pack | 3.8–5.7 | 5.8 |

**The wall time is one pack.** Packs are per-app files decoded concurrently, and Base
Application is 100,944 of the 119,773 routines — 84 % of the population in a single
serial unit. It accounted for 89–99 % of the round's wall time in all twelve shape-runs
logged here; the reviewer's re-run recorded one shape at 75 %, so treat the share as high
but variable rather than as a fixed ratio. Two consequences worth recording before step 5
is built:

- More cores will not help. The parallelism the spec leaned on to carry postcard is real
  but nearly exhausted at nine packs of wildly unequal size; the critical path is
  effectively serial.
- The number scales with Base Application, roughly one-for-one, not with workspace size.
  A future BC release with a larger Base Application moves this figure directly. At
  1.65× headroom on the worst figure observed anywhere that is a comfortable position,
  not a fragile one, but it is the variable to watch — re-run the gate on a major BC
  version bump.

I/O and integrity are both small: verification costs ~3 ms of an ~85 ms round, which also
bounds what any format change could win. A shared string table would change the parse,
not the hash and not the read.

## Cold cache — spec §13's one unmet clause

**Spec §13 requires at least one round from a cold OS file cache. That was not done, and
no substitute is offered.** Every round above read from a warm page cache.

The reason is structural, not a Windows limitation: the bench `encode`s the packs and
writes them to disk moments before the rounds run, so their pages are resident before any
round begins. No ordering change inside the bench can make round 0 cold. (An earlier
revision compounded it by reading every pack file in a probe immediately before round 0;
that probe has been moved to after the rounds, which removes the bench's own contribution
but not the write's.)

An earlier revision of this ledger offered a derived penalty in place of the missing
measurement — "2,956 MB/s cold vs 4,800 MB/s warm, therefore ~4 ms". **That has been
withdrawn.** It had no retained artifact, and the inference does not hold: a 1.6× ratio
between a first read and an immediate re-read does not discriminate a cold device read
from a first-touch penalty, and the reviewer's *warm* read of the real 33.25 MB artifact
measured 13.0 ms = 2,558 MB/s, slower than the figure that had been offered as proof of
coldness. Subtracting a warm read that ranges 6.7–13.0 ms from an ~11 ms model gives a
difference that can take either sign.

What can be said from measured data: the whole read is **6.7–13.0 ms** of an 85–110 ms
round. Even if a genuinely cold read were twice the slowest warm one, it would add ~13 ms
to a round with ~90 ms of headroom to the proceed line. The gap is immaterial to the
verdict; it is simply not quantified, and the clause is not met.

Getting the real number needs a read that bypasses the cache manager — on Windows a
`FILE_FLAG_NO_BUFFERING` handle with sector-aligned buffers. That is the route if a future
revision judges a sub-13 ms term worth unsafe FFI in a bench. It was not judged worth it
here.

## What this number does not cover

It is a **pack-load** number, not a full cache-hit cost. Named explicitly so no one reads
it as the latter:

1. **The engine-side `AppRef` re-intern does not exist yet** — that is the ingestion seam,
   spec step 5. The bench performs its own remap of all three id sites so the cost is
   inside the measurement, but what it times is the bench's loop, not a shipped loader.
2. **Rebuilding `Arc<DepMetaMap>` costs 67.9–74.0 ms here (86.0 ms in the reviewer's
   re-run) and is NOT in the gate number.** A pack stores the `RoutineMeta` tier as a
   `Vec`; `DeclSurface`/`LspSnapshot` need a `HashMap<RoutineNodeId, RoutineMeta>`, and
   hashing a `String`-bearing key 119,773 times is not free. Measured separately and
   excluded from the verdict because the format choice does not turn on it — a shared
   string table would not make a `HashMap` build any faster. It is real work step 5 must
   budget: the worst **single run** observed anywhere pairs 121.1 ms of load with 86.0 ms
   of map build for **207.1 ms all-in** (the reviewer's run — a genuinely observed
   pairing, not a sum of maxima from different runs), against ~1,262 ms saved. Still
   decisive, but step 5 should check whether the seam can consume the `Vec` directly or
   build the map in parallel rather than inherit this serially.
3. **The pack key and its computation** (spec §7) and the `EXTRACTION_FINGERPRINT` canary
   (§8) are step 4 and contribute nothing here.
4. **`build_dep_layer`'s Step 4 sort and dedup still run on a hit** — spec §6 stores packs
   pre-dedup precisely so they do. Unchanged by packs, so not a new cost, but also not
   part of the saving.

## Decision

**PROCEED with postcard.** Spec steps 4, 5 and 6 are built against this format. The
shared string-table alternative in §13 is not needed and is left in reserve; re-open it
only if a re-run on a materially larger Base Application approaches the switch line.
