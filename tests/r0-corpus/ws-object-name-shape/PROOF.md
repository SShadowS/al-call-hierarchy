# Fixture — object-typed declared-var shape preservation (Task 2, mirrors I1)

**Status: FRESH-ENGINE-RUN.** Unlike most `r0-corpus/ws-*` fixtures (which are
spec-stated documentation for inline Rust unit tests), this directory is a real
workspace (`app.json` + 3 `.al` files) loaded end-to-end by
`resolve_full_program` in
`object_name_shape_quoted_digit_name_resolves_to_named_object_not_numeric_id`
(`tests/program_resolve_harness.rs`) — the AL source below is what the test
actually parses, not a paraphrase of it.

## The bug (pre-fix)

`resolve_object_name_lc` (`src/program/resolve/receiver.rs`) re-parsed an
ALREADY-unquoted object name string with `.parse::<i64>()`. `parse_object_kind_type`
had already stripped the quotes before that point, so `Codeunit 80` (a numeric id
reference) and `Codeunit "80"` (a codeunit literally NAMED `80`) both produced the
identical string `"80"`, which then both parsed as the numeric id `80` — the SAME
shape-loss bug I1 fixed for `Record`/`SourceTable`/`TableNo` (`ParsedType::Record`'s
`table_ref: ObjectRef`), just still open for the `ParsedType::Object` sibling.

## Fixture

| File | Object | What it proves |
|---|---|---|
| `RealById.Codeunit.al` | `codeunit 80 RealById` | The numeric-id target. Deliberately has NO `P()` — a pre-fix false resolution here is directly falsifiable (missing `P` ⇒ `Unknown`, not a silently-plausible wrong `Source`). |
| `Named80.Codeunit.al` | `codeunit 50100 "80"` | The NAME target — a codeunit literally named `"80"`, unrelated id. Declares `P()`. |
| `Caller.Codeunit.al` | `codeunit 50101 Caller` | `var C: Codeunit "80"; C.P();` — a QUOTED name reference, never a numeric one. |

## Expected (post-fix) resolution

`Caller.Trigger`'s `C.P()` call site classifies with `Evidence::Source`, target
`RouteTarget::Routine` whose `object.key == ObjKey::Id(50100)` (`Named80`,
routine `p`) — **never** `ObjKey::Id(80)` (`RealById`, which has no `P` to
resolve to in the first place).

## Pre-fix behavior (verified via a temporary revert during TDD)

`C: Codeunit "80"` unquoted to `"80"`, parsed as numeric id `80`, resolved to
`RealById` — which has no `P()` — producing a false `Unknown` edge instead of
the correct `Source` edge to `Named80.P`.
