# Resolution Push-to-Zero Implementation Plan (3 features)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Drive the deps-loaded real-unknown edge rate further toward zero by closing the three largest remaining buckets, validated against CDO (clean dep closure) — `non-object-receiver-type`, `compound-receiver` (single-hop subset), and report-dataitem `untracked-receiver`.

**Architecture:** Three independent features on the existing ReceiverType lattice (`src/engine/l3/receiver_type.rs` Phase A infer / Phase B dispatch) + the builtin catalog (`src/engine/l3/member_builtins.rs`). No new subsystem.

**Tech Stack:** Rust, `phf` perfect-hash catalogs, tree-sitter-al. Measure with `aldump --l3-call-graph-stats-cross-app` / `--l3-unknown-breakdown-cross-app` on `U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud` (CDO, clean deps — the source of truth) and `.../DocumentCapture/Cloud` (DC).

**Baseline (deps-loaded):** CDO 1.55% (212 unknown: non-object 68, untracked 81, compound 62). DC 2.10%.

**Source-of-truth caveat:** DC's compound/untracked are inflated by the missing Continia Document Output 29.0 dep. Prioritize by CDO's clean numbers; treat DC deltas as confirmation only.

---

## Feature A — AL platform-type builtin catalogs (the `non-object` win)

The `non-object-receiver-type` bucket is NOT a primitives floor — it is AL platform types with real builtin method surfaces that aren't wired as `ReceiverBuiltinKind`. The method catalogs ALREADY exist in `tools/gen-al-builtins/out/member_builtins.json` (keys: `Notification`, `ErrorInfo`, `ModuleInfo`, `RecordId`, `BigText`, `SecretText`, `DataTransfer`, `SessionSettings`, `Text`, `Label`, `Date`, `DateTime`, `Time`, `Guid`, `Integer`, `Decimal`, `Boolean`, `Duration`, `BigInteger`, `Byte`, `File`, `FileUpload`, `NumberSequence`, `Version`, `Cookie`, `FilterPageBuilder`, `Debugger`, `SessionInformation`, `CompanyProperty`, `ProductName`, …). Observed on CDO+DC (counts): Notification 77, Text 30, RecordId 17, ErrorInfo 19, ModuleInfo 13, BigText 9, SecretText 6, DataTransfer 5, SessionSettings 4.

**Files:** `src/engine/l3/member_builtins.rs` (enum + catalogs + classify_receiver + disposition). `tests/`.

### Task A1: Add platform-type catalogs + kinds

- [ ] **Step 1:** For EACH of these types, add a `ReceiverBuiltinKind` variant and a `phf_set!` catalog transcribed from the corresponding key in `tools/gen-al-builtins/out/member_builtins.json` (lowercase every method name): `Notification`, `ErrorInfo`, `ModuleInfo`, `RecordId`, `BigText`, `SecretText`, `DataTransfer`, `SessionSettings`, `Text` (also covers `Code` and `Label` — same string-method surface), `Date`, `DateTime`, `Time`, `Guid`, `Integer`, `Decimal`, `Boolean`, `Duration`, `BigInteger`, `Byte`, `File`, `FileUpload`, `NumberSequence`, `Version`, `FilterPageBuilder`, `SessionInformation`. Read the JSON for the exact method lists — do NOT hand-guess method names. (If `Code`/`Label` are absent as JSON keys, alias them to the `Text` catalog.)

- [ ] **Step 2:** Add a `member_builtin_disposition` match arm per new kind (`Kind => set_hit(&KIND, method_lc)`). Keep the match exhaustive.

- [ ] **Step 3:** Extend `classify_receiver` to map the declared-type FIRST token (lowercased) to each kind: `"notification" => Notification`, `"errorinfo" => ErrorInfo`, `"recordid" => RecordId`, `"moduleinfo" => ModuleInfo`, `"bigtext" => BigText`, `"secrettext" => SecretText`, `"datatransfer" => DataTransfer`, `"sessionsettings" => SessionSettings`, `"text" => Text`, `"code" => Text`, `"label" => Text`, `"date" => Date`, `"datetime" => DateTime`, `"time" => Time`, `"guid" => Guid`, `"integer" => Integer`, `"decimal" => Decimal`, `"boolean" => Boolean`, `"duration" => Duration`, `"biginteger" => BigInteger`, `"byte" => Byte`, `"file" => File`, `"fileupload" => FileUpload`, etc. NOTE `classify_receiver` already splits on the first space, so `Text[1024]` / `Code[20]` arrive as `text[1024]` — handle the `[` by splitting the first token on `[` too (so `text[1024]` → `text`). Add that normalization.

