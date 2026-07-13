# T3 Task 4 — Resolver-read audit: the definition-surface fingerprint field list

- **Date:** 2026-07-12
- **Status:** DONE — audit complete, rung 1's body-read question answered, ONE binding
  implementation constraint discovered (see §6.1) that Task 9 MUST honor. **Patched
  2026-07-12 (Task 7 review fix-wave):** §4's per-object list gained a `name` field
  Task 7's implementation found missing and this review confirmed — see §6.5.
- **Scope:** documentation only. No code changed while producing this audit; the
  2026-07-12 patch above amends this doc's own field list, also documentation-only.
- **Parent design:** `docs/superpowers/specs/2026-07-12-t3-lsp-migration-design.md`
  §4 (two-rung soundness ladder), §11.1 ("fingerprint completeness is THE
  correctness risk of rung 1").

## 1. Method

The two entry points named in the brief are:

- `resolve_call_site_obligation` — defined `src/program/resolve/full.rs:344-612`,
  called once per extracted call site at `full.rs:697-712` (inside
  `resolve_full_program_from_parts`'s Phase-1 loop, `full.rs:619-752`).
- `emit_event_flow_edges` — defined `src/program/resolve/resolver.rs:3005-3104`,
  called once for the whole graph at `full.rs:757` (Phase-2).

Both take exactly three "whole-program" handles: `graph: &ProgramGraph`,
`index: &ResolveIndex`, `body_map: &BodyMap<'_>`. Every other read the resolver
performs is reachable ONLY through one of these three, or through the small set of
parameters `full.rs` threads in directly from the file/routine/object currently
being iterated (`routine`, `obj`, `file`, `call_args`, `with_state` — see §5).

**Classification rule (load-bearing, stated explicitly because it is not obvious):**
a read is classified by **accessor type**, not by where its answer happens to live
for one particular call. `graph.*`/`index.*`/`body_map.*` accessors are uniformly
**SURFACE-side**, even when, for a specific obligation, the data they return
happens to originate in the caller's own file — because the SAME accessor, called
from a different obligation (or even the same one, for a Table/Page/Report that
has extensions), can and does return another file's data. §5.3 walks the concrete
case that makes this non-optional: `resolve_in_extendable_scope` merges a base
object's OWN routine/field scope with every sibling extension object's scope
(different files, different `ObjectDecl`s) even when the call site being resolved
lives inside the base object's file. Classifying by "whose file is this,
this time" would misclassify that merge as caller-side.

Given that rule, the two remaining questions are mechanical:

1. **What does each accessor expose?** Answered once per accessor (not once per
   call site) by reading the struct/method definitions
   (`node_extract.rs`, `node.rs`, `graph.rs`, `index.rs`, `body_map.rs`) — §2.
2. **Does any accessor, or any consumer of its result, ever reach a `RoutineDecl`
   belonging to a file other than the one currently being extracted, and if so,
   which fields of it does it read?** Answered by tracing every call site of
   `BodyMap::get`/`get_with_path` (the ONLY constructor of a borrowed
   `&RoutineDecl` outside the caller's own extraction-loop parameters) — §3.

**Verification method for §2:** `Grep '\b(graph|index)\.[a-zA-Z_]+'` over
`resolver.rs`, `receiver.rs`, `arg_dispatch.rs` (the three files containing
`#[cfg(test)]` boundaries at `resolver.rs:3109`, `receiver.rs:3150`,
`arg_dispatch.rs:1565` — everything past those lines is test fixture code and was
excluded), then every DISTINCT accessor name appearing before those boundaries was
traced to its definition. This produced a closed, finite set of ~15 distinct
accessor methods (§2.2) — not an open-ended one — which is what makes the
per-accessor (rather than per-call-site) table both tractable and complete.

**Verification method for §3:** `Grep 'body_map\.(get_with_path|get)\('` over the
whole `src/program/resolve/` tree found exactly **3 non-test call sites** in the
entire directory (plus a handful inside `#[cfg(test)]` modules and `body_map.rs`'s
own unit tests, excluded). `receiver.rs` and `arg_dispatch.rs` both thread
`body_map`/`bm` through as an opaque parameter and never call `.get`/`.get_with_path`
themselves (confirmed by a second grep for `\bbm\.` / `\bbody_map\.` in both files —
no hits) — every actual dereference happens in `resolver.rs`. A supplementary grep
for `ParsedUnit`/`unit.files`/`.file.objects` in `resolver.rs`/`receiver.rs`
confirmed there is no second, BodyMap-bypassing route to a foreign `RoutineDecl`:
every production-code hit is `use` of test-only helper types, or lives past the
`#[cfg(test)]` boundary.

## 2. The full read table

### 2.1 Caller-side parameters (never touch graph/index/body_map)

These are the parameters `resolve_call_site_obligation` receives directly from the
Phase-1 loop (`full.rs:676-712`), sourced from the SAME `pf: &ParsedFile` currently
being iterated — never looked up by identity, so they cannot silently resolve to
another file's data.

| Parameter | Source struct | Fields read (representative) | Evidence |
|---|---|---|---|
| `routine: &RoutineDecl` | the call site's OWN enclosing routine | `.params`, `.locals`, `.return_name`, `.return_type`, `.body` (walked for local var typing / with-state), `.origin`, `.enclosing_member` | `full.rs:676-703` (`obj.routines.iter().enumerate()` on `pf.file`); consumed by `receiver.rs` fns taking `routine: &RoutineDecl` at lines 653, 1322, 1554, 2083, 2175, 2293 (caller-scope bare-identifier / named-return binding lookups) and `arg_dispatch.rs` at 564, 593, 892, 976 (`type_call_args`'s own-scope arg typing) |
| `obj: &al_syntax::ir::ObjectDecl` | the call site's OWN enclosing object | `.globals` (record-typed globals set, `full.rs:665-674`), `.routines` (sibling routines, same file) | `full.rs:652-664, 676` |
| `file: &al_syntax::ir::AlFile` | the call site's OWN file | dereferences a `Member.receiver`/call-arg `ExprId` into its `Expr` (`resolver.rs:439-448` `infer_receiver_type(... receiver.map(|id| (file, id)) ...)`) | `full.rs:361, 682, 710`; consumer `resolver.rs:439-448` |
| `call_args: &[ExprId]` | the call site's OWN argument expression list | typed once via `arg_dispatch::type_call_args` | `full.rs:367, 383-396` |
| `with_state: WithState` | the call site's OWN enclosing `with` scope, computed by a body-walk of `routine`'s OWN block | consumed by `resolve_bare`'s Step-3 with-guard and `arg_dispatch`'s bare-arg with-guard | computed `extract.rs:971-976` (`WithCtx`/`routine_has_with_token(src, routine.origin.byte.clone())`); `WithState` enum at `extract.rs:194`; consumed `resolver.rs` (with-guard sites), `arg_dispatch.rs` (with-guard sites) |
| `primary_app_ref: AppRef` | the workspace's own primary app | identity comparison only | `full.rs:622, 640-645, 705` |

`extract_sites_for_routine` (`extract.rs:951-976`), which produces the `RawSiteV2`
list `resolve_call_site_obligation` is called once per, is ALSO 100% caller-side:
it reads `file.objects[obj_idx].routines[routine_idx]` (the identical routine
`resolve_call_site_obligation` is later called for), its `.body`, `.name`,
`.origin.byte`, and scans `src` (the file's own text) for `with` tokens — never a
`graph`/`index`/`body_map` parameter exists in its signature at all.

### 2.2 Surface-side accessors (graph / index / body_map)

Every accessor below was confirmed, by reading its definition, to read only
`graph.objects`/`graph.routines` (pre-extracted `ObjectNode`/`RoutineNode` arrays,
built ONCE at graph-build time from every workspace+dep file) or the `ResolveIndex`
built once from those same arrays — never a live `ParsedUnit`/`AlFile` walk, except
for the 3 `BodyMap` sites traced separately in §3.

| Accessor | Returns | Classification | Evidence (definition) | Representative call sites |
|---|---|---|---|---|
| `graph.objects` (linear scan / `graph.obj_index`) | `&ObjectNode` by `ObjectNodeId` | SURFACE | `graph.rs:37, 40` | `resolver.rs:1072-1081` (base+extension tier lookup), `1941, 2446, 3309`; `receiver.rs:690, 1201, 1966, 1979, 2380, 2524` |
| `graph.routines` (binary search, sorted by id) | `&RoutineNode` by `RoutineNodeId` | SURFACE | `graph.rs:39`; sort invariant documented `index.rs:134` | `resolver.rs:159-166` (`routine_is_collapse_marked`), `241-269` (`routine_is_source_aliased`), `853, 867` (`lookup_routine_access`); `receiver.rs:1904, 1968, 2144, 2153` |
| `graph.topology.closure(app)` | `HashSet<AppRef>` — the app's full dependency closure | SURFACE (workspace-level, not file-scoped) | `graph.rs:35`, `topology.rs` | `resolver.rs:1065, 1087` (`resolve_in_extendable_scope`'s cross-app visibility gate) |
| `graph.friends` | `HashMap<AppRef, BTreeSet<AppRef>>` — `InternalsVisibleTo` grants | SURFACE (manifest-level) | `graph.rs:41-51` | `resolver.rs:700` (`internal_visible_across`'s `graph.friends.get(&exposing_app)` — the actual read; `resolver.rs:689,694` are doc-comment prose in the same function's own doc, not code, and `resolver.rs:3252` is a test-module doc-comment past the `#[cfg(test)]` boundary — none of the three counted as call sites) |
| `graph.resolve_object(app, kind, name_lc)` | `Option<&ObjectNode>` | SURFACE | `graph.rs:86-…` | `resolver.rs:1633, 1936, 2249, 2447`; `receiver.rs:289, 1891, 4040` |
| `index.routines_in_object(obj, name_lc)` | `&[RoutineNodeId]` (overload candidates, identity only) | SURFACE | `index.rs:429` | `resolver.rs:361, 725, 739, 1154, 1169, 1848, 2045, 2062, 2671` (all pre-`#[cfg(test)]`, production/doc); `receiver.rs:891, 2367`; `arg_dispatch.rs` (via `resolve_member`/`resolve_bare` chain) |
| `index.object_extends(graph, ext, base)` | `bool` — is `ext` a direct kind-compatible extension of `base` | SURFACE | `index.rs:861` | `resolver.rs:655, 783, 806, 913` (`Access::Protected` visibility) — all pre-`#[cfg(test)]`; the ×8 hits in the 8153-8289 range and 13473-13474 are inside `resolver.rs`'s test module (past line 3109) and are excluded |
| `index.resolve_object_ref(graph, from_app, obj_ref)` | resolves a `SourceTable`/`TableNo`/page-control `ObjectRef` to an `ObjectNodeId` | SURFACE | `index.rs:525` | `receiver.rs:684, 1122, 1448, 2422, 2446, 2498, 2766, 2830`; `arg_dispatch.rs:345, 409, 726, 1284` |
| `index.table_extensions_of` / `page_extensions_of` / `report_extensions_of` | `&[ObjectNodeId]` — sibling extension objects of a base Table/Page/Report | SURFACE (explicitly cross-FILE by design — see §5.3) | `index.rs:594, 608, 623` | threaded as the `extensions_of: fn` parameter of `resolve_in_extendable_scope`, `resolver.rs:1053-1105`; concretely instantiated at `resolver.rs:2055` (tables) and the Page/Report call sites of the same function |
| `index.table_scope_has_routine` | `bool` — does a routine of this name exist anywhere in the table's extension scope | SURFACE | `index.rs:786` | `receiver.rs:928, 1232, 1237, 1690`; `arg_dispatch.rs:741` |
| `index.field_in_table(graph, from, table, field_lc)` | `Option<FieldNode>` (`name_lc`, `type_text`) | SURFACE | `index.rs:678` | `receiver.rs:933, 1691`; `arg_dispatch.rs:744` |
| `index.implementers_of(iface_lc)` | `&[ObjectNodeId]` implementing an interface | SURFACE | `index.rs:633` | `resolver.rs:2256, 2625` |
| `index.subscribers_of(publisher)` | `&[SubscriberEntry]` (subscriber id + conditions + element filter) | SURFACE | `index.rs:899` | `resolver.rs:3023` (`emit_event_flow_edges`) |
| `body_map.get` / `get_with_path` | `&RoutineDecl` (+ virtual_path) | SURFACE, but field-restricted — see §3 | `body_map.rs:71, 79` | exactly 3 non-test sites, all in `resolver.rs` — traced fully in §3 |

`ObjectNode` (`node_extract.rs:110-160`) fields consulted through the above:
`id`, `name`, `declared_id`, `extends_target`, `implements`, `tier`, `source_table`,
`table_no`, `source_table_temporary`, `page_controls: Vec<PageControlNode>`
(`name_lc`, `kind: Part|SystemPart|UserControl`, `target`), `fields: Vec<FieldNode>`
(`name_lc`, `type_text`), `dataitems: Vec<DataitemNode>` (`name_lc`, `name`,
`source_table`), `parse_incomplete` (file-level parse-health degrade).

`RoutineNode` (`node_extract.rs:222-366`) fields consulted: `id`, `name`,
`access`, `tier`, `event_subscribers: Vec<ParsedSubscriberArgs>`
(`publisher_object_type`, `publisher_name`, `event_name`, `element`,
`skip_on_missing_license`, `skip_on_missing_permission` — `event.rs:19-31`),
`subscriber_instance_manual`, `publisher_kind`, `include_sender`, `return_type`,
`return_type_id` (ABI only), `abi_routine_kind`/`abi_event_kind` (ABI only),
`param_sig_key`, `abi_overload_collapsed`, `source_overload_aliased`, `abi_params`
(ABI only — `AbiParamRetained`: `type_text`, `is_var`, `subtype_id`,
`subtype_raw_name`, `subtype_tag`).

### 2.3 Static catalogs (neither caller- nor surface-side — compile-time constants)

`builtins.rs` (global builtin catalog, 785 names sourced from the AL compiler DLL's
generated documentation — `builtins.rs:1-40`) and `member_catalog.rs` (per-type
member surfaces: `Record`, `Page`, `Report`, `Enum`/`EnumTypeStatic`,
`SessionInformation`, `ControlAddIn`, etc. — MS-Learn sourced, e.g.
`member_catalog.rs:390-497`) are hardcoded Rust tables, unaffected by ANY workspace
file. They participate in resolution (a hit means `Evidence::Catalog`) but are not
part of the definition surface — no workspace edit can change them; only a manual
regeneration (`tools/gen-al-builtins`) or a source edit to `member_catalog.rs`
itself does, and both are engine-code changes, not data the fingerprint tracks.

**Verified, not assumed — "enum values" turned out NOT to be a real read.** The
design doc's §4 lists "enum values" as an expected fingerprint class. Tracing
`Enum::Value` / `Enum::"Type"` resolution (`receiver.rs:1399-1418`,
`member_catalog.rs:390-497`) shows resolution never validates an individual enum
VALUE name against a per-type value list at all — there is no `enum_values` field
anywhere in `ObjectNode` or any sibling structure. Only the ENUM TYPE's identity
(kind == `Enum`, resolved via the same `graph.resolve_object`/
`index.resolve_object_ref` path as any other object kind) feeds resolution, and the
member surface reachable on it (`AsInteger`/`Names`/`Ordinals` for a value instance
vs. `FromInteger`/`Names`/`Ordinals` for the type-static reference) comes from the
STATIC `member_catalog.rs` tables in §2.3, split by `FrameworkKind::Enum` vs.
`FrameworkKind::EnumTypeStatic` — never per-workspace data. **Conclusion: drop
"enum values" from the fingerprint's field list entirely** — the enum TYPE's own
identity (already covered as an ordinary `ObjectNode`) is the only real dependency.

## 3. Does resolution ever read another file's routine BODY? (the load-bearing question)

**Answer: NO — not in the sense Step 3 asks (inspecting body statements/expressions
to make a resolution decision). One related but DIFFERENT subtlety exists (a
byte-span, not a body-content, dependency) and is called out in §3.4/§6.1 as a
binding constraint on Task 9, not as a rung-1 blocker.**

Per §1's verification method, there are exactly 3 non-test consumers of
`BodyMap::get`/`get_with_path` in the entire `src/program/resolve/` tree.

### 3.1 `make_routine_route` — `resolver.rs:131-196`, BodyMap read at `resolver.rs:137`

```rust
fn make_routine_route(rid: &RoutineNodeId, obj_tier: TrustTier,
                       body_map: &BodyMap<'_>, graph: &ProgramGraph) -> Route {
    if let Some((decl, path)) = body_map.get_with_path(rid) {
        Route {
            target: RouteTarget::Routine(rid.clone()),
            evidence: tier_evidence(obj_tier),
            witness: Witness::SourceSpan {
                file: path.to_string(),
                span: (decl.origin.byte.start as u32, decl.origin.byte.end as u32),
            },
            ...
```

Fields read: **`decl.origin.byte.{start,end}`** and `path` only. This is the
CENTRAL routine-resolution route-builder — called for every dispatch-resolved
target (`resolve_in_object`'s single/ambiguous-overload arms at `resolver.rs:586`,
`resolve_object_run`, `resolve_implicit_trigger`'s trigger fan-out, `resolve_member`'s
`Codeunit.Run` special case). It is used to build the `Witness` attached to a
resolved edge — i.e. it decides nothing about DispatchShape/RouteTarget (those are
already decided by the caller before `make_routine_route` runs); it only stamps a
definition-location witness onto an already-resolved target. **Never reads
`decl.body`, `.locals`, `.return_name`, `.attributes`, `.access_modifier`,
`.enclosing_member`, `.dataitem_source_table`, or `.name`.**

### 3.2 `candidate_param_infos_either` → `candidate_param_infos` — `resolver.rs:622-632` → `arg_dispatch.rs:1139-1159`, BodyMap read at `resolver.rs:628`

```rust
fn candidate_param_infos_either(rid: &RoutineNodeId, ...) -> Option<Vec<ParamDispatchInfo>> {
    if let Some(decl) = body_map.get(rid) {
        return candidate_param_infos(decl, &rid.object, graph, index);
    }
    candidate_param_infos_abi(rid, graph, index)   // ABI-tier fallback, no BodyMap
}

pub(crate) fn candidate_param_infos(decl: &RoutineDecl, ...) -> Option<Vec<ParamDispatchInfo>> {
    if decl.parse_incomplete { return None; }
    for p in &decl.params { ... p.ty, p.by_ref ... }
}
```

Fields read: **`decl.parse_incomplete`** (a fail-closed trust gate — an
untrustworthy parse degrades the WHOLE arg-type-dispatch pick, never a partial
read) and **`decl.params[].ty` / `.by_ref`** (parameter TYPE TEXT and by-ref-ness —
pure signature data). This is `rid`'s role as a CANDIDATE TARGET being probed for
arg-type-dispatch overload disambiguation (the routine being considered as a match
for a call site's arguments) — i.e. reading the CALLEE's declared parameter types,
exactly the "routine signature ... param types incl. var-ness/subtypes" class the
design doc expects. **Never reads `decl.body`** — no statement, local variable, or
expression inside the routine is touched; `decl.params` is the parameter LIST
(types), a purely declarative signature artifact independent of what the routine's
body does.

### 3.3 `emit_event_flow_edges`'s `SiteId` construction — `resolver.rs:3005-3104`, BodyMap read at `resolver.rs:3061`

```rust
let site = if let Some((decl, path)) = body_map.get_with_path(&pub_routine.id) {
    SiteId { caller: pub_routine.id.clone(),
             span: CanonicalSpan { unit: path, start: decl.name_origin.start, end: decl.name_origin.end },
             callee_fingerprint: ... }
} else { /* synthetic zero-span site for a SymbolOnly/integration-gap publisher */ };
```

Fields read: **`decl.name_origin.{start,end}`** and `path` only — the publisher
ROUTINE'S OWN name-token span (used to anchor the `EventFlow` edge's own site,
analogous to a call site's span for an ordinary `Call` edge). Note `pub_routine`
here is the routine the edge is FROM (the publisher itself, i.e. this IS the
"caller" of an EventFlow edge, not a foreign target) — so this read is
structurally the EventFlow counterpart of §2.1's own-site span, not a foreign-file
signature read. **Never reads `decl.body`.**

### 3.4 The one real subtlety: `decl.origin` spans the WHOLE declaration, including its body

`crates/al-syntax/src/lower/mod.rs:868-885` constructs `RoutineDecl.origin` as
`origin_of(node)` where `node` is the ENTIRE routine CST node — header AND body.
This means `decl.origin.byte.end` (read at §3.1) **shifts whenever the routine's
body is edited**, even though NO body STATEMENT is ever inspected. This is a
byte-EXTENT dependency, not a body-CONTENT dependency — it cannot change
`DispatchShape`/`RouteTarget`/`SetCompleteness` for any edge (those are decided
before `make_routine_route` runs), but it DOES mean a `ClassifiedEdge` computed by
some OTHER file G that targets a routine in file F carries a `Witness::SourceSpan`
byte range that goes stale the instant F's body is edited — even under the
"unchanged fingerprint, only rebuild F's own edges" rung-1 path, since G's stored
edge is never touched.

**This does not block rung 1** — the resolution OUTCOME (which edge, which target,
which classification) is unaffected — but it DOES mean rung 1 is only sound for
the LSP's actual features if no handler ever serves position/range data straight
from a stored `Route::Witness`. The design doc's own handler mapping (§5:
"`incomingCalls` | ... caller item via BodyMap decl+path") already independently
arrived at re-deriving the caller item's range LIVE from the CURRENT `BodyMap` at
query time rather than trusting a stored edge field — this audit's finding is that
the SAME live-re-derive rule is a **hard requirement**, not merely the chosen
approach, and it must apply symmetrically to every position-bearing surface a
handler emits (`outgoingCalls`' target range, `prepareCallHierarchy`'s selection
range, any dep-source span) — see §6.1 for the binding constraint this places on
Task 9.

## 4. Derived `DefSurface` field list (Task 7 implements this verbatim)

A file F's definition-surface fingerprint is the hash of the following, computed
from F's OWN freshly re-parsed `ParsedFile` only (no graph/index/body_map lookups
needed to COMPUTE the fingerprint itself — only to CONSUME it, by comparing old vs.
new). Order matters for a stable hash; grouped by the object/routine each
sub-fingerprint belongs to, then hashed as one ordered sequence:

1. **The SET of `ObjectNodeId`s F declares** (kind + declared_id-or-name-key per
   object in `pf.file.objects`) — an add/remove/rename is a surface change by
   itself, independent of any per-field comparison below.
2. Per object (in a stable, deterministic order — e.g. sorted by `ObjectNodeId`):
   1. `name`, lowercased (**added during Task 7 implementation, confirmed by
      review — see §6.5**; item 1's identity key alone does NOT cover this
      for a NUMBERED object)
   2. `declared_id`
   3. `extends_target` (raw, as extracted — see note below on normalization)
   4. `implements` (as extracted; see §4 note on ordering)
   5. `source_table` + `source_table_temporary` (already conflict-degraded per
      §5.4 — hash the RESULT, no separate "had a preproc conflict" bit needed)
   6. `table_no` (same degrade-then-hash rule)
   7. `page_controls` (`name_lc`, `kind`, `target`, in document order)
   8. `fields` (`name_lc`, `type_text`, in document order)
   9. `dataitems` (`name_lc`, `name`, `source_table`, in document order)
   10. `parse_incomplete` (file-level parse-health flag)
3. **The SET of `RoutineNodeId`s F declares per object** (name_lc +
   enclosing_member_lc + params_count + sig_fp — an add/remove/rename/re-arity/
   overload change is a surface change by itself, same rationale as #1).
4. Per routine (stable order):
   1. `access` (Public/Local/Internal/Protected)
   2. `event_subscribers` (`publisher_object_type`, `publisher_name`, `event_name`,
      `element`, `skip_on_missing_license`, `skip_on_missing_permission`, per
      attribute, in source order)
   3. `subscriber_instance_manual`
   4. `publisher_kind`
   5. `include_sender`
   6. `return_type` (source-shaped text)
   7. `param_sig_key`
   8. Per-parameter (from `RoutineDecl.params`, SOURCE tier only — this is the
      ONE place the fingerprint reads through to `RoutineDecl` rather than
      `RoutineNode`, mirroring §3.2's live accessor): `ty`, `by_ref`, in
      declaration order
   9. `decl.parse_incomplete` (routine-level parse-health flag, gates whether #8
      is trustworthy — mirrors §3.2's fail-closed gate exactly)

**Explicitly EXCLUDED, with reasons:**

- **`decl.origin` / `decl.name_origin` (any span/position data).** Per §3.4, these
  are body-extent-dependent, never resolution-outcome-dependent, and per §6.1 must
  never be trusted stale by a handler anyway — so they carry no fingerprint value
  and MUST NOT be included (including them would make the fingerprint change on
  EVERY body edit, defeating rung 1's entire purpose).
- **`tier` (TrustTier).** Fixed by which app root the file lives under; a
  body-only re-parse of the same file can never change it. Structurally invariant
  under rung 1 by construction — no need to track.
- **ABI-only `RoutineNode` fields** (`abi_routine_kind`, `abi_event_kind`,
  `return_type_id`, `abi_params`) — always `None`/`Missing` for a source routine
  (see each field's doc, `node_extract.rs:252-365`); a workspace file's own
  fingerprint never touches them.
- **`abi_overload_collapsed` / `source_overload_aliased`.** These are DERIVED at
  `build.rs`'s cross-file dedup step (comparing F's routine against POSSIBLE
  duplicates in sibling files, e.g. `.dependencies/` shadow copies), not a pure
  per-file extraction result — but they are keyed off `param_sig_key`/identity
  equality, which is itself derived from `params` (already in the fingerprint, item
  4.8) — so no INDEPENDENT information would be gained by hashing these flags too;
  a body-only edit can never change `param_sig_key`, hence can never flip either
  flag. Excluded as redundant, not unsafe.
- **"Enum values."** Per §2.3, this class does not exist as a real read — dropped.
- **`is_trigger`.** NOT derivable from `RoutineNodeId` — that struct
  (`node.rs:133-147`) carries only `object`, `name_lc`, `enclosing_member_lc`,
  `params_count`, `sig_fp`; no kind/trigger discriminant exists there at all, so
  the original "redundant with the identity key" rationale was wrong and is
  corrected here. The real, verified reason to exclude it: `RoutineNode::
  is_trigger` has **zero reads anywhere in `resolver.rs`/`receiver.rs`/
  `arg_dispatch.rs`** (confirmed by grep) — the only production reader in the
  whole repo is `graphify_export.rs:823`, an unrelated CLI export path with no
  connection to call-site resolution. A field no resolution-path code ever
  consults cannot affect a resolution outcome, so it carries no fingerprint
  value. (See §6.2 — this correction was itself found by independent review.)

## 5. Explicitly out-of-scope (caller-side) — recap with the "why," including the
   file-boundary vs. object-boundary trap the brief called out

- **The call site's own routine** (`routine: &RoutineDecl`, full fields including
  `.body`) — read directly from `pf.file.objects[obj_idx].routines[routine_idx]`
  in the SAME iteration that also runs `resolve_call_site_obligation` for it
  (`full.rs:676-712`). Never looked up by identity, so it is provably the SAME
  file every time — this is the ONE read in the whole pipeline that is safe to
  classify by content rather than accessor type.
- **The call site's own object's globals** (`obj.globals`) — same rationale,
  `obj` comes from the SAME `pf.file.objects` iteration (`full.rs:652-676`).
- **The call site's own file's raw text/expression arena** (`file`, `call_args`)
  — same rationale.
- **`with_state`** — computed by a body-walk of the call site's OWN enclosing
  routine (`extract.rs:951-976`), never another file's.

**The trap the brief warned about, confirmed real:** a Page/Table/Report's OWN
`ObjectNode` (found via `graph.resolve_object`/`graph.objects`, "the caller's own
object") is NOT equivalent to "the caller's own file's data" once
`resolve_in_extendable_scope` (`resolver.rs:1053-1105`) runs. Resolving a BARE call
inside a base Page's OWN file can dispatch to a routine declared in a
PageExtension — a SEPARATE `ObjectDecl`, in a SEPARATE file, discovered via
`index.page_extensions_of(base_name_lc)` (`index.rs:608`) and merged into the same
candidate scope as the base object (`resolver.rs:1083-1101`). If site resolution in
file F (the base Page) can, for a DIFFERENT call site, actually dispatch into file
G (a PageExtension) — the accessor path (`graph.objects`/`index.*`) that answers
"what is my own object's scope" is uniformly SURFACE-side, full stop, regardless of
which specific instance happens to stay within one file. This is exactly why §1's
classification rule is "by accessor type," not "by data provenance for this one
call" — the latter would have missed this case, since a base Page with ZERO
extensions today could grow one tomorrow with no change to the base Page's own
file at all (a rung-2-triggering event on the EXTENSION side, but the base Page's
resolution logic ran through the identical extendable-scope code path all along).

## 6. Residual risks

### 6.1 BINDING constraint on Task 9 (from §3.4) — not a rung-1 blocker, but non-negotiable

No LSP handler may ever serve a position/range value read from a stored
`Route::Witness::SourceSpan` (or any other baked-in span inside a `ClassifiedEdge`)
for a TARGET routine — only for identifying/navigating purposes should a handler
re-derive the current span LIVE from the CURRENT snapshot's `BodyMap`/`decl_index`,
keyed by `RoutineNodeId`, at query time. The design doc's §5 handler-mapping table
already does this for `incomingCalls`' caller item; Task 9 must apply the identical
rule to `outgoingCalls`' target range, `prepareCallHierarchy`'s selection range for
ANY routine (not just the one navigated to), and any dep-source span. If this rule
is violated anywhere, rung 1 will silently serve stale byte ranges for any routine
whose declaring file was edited after the edge referencing it was last resolved —
this is invisible to the incremental-vs-batch differential gate UNLESS that gate's
scripted edit sequences specifically include "edit F's routine body, then query an
outgoingCalls/incomingCalls response that targets/originates in F from an
unrelated caller file G" — recommend adding that scenario explicitly to the gate's
script (flagged for whoever writes Task 8/the gate, not actioned here).

### 6.2 `is_trigger` exclusion — corrected during review (fix-wave, see below)

The original version of this audit excluded `is_trigger` on the wrong grounds
("derivable from `RoutineNodeId`/`RoutineKind`" — `RoutineNodeId` has no such
field, so that claim was false). Independent review caught this; §4 now states
the corrected, verified reason (zero reads in any resolution-path file). This
entry is kept as a record that the exclusion's JUSTIFICATION changed, not its
OUTCOME — `is_trigger` is still correctly excluded from the fingerprint, now for
a reason actually checked by grep rather than asserted from a false premise.

### 6.3 Non-source (dependency/ABI) files are out of this audit's scope

This audit traced the WORKSPACE-caller path (Phase-1 iterates
`ws_file_set`-filtered files only, `full.rs:648`). Rung 1 as specced
(§4 of the design doc) is a body-only-edit fast path for a SAVED WORKSPACE file;
whether a body-only edit to a dependency's EMBEDDED source (when a dep ships
source, not just SymbolReference ABI) needs the same treatment is a separate
question the design doc does not currently claim to answer, and this audit did
not investigate dependency-source edit flows (out of scope: deps are not
individually re-saved by a workspace developer in the modeled flow).

### 6.4 Ordering/normalization of list-valued fields (`implements`, `page_controls`, `fields`, `dataitems`, `event_subscribers`)

All of these are extracted in DOCUMENT ORDER (each field's own doc comment states
this). A preproc union-read (`#if`/`#else`) can make `implements` contain the SAME
interface name twice from two branches — this is fine for a `might-implement`
existence check (the consumer, per `node_extract.rs:453-460`, never treats it as a
singular pick) but means two textually-different-but-semantically-identical parses
(e.g. branch reordering with no logic change) COULD in principle hash differently
even though nothing resolution-relevant changed. This is a false-positive-only risk
(an unnecessary rung-1→rung-2 escalation, never an unsound skip) — acceptable
under the fail-closed doctrine ("any doubt fails toward rung 2"), noted so Task 7
does not need to chase determinism harder than the fail-closed direction requires.

### 6.5 Object `name` — added during Task 7 implementation, confirmed by review

**Original gap:** §4's per-object list (as first drafted) never mentioned
`ObjectNode::name` — item 2's fields ran `declared_id`, `extends_target`,
`implements`, `source_table`+`temporary`, `table_no`, `page_controls`, `fields`,
`dataitems`, `parse_incomplete`, and item 1's identity key (`kind` +
`declared_id`-or-`name`-key) only carries the display NAME for an ID-LESS object
(where the key is `ObjKey::Name(name_lc)`) — for a NUMBERED object the key is
`ObjKey::Id(n)`, which never changes on a rename.

**Why this was a real false-negative, not a nit:** §2.2's own read-table already
listed `name` among the `ObjectNode` fields "consulted through the [audited]
accessors" — but that fact never made it into §4's derived list. Tracing the
actual consumer confirms the risk is real: `graph.rs:18-28`'s `ObjectIndex::build`
keys its `by_app_kind_name` index on `(obj.id.app, obj.id.kind,
obj.name.to_ascii_lowercase())` for **every** object, numbered or not; `graph.rs:
86-127`'s `resolve_object` looks up purely through that `name_lc`-keyed index (the
own-app-shadow fast path and the dependency-closure fallback both key on
`name_lc`) and never reads `declared_id` at all. So renaming a NUMBERED object
(`codeunit 50100 "A"` → `codeunit 50100 "B"`, id held constant) changes what
`Codeunit "B".Foo()` call sites elsewhere in the workspace resolve to — a
resolution-outcome change with NOTHING in §4's then-current field list moving to
catch it.

**Disposition:** found and fixed during Task 7's implementation (not proposed
speculatively — the implementer traced the discrepancy between §2.2 and §4 while
implementing item 1, then verified the `ObjectIndex`/`resolve_object` read path
before acting), applying this design's own "when in doubt, INCLUDE the field"
directive rather than leaving the gap for a later task to rediscover. A dedicated
regression test (`object_renamed_with_same_numeric_id_changes_fingerprint`,
`src/lsp/def_surface.rs`) pins the numbered-object case; a companion test
(`id_less_object_renamed_changes_fingerprint_via_obj_key_name_arm`, added in the
same review fix-wave that produced this section) pins the id-less case via
`ObjKey::Name`, confirming both branches of the identity key are exercised.
Independently reviewed and CONFIRMED (re-verified the same `graph.rs` citations
above) before this doc was patched — §4's per-object list now includes `name` as
item 2.1, ahead of `declared_id`.

### 6.6 LSP-surface display reads of `is_trigger`/raw `name` — dated correction, 2026-07-13 (t3 whole-branch review)

**This is NOT a reversal of §6.2's `is_trigger` correction or of item 2's
lowercased-`name` reasoning.** Both remain true on their own terms:
`RoutineNode::is_trigger` still has zero RESOLUTION-path reads, and item 2's
lowercased `name` is still what `graph.resolve_object`'s by-name index keys
on. What this audit's scope (§1's "does a call-graph resolution path ever
read this field") never considered, because it did not exist yet when this
audit was written, is a SECOND class of reader added by LATER T3 tasks: the
LSP SURFACE's own DISPLAY code, which reads the identical `ProgramGraph`/
`ObjectNode`/`RoutineNode` data this fingerprint is built from, for a
DIFFERENT purpose than resolution.

**The finding:** `src/lsp/handlers.rs`'s `symbol_kind_for` (added by Task 11,
after this audit) reads `RoutineNode::is_trigger` to classify a
`CallHierarchyItem`'s `SymbolKind` (FUNCTION vs. EVENT); `object_name_for`
(also Task 11) reads the graph's RAW-cased `ObjectNode::name` for a
`CallHierarchyItem`'s `detail` text. Since rung 1 (Task 9) never rebuilds
`graph` (it Arc-clones `cur.graph` unchanged — see `src/lsp/updater.rs`'s
module doc), a source edit invisible to THIS fingerprint but visible to one
of these display reads would leave a stale icon/detail string served across
every subsequent rung-1 save, until an unrelated rung-2/3 event happened to
rebuild the graph and refresh it. Two concrete edits demonstrate this: a bare
`procedure Foo()` <-> `trigger Foo()` kind flip (same name/arity — no OTHER
fingerprint field moves either) and a case-only object rename on a NUMBERED
object (e.g. `"Sales Helper"` -> `"SALES HELPER"` — item 1's `ObjKey::Id`
identity is unchanged, and item 2's lowercased `name` folds the case
difference away).

