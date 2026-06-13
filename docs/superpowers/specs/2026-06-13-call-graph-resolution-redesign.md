# Call-Graph Resolution Redesign — Typed Receiver Model

**Status:** Spec (pre-implementation). Author: engine team + external review (Gemini 3.1 Pro).
**Goal:** Make whole-program call-graph resolution precise — the engine's moat. Drive the real-`unknown` edge rate on real BC apps from ~47% to low single digits, where the residual is provably dynamic, WITHOUT triggering a false-positive explosion in the L5 detectors.

---

## 1. Why (measured diagnosis)

On a real 4842-routine BC app (CDO), `aldump --l3-call-graph`:
- **13971 call edges, 6541 unresolved (47%).**
- 6541 = 5934 `unknown` + 528 `external-target` + 58 `member-not-found` + 21 `ambiguous`.

The 5934 `unknown` are almost all **member calls** (`receiver.method()`), by receiver declared type:

| Receiver type | count | nature |
|---|---|---|
| Record | 2299 | ~1507 record **built-ins** (FieldNo/GetView/SetRecFilter/Mark/…) misclassified as unknown; ~792 genuine **table procedures** (resolvable, real FN) |
| Codeunit | 3852 | mostly resolve today; tail → external-target |
| `<unknown-var>` (receiver not in routine.variables) | 1124 | object globals, CurrPage/CurrReport, chained `a.b.M()` |
| RecordRef | 670 | dynamic / runtime table |
| Framework (JsonObject 142, TextBuilder 62, Dialog 48, List/Dict/Stream…) | few hundred | should be `builtin` |

**Root cause:** the member resolver is a string-keyed step ladder — `simple_receiver_name` → find in `routine.variables` → `parse_object_type_ref(declared_type)` (returns `None` for Record, RecordRef, all framework types) → object dispatch. Everything off that path returns `unknown`. There is **no model of what a receiver IS**, no type inference beyond a local/param's declared-type string, and built-in recognition exists **only** on the bare-call path (`call_resolver.rs:404`), never on the member path.

### Audit findings (this spec's investigation)
- **L3 never consults receiver scope.** `L3Routine` carries no scope frames. The L2 `scope_frames` are *control-flow* frames (branch/loop/try terminates), NOT receiver-type scopes. The WITH-receiver / implicit-Rec *type* context lives only in body_walk's transient `implicit_receiver_stack` and is **never persisted past L2**. So a bare/member call inside `WITH CustVar DO` or on the implicit trigger `Rec` cannot be resolved against that receiver.
- Bare path (`call_resolver.rs:390`) resolves against the routine's **own object only**, then builtin, then unknown.
- `parse_object_type_ref` (`type_ref.rs:67`) accepts only Codeunit/Page/Report/Query/XmlPort/Interface/Enum — Record/RecordRef/framework → `None` → unknown.
- Dependency symbols ARE fully ingested (CDO preflight `opaque_apps = []`). **This is not a missing-symbol problem — it's that the resolver never uses the loaded symbols for member/Record dispatch.**

---

## 2. Architecture — two-phase typed resolver

Replace the ladder with **receiver type inference → typed dispatch**.