- [ ] **Step 4 (test):** A unit test in `member_builtins.rs` asserting `classify_receiver("Notification") == Some(Notification)`, `classify_receiver("Text[1024]") == Some(Text)`, and `member_builtin_disposition(Notification, "send") == Some(Builtin)`, `member_builtin_disposition(Text, "split") == Some(Builtin)`.

- [ ] **Step 5:** `cargo test`. Then release-build aldump and measure both apps. Expected: CDO non-object 68 → small residual (only genuinely-uncatalogable types); a `framework-method-not-in-catalog` bucket may appear for any method the JSON catalog lacks — that is HONEST (a real catalog gap), list it. realUnknownRate drops on both.

- [ ] **Step 6:** Commit `feat(engine-d22): AL platform-type builtin catalogs (Notification/ErrorInfo/Text/RecordId/…) — non-object-receiver win`. Include the probe tag already on `ReceiverType::Primitive` (declared-type::method receiver_shape).

NOTE: the `Primitive` Phase-A path now only fires for a type `classify_receiver` STILL rejects; those stay `non-object-receiver-type` (honest floor) and the receiver_shape tag names them. No regression risk — extending `classify_receiver`/catalogs only converts previously-unknown edges to `builtin`.

---

## Feature B — Report-dataitem implicit `Rec`

A report's dataitem triggers (`OnAfterGetRecord`, `OnPreDataItem`, …) operate on an implicit `Rec` typed to that dataitem's source table. Today only Table/Page/extension methods seed implicit `Rec` (`l2/mod.rs::implicit_base_receiver`), so report-dataitem `Rec.<field>` / bare calls fall to `untracked-receiver`.

**Files:** `src/engine/l2/mod.rs` (+ wherever object/dataitem metadata is parsed), grammar inspection for the `dataitem` node, `src/engine/l3/record_types.rs` (table-id backfill), tests.

### Task B1: Investigate dataitem structure (no code)

- [ ] **Step 1:** Find the tree-sitter-al `dataitem` node shape: `rg -n "dataitem" tree-sitter-al/grammar.js`. Determine how a dataitem declares its name + source table (`dataitem(Name; "Source Table")`), and how a routine/trigger is associated with a dataitem (nesting). Report the node kinds + fields. This drives B2's seeding. If reports nest dataitems and each trigger lives under its dataitem, the implicit `Rec` table = that dataitem's source table.

### Task B2: Seed implicit `Rec` for report-dataitem triggers

- [ ] **Step 1 (test):** `tests/` — a report with `dataitem(Cust; Customer) { trigger OnAfterGetRecord() begin if Rec.Get(...) then; CalcSomething(); end; }` resolves the `Rec.Get` builtin + the dataitem's table for field accesses. Assert via `assemble_workspace_units` + resolve that the `Rec` receiver in the dataitem trigger types as `Record` for table `Customer`.

- [ ] **Step 2:** In `l2/mod.rs` (the routine-feature projection), when a routine is a report DATAITEM trigger, seed a `Rec` record variable whose table is the dataitem's source table. Mirror the existing implicit-`Rec` seeding (table_name left None; `record_types` pass-3 backfills `table_id` from the dataitem's source table). You will likely need to thread the dataitem's source-table name to the routine-projection (analogous to how `source_table_name` is threaded for pages). Extend `record_types.rs` pass-3 `own_table_id` with a `Report`/dataitem case if the table must be resolved there.

- [ ] **Step 3:** Build, test, measure CDO+DC untracked-receiver (expect the `implicit-rec` sub-shape to drop). Full suite green (regen any fixture golden that gains a report-dataitem `Rec`).

- [ ] **Step 4:** Commit `feat(engine-d22): seed implicit Rec for report dataitem triggers`.

---

## Feature C — Single-hop compound receivers