**Disposition:** both fields are now ALSO in the fingerprint — `is_trigger`
as item 23 (routine-level), the RAW-cased `name` as item 22 (object-level,
alongside item 2's existing lowercased form, not replacing it) — but for
DISPLAY-staleness prevention, not resolution-soundness, which is why they
are recorded as NEW items appended after the audit's original list rather
than edits to items 1/2/§6.2's own reasoning. Cost: an extra rung-2 rebuild
on either edit (both rare) — negligible against the correctness gap closed.
Regression tests: `is_trigger_flip_changes_fingerprint` and
`object_renamed_case_only_changes_fingerprint` (`src/lsp/def_surface.rs`),
both confirmed to FAIL against the pre-fix fingerprint before the fix
landed (TDD), per this project's own verification discipline.

## 7. Reviewer instruction (per the brief's Step 4 — refute-by-default)

Independently grep `\b(graph|index)\.[a-zA-Z_]+` and `body_map\.(get|get_with_path)\(`
over `src/program/resolve/{resolver,receiver,arg_dispatch,extract,builtins,
member_catalog}.rs`, restricted to each file's own pre-`#[cfg(test)]` boundary
(`resolver.rs:3109`, `receiver.rs:3150`, `arg_dispatch.rs:1565`, `extract.rs:981`,
`builtins.rs:82`, `member_catalog.rs:624` — all six files DO have a `#[cfg(test)]`
split; the last three simply have ZERO `graph.`/`index.` accessor hits on either
side of it regardless, which is why §2 does not cite them for accessors — that
is a fact about their content, not an exemption from the boundary rule). Diff the
resulting accessor-name list against §2.2's table
and the BodyMap call-site count against §3's "exactly 3." Any accessor or call site
found that is NOT in this audit is a finding against this document, not against the
code — treat it as reopening this task, not as a bug report.