### Phase A — `infer_receiver_type(expr, env) -> ReceiverType`
A type lattice:
```
ReceiverType =
  | Object { kind: ObjectKind, id: ObjectId }      // Codeunit/Page/Report/Query/XmlPort
  | Record { table_id: TableId }
  | RecordRef | FieldRef | KeyRef                    // (RecordRef may carry an inferred table_id, §5)
  | Interface { name }
  | Framework { kind: FrameworkKind }                // JsonObject/Token/Array/Value, Http*, Stream, List, Dictionary, TextBuilder, Dialog, Blob, Xml*, …
  | Enum { name }
  | Primitive
  | SystemSingleton { CurrPage | CurrReport | … }
  | Unknown
```
Resolves from an **environment** (`env`), not just a flat var list:
- variable declared-types (params/locals/**object globals**),
- the implicit `Rec`/`xRec` (→ effective own table — reuse the d22 `record_types` pass-3 effective-own-table logic),
- **WITH-receiver scope** (the receiver type of the enclosing `WITH` block — requires persisting the receiver-type scope, see §3),
- static refs (`Codeunit::"X"`, `Page::"Y"`),
- `CurrPage`/`CurrReport` → host page/report type,
- **recursively** for chained receivers (`a.b.M()` → infer type of `a.b`) and **method/function return types** (`GetX().M()` → return type of `GetX`).

### Phase B — `dispatch(receiver_type, method, arity, env) -> Resolution`
One rule per variant:
- **Object** → `resolve_by_name_and_arity` on that object's procedures (today's logic, subsumed unchanged).
- **Record** → (a) record-builtin catalog → `builtin`; (b) else table procedure via `routines_in_object(tableObj)` → `resolved`; (c) else `member-not-found`.
- **RecordRef/FieldRef/KeyRef** → builtin catalog → `builtin`; `GetTable`/`Open(tableId)` flows a table type (§5).
- **Framework** → builtin catalog → `builtin`.
- **Interface** → multi-edge to implementations (today's `resolve_interface_dispatch`).
- **Enum/Primitive** → `builtin` / n/a.
- **Unknown** → `unknown` (a TRUE failure — the FN signal).

---

## 3. Receiver-type scope (the WITH / implicit-Rec fix)

Persist a **receiver-type environment** from L2 → L3 (today only the transient `implicit_receiver_stack` exists, dropped at L2). Two options, decide at impl:
- (a) Persist per-callsite the enclosing receiver-type context (WITH receiver var name + implicit-Rec flag), so the resolver can resolve a bare/member call against it; OR
- (b) Reconstruct the WITH/implicit scope in L3 from the AST/feature data.

Prefer (a) — additive per-callsite metadata (mirrors how `loop_stack` / `in_until_condition` were threaded). The env stack the resolver consults: **Locals → Params → Globals → implicit `Rec` → ordered enclosing `WITH` receivers.**

---

## 4. The intrinsic built-in catalog

A data-driven `(ReceiverType-kind, method_lc) -> Disposition` table covering Record, RecordRef/FieldRef/KeyRef, and every framework type.

**Source (decided):** hand-built / MS-Learn-doc-scraped. AL record + framework built-ins are **compiler intrinsics** (baked into `Microsoft.Dynamics.Nav.CodeAnalysis.dll`) — they do NOT ship in any `.app` `SymbolReference.json` (those carry only AL-declared user objects). So the `.app` symbols resolve user/table/cross-app targets + arity; the intrinsic catalog is a separate, necessary knowledge asset. (External review initially proposed scraping `.app`; conceded this point.)

**Implementation:** `phf` perfect-hash for 0-cost lookup; kept separate from the `.app` symbol pool. Schema must handle overloads/arity-insensitive built-ins (most intrinsics are name-only dispositions; a few need arity, e.g. `Modify([RunTrigger])`). Disposition ∈ {builtin, dynamic(RecordRef runtime), flows-type(GetTable/Open)}.

---

## 5. TableID constant-propagation (dynamic → static)

The dynamic-dispatch moat. **Keep L3/L4 strictly layered — no L4→L3 feedback** (cyclic fixpoint = unmaintainable). Build a **cheap intra-procedural** constant tracker in L3 Phase A:
- `MyTableID := Database::Customer` → cache in the routine env;
- `RecordRef.Open(MyTableID)` / `.GetTable(rec)` → if the arg resolves to a static table → emit a static `Record{table_id}` flow, turning `dynamic` into `resolved`;
- cross-procedural / DB-derived table ids → fall to `dynamic` (L4's domain, not fed back).

Reuse the *spirit* of the existing L4 value-source classifier (literal/enum/constant-var/parameter/table-field) but implement the intra-procedural tracker natively in L3 to avoid the layering cycle.

---

## 6. Honest resolution taxonomy

Split today's overloaded `unknown` into:
- `builtin` — platform method, no AL target. **Not a hole.**
- `dynamic` — RecordRef/runtime/variant. **Genuinely indeterminate.**
- `external` — resolves into a dependency object.
- `unknown` — **TRUE** resolution failure = the FN signal to drive toward zero.

Only after this split is "perfect graph walking" measurable.

---

## 7. The false-positive explosion (central risk, NOT a footnote)

Today's clean CDO baseline (699 actionable, heavily triaged) is **partly an artifact** of a graph that can't see through 47% of calls. Driving `unknown`→0 grows the capability cone and WILL surface latent transitive FPs (e.g. `db-op-in-loop` now tracing a benign loop 6 codeunits deep to a `Record.Find`).

**Decision (do NOT make the cone path-sensitive):** over the SCC condensation, path count is unbounded → fixpoint/memory blowup. Instead:
- **Guard-predicate edge annotations.** At L2/L3, walk dominating control-flow guards of each call/op and tag the edge with a minimal high-impact guard set: `GuiAllowed`, `IsTemporary`, `HasFilter`, (extend as needed). Cheap, local.
- **Cone stays flow-insensitive** (fast set-union).
- **Suppression at L5:** detectors intersect guard tags along the reachability path; discard findings whose path requires a guard incompatible with the root context (e.g. requires `GuiAllowed` but root is a background session). ~80% of path-sensitivity's precision at a fraction of the cost.
- **Paired re-triage:** every resolver phase that lands new edges is followed by a CDO re-triage to catch the FP wave before it erodes trust.

---

## 8. Validation — the north-star

- **Metric:** CDO real-`unknown` edge rate (after the §6 taxonomy split). Measure after EVERY phase; progress is proven, not asserted.
- **Contract oracles** (extend `l3cg_oracles.rs` — assert the CONTRACT, not byte-parity, so "both engines wrong" can't pass):
  - every `resolved` edge's `to` exists in the symbol table;
  - every `builtin` is a real platform method (in the catalog);
  - every `unknown` has no inferable receiver type (no catalog hit, no resolvable target);
  - `dynamic` only where the receiver is genuinely runtime-typed.
- **Differential goldens** rebaselined (Rust-owned baselines; TS oracle retired) as the regression net.
- **Detector precision:** per-detector TP% on CDO must not regress as edges are added (the §7 guard work is the lever).

---

## 9. Build sequence (ground-truth before inference)

1. **Taxonomy split** (`builtin`/`dynamic`/`external`/`unknown`) — make the metric honest.
2. **Intrinsic catalog** (`phf`) for Record/RecordRef/FieldRef/KeyRef + framework types → the ~1500 misclassified built-ins become `builtin`.
3. **`ReceiverType` lattice + Phase A/B**; route the **existing object path** through it first (goldens stable — proves the refactor sound), then add **Record-receiver dispatch** (the ~792 real table-procedure edges).
4. **Receiver-type scope** (§3): WITH + implicit-Rec consultation; **globals + return-type** inference.
5. **Intra-procedural TableID/Enum constant-prop** (§5) → dynamic→static.
6. **Guard-predicate edge annotations + L5 guard-intersection suppression** (§7), paired with CDO re-triage.

Measure CDO real-`unknown` after each step. Phases 1–2 are pure accuracy (low risk). Phase 3 is the core refactor. Phases 4–6 are the precision/depth moat.

---

## 10. Open questions for impl
- Catalog schema: name-only vs (name, arity) keys; how to encode `flows-type` built-ins (GetTable/Open).
- Scope persistence (§3): per-callsite metadata vs L3 reconstruction.
- Guard-predicate set (§7): exact minimal list + how detectors consume it (edge tag intersection vs a per-finding guard-context check).
- Return-type inference depth (chained receivers): cap recursion (like the L4 value-source `MAX_CHASE_DEPTH=3`).