Do NOT build a general linear type-checker (Gemini's caution). Resolve two concrete single-hop shapes only.

**Files:** `src/engine/l3/receiver_type.rs` (compound handling at the top of `infer_receiver_type`, near `compound_blob_media_field_kind` / `currpage_control_receiver`), `src/engine/l3/member_builtins.rs` (framework-property→type map), tests.

### Task C1: Framework-property → framework-type single hop

`HttpClient.DefaultRequestHeaders.Add()` → `DefaultRequestHeaders` is an `HttpHeaders`; `HttpResponseMessage.Content.ReadAs()` → `Content` is `HttpContent`; `HttpRequestMessage.Content` → `HttpContent`. These are PROPERTIES on framework types that RETURN another framework type.

- [ ] **Step 1:** Add a small static map `framework_property_type(kind, property_lc) -> Option<ReceiverBuiltinKind>` in `member_builtins.rs` seeded with the known framework property→type returns (HttpClient.defaultrequestheaders→HttpHeaders, HttpResponseMessage.content→HttpContent, HttpRequestMessage.content→HttpContent, HttpResponseMessage.headers→HttpHeaders, HttpRequestMessage.headers→HttpHeaders, …). Source from the AL docs / the `member_builtins.json` if it encodes property return types; otherwise the few Http* ones observed.

- [ ] **Step 2:** In `infer_receiver_type`, in the compound (`simple_receiver_name` declined) branch, BEFORE declining: split `<base>.<prop>` on the last `.`; infer the BASE receiver type (recursive `infer_receiver_type` on `<base>`); if it is `Framework{kind}` (or `Object`/`Record`) and `<prop>` maps via `framework_property_type`, return `Framework{prop_kind}`. Dispatch then resolves the method on the property's type. Decline (stay compound) otherwise.

- [ ] **Step 3:** Test `ResponseMsg.Content.ReadAs(...)` (ResponseMsg: HttpResponseMessage) classifies builtin. Build, measure compound-receiver, full suite green.

- [ ] **Step 4:** Commit `feat(engine-d22): single-hop framework-property compound receivers (HttpClient.DefaultRequestHeaders.Add etc.)`.

### Task C2: Call-result single hop (`Func().M()`)

`SomeFunc().Method()` where `SomeFunc` is a resolvable procedure with a known return type → type the receiver as that return type, dispatch `Method`.

- [ ] **Step 1:** Confirm the receiver expr for a call-result is shaped like `Func()` or `Obj.Func()` (the `compound-receiver::call-result` shape tag already exists — inspect how it is detected). Determine whether the caller's resolved call graph / routine signatures give `Func`'s return type at this point (return types are on `L3Routine`). If the return type is NOT available in `infer_receiver_type`'s context, SCOPE THIS DOWN: handle only `Func()` where `Func` is a bare own-object/global procedure whose return type resolves to an object/record/framework type; otherwise decline.

- [ ] **Step 2:** Implement the narrowest correct version: parse `Func()` (strip trailing `()`), resolve `Func` as a bare call would (own-object → … ), read its return type, classify that return type (`parse_object_type_ref` / `classify_receiver`), dispatch the outer method. Decline on any uncertainty (no false resolutions — a wrong return-type guess is a precision regression).

- [ ] **Step 3:** Test, measure, full suite green. Commit `feat(engine-d22): single-hop call-result compound receivers (Func().M())`.

NOTE C2 is the riskiest for precision; if return-type info isn't cleanly available, ship C1 + report C2 as deferred rather than guess.

---

## Final gate

- [ ] Measure CDO + DC deps-loaded after all features; record the new realUnknownRate + residual buckets. Update `CHANGELOG.md` + the `[[cross-app-namespace-recursion]]` memory.
- [ ] Full `cargo test` green; regen + inspect any Rust-owned golden that shifted; update matrix oracles if aggregates moved; run the separate r3a/r4f regen paths if a touched fixture is in them.
- [ ] Precision spot-check: sample 5–10 newly-`builtin`/`resolved` edges from the new paths and confirm each is a real method on the real type (no masking of true holes), per Gemini's precision caveat.

## Sequencing

A (biggest clean win, data ready) → C1 (clean, small) → B (structural) → C2 (precision-sensitive, may defer). Each task is independently committable + measured.
