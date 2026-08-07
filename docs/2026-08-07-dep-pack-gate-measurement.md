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

## Verdict: **PROCEED with postcard**

**Worst round observed across 9 full runs (54 rounds): 113.1 ms.** Typical round ~92 ms;
full observed range 66.8–113.1 ms. Against a ~200 ms proceed line and a ~600 ms switch
line, to save ~1,280 ms per hit. The shared string-table format stays in reserve and is
not needed.

The verdict is taken from the WORST round observed, not a median and not the best run —
and even that worst round is 1.8× under the proceed threshold and 5.3× under the switch
threshold. The margin therefore does not depend on the residual imprecisions listed under
*What this number does not cover*. Those are stated anyway, and one of them (a 58–87 ms
`Arc<DepMetaMap>` rebuild) is large enough to matter to the seam's own budget even though
it cannot change the format choice.

## Machine and build

| | |
|---|---|
| CPU | AMD Ryzen 9 9950X3D, 16 cores / 32 threads |
| RAM | 93.7 GB |
| OS | Windows 11 Enterprise 10.0.26200 |
| Artifact volume | `C:\Users\SShadowS\AppData\Local\Temp` (Samsung NVMe; 2,956 MB/s cold, 4,800 MB/s warm — measured, see *Cold cache* below) |
| Profile | `release-fast` (thin LTO, `codegen-units = 4`) — never a debug build |
| Pool | `big_stack::big_stack_pool()`, the same local rayon pool `snapshot::parse::parse_snapshot` installs into, not rayon's global pool |
| Date | 2026-08-07 |

**The machine was not quiet, and that is why the spread is what it is.** These runs shared
the box with a dozen sibling agent processes; measured baseline CPU load during the later
runs was 26 %. The first four runs landed at 66.8–84.0 ms and the last five at
80.9–113.1 ms with no code change to the decode path in between — contention, not a
regression. A quiet machine would sit at the bottom of the range; the ledger reports the
top of it, which is the conservative reading and the one the verdict uses. It also means
no figure below should be read as precise to better than about ±20 %.

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

### The population is ~5–9 % below the figures the plan quoted, and that is a workspace difference, not a snapshot defect

`docs/2026-08-02-dep-parse-sizing.md` recorded 11,856 dependency files / 11,165
`dep_objects` / 126,640 `dep_routines` on DO — the same three metrics, read from the same
`DepLayer` fields. This run measures 10,800 / 10,662 / 119,773: −8.9 % / −4.5 % / −5.4 %.

That gap was chased before the number was accepted, because the brief's instruction was
to stop if the snapshot is not seeing what the plan assumed. It is seeing all of it:

- Unzipping every `.app` in `.alpackages` directly and counting `.al` entries gives
  **10,800**, matching the snapshot's dependency file count **exactly**, app for app. The
  engine ingests 100 % of the dependency source physically present. Nothing is dropped.
- The primary app contributes 552 files, so the whole snapshot is 11,352 — still 504 short
  of 11,856 even counting the primary. Both `.app` packages sitting in the workspace root
  hold 552 embedded `.al` files each, i.e. the primary's own source, so neither accounts
  for the gap either.

The conclusion is that DO's `.alpackages` contents differ from the checkout the August 2
sizing ran against; the reconstruction of that state is not available and is not worth
recovering. What matters for this gate is that the measured population is complete with
respect to the workspace as it stands, and that a 5–9 % population difference cannot
close a 1.8× margin: at the plan's larger 126,640 routines the worst round would scale to
roughly 120 ms, still inside the proceed band. **That scaling remark is context for the
margin, not an adjustment — the verdict is taken from the measured 113.1 ms.**

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
non-empty before it measures anything.

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
both and taking the larger keeps the decision on measured ground rather than on an
estimate of what framing "would have" cost.

Measured, per-file framing costs 0.42 MB over 7,407 extra file frames — about 59 bytes
each — and it does not cost time: shape A decoded faster than shape B in 8 of the 9 runs,
because many moderate `Vec`s allocate better than three enormous ones. Shape B is
therefore the conservative shape on bytes-per-time, though the single worst round overall
came from shape A, and the verdict takes the worst round of either.

One residual: the per-file grouping is reconstructed from `RoutineMeta::virtual_path`, the
only path any packed record type carries, so the 3,384 dependency files that declare no
routines never become buckets — shape A has 7,416 frames, not 10,800. At the measured
59 bytes per frame that is ~0.20 MB (0.6 %) of unrepresented framing, and since framing
measured as a time *win*, it cannot push the number up.

## The measurement

Three rounds per shape per run. Each round is the hit-path shape end to end: parallel
read + `decode` (schema check, blake3 verify, postcard parse) + `AppRef` re-intern,
across per-app pack files, on `parse_snapshot`'s own pool.

**Every round of every run, in order.** Nothing here is a median, nothing is rounded in
the favourable direction, and no run is dropped.

| run | A r0 | A r1 | A r2 | B r0 | B r1 | B r2 | seam map |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 78.5 | 76.6 | 79.3 | 80.6 | 79.4 | 83.9 | — |
| 2 | 78.1 | 71.7 | 68.9 | 81.6 | 76.8 | 81.2 | — |
| 3 | 79.4 | 74.9 | 72.3 | 83.0 | 77.1 | 81.0 | — |
| 4 | 72.2 | 70.1 | **66.8** | 74.0 | 84.0 | 83.1 | 58.0 |
| 5 | 96.6 | 99.3 | 98.8 | 103.5 | 108.6 | 102.4 | 86.8 |
| 6 | 94.9 | 93.2 | 88.6 | 94.6 | 95.7 | 94.3 | 71.5 |
| 7 | 98.5 | **113.1** | 80.9 | 90.8 | 97.3 | 97.7 | 80.3 |
| 8 | 97.7 | 88.9 | 89.2 | 96.7 | 95.1 | 96.0 | 73.8 |
| 9 | 94.6 | 98.7 | 93.6 | 95.4 | 92.6 | 91.1 | 78.5 |

All times in ms. Runs 1–3 predate the seam-cost probe, which is printed outside the timed
rounds and cannot affect them; the decode path is byte-identical across all nine runs.
Runs 1–4 were taken before the machine picked up sustained sibling load (see *Machine and
build*), which is the whole of the step between run 4 and run 5.

| | shape A | shape B | overall |
|---|---:|---:|---:|
| best round | 66.8 | 74.0 | **66.8** |
| worst round | 113.1 | 108.6 | **113.1** |

**The verdict uses 113.1 ms**, the single worst round observed anywhere in the sample.

### Where the time goes

| component | ms |
|---|---:|
| read only, warm page cache (9 files, 33 MB, parallel) | 7.0–8.9 |
| blake3 over every pack, parallel | 3.1–3.4 |
| **Base Application pack alone (decode + re-intern)** | **65.8–101.3** |
| System Application pack | 6.8–8.7 |
| Continia Delivery Network pack | 3.8–4.7 |

**The wall time is one pack.** Packs are per-app files decoded concurrently, and Base
Application is 100,944 of the 119,773 routines — 84 % of the population in a single
serial unit. It accounts for 90–99 % of the round's wall time in every run but one (71 %
in a single outlier round). Two consequences worth recording before step 5 is built:

- More cores will not help. The parallelism the spec leaned on to carry postcard is real
  but nearly exhausted at nine packs of wildly unequal size; the critical path is
  effectively serial.
- The number scales with Base Application, roughly one-for-one, not with workspace size.
  A future BC release with a larger Base Application moves this figure directly. At
  1.8× headroom on the worst observed round that is a comfortable position, not a fragile
  one, but it is the variable to watch — re-run the gate on a major BC version bump.

I/O and integrity are both negligible: verification costs ~3 ms of a ~90 ms load, which
also bounds what any format change could win. A shared string table would change the
parse, not the hash and not the read.

## Cold cache

Spec §13 asks for a cold OS file cache at least once. **All rounds reported above read
from a warm page cache** — the bench writes the packs and reads them moments later, and
Windows offers no user-mode way to evict them. Rather than claim a cold round that was
not cold, the penalty was measured directly on the artifact volume: sequential reads of
three large previously-untouched files averaged **2,956 MB/s cold**, and re-reading the
same file immediately gave **4,800 MB/s**, which confirms the first reads were genuinely
cold rather than merely slow.

A cold read of the 33.25 MB artifact therefore costs ~11.5 ms against the ~7 ms already
inside the measured rounds: **a cold first round adds roughly 4 ms.** Immaterial at this
scale, and the verdict is unchanged whether the cache is warm or cold.

## What this number does not cover

It is a **pack-load** number, not a full cache-hit cost. Named explicitly so no one reads
it as the latter:

1. **The engine-side `AppRef` re-intern does not exist yet** — that is the ingestion seam,
   spec step 5. The bench performs its own remap of all three id sites so the cost is
   inside the measurement, but what it times is the bench's loop, not a shipped loader.
2. **Rebuilding `Arc<DepMetaMap>` costs 58.0–86.8 ms and is NOT in the gate number.** A
   pack stores the `RoutineMeta` tier as a `Vec`; `DeclSurface`/`LspSnapshot` need a
   `HashMap<RoutineNodeId, RoutineMeta>`, and hashing a `String`-bearing key 119,773 times
   is not free. Measured separately (6 runs) and excluded from the verdict because the
   format choice does not turn on it — a shared string table would not make a `HashMap`
   build any faster. It is, however, real work step 5 must budget: **~200 ms all-in on the
   worst observed pairing, against ~1,280 ms saved.** Still a decisive win, but not the
   ~90 ms headline, and step 5 should look at whether the seam can consume the `Vec`
   directly or build the map in parallel rather than inherit this serially.
3. **The pack key and its computation** (spec §7) and the `EXTRACTION_FINGERPRINT` canary
   (§8) are step 4 and contribute nothing here.
4. **`build_dep_layer`'s Step 4 sort and dedup still run on a hit** — spec §6 stores packs
   pre-dedup precisely so they do. Unchanged by packs, so not a new cost, but also not
   part of the saving.

Setup (snapshot build, `parse_snapshot`, `build_dep_layer`, `assemble_program_graph`,
`DeclSurface::build_split`) took ~2.2 s and is the *miss* path — it is what a hit replaces,
not part of what a hit costs.

## Decision

**PROCEED with postcard.** Spec steps 4, 5 and 6 are built against this format. The
shared string-table alternative in §13 is not needed and is left in reserve; re-open it
only if a re-run on a materially larger Base Application approaches the switch line.
