# L3 Event-Graph Oracle Qualification

*Phase 4b Task 0 pre-requisite — governs Tasks 4-5 validation decisions.*

---

## What L3 models (as of `src/engine/l3/event_graph.rs`)

L3 `build_event_graph` (lines 243–360) processes `routines` twice: publishers first, subscribers second.

### Publisher side
- Detects publishers via `routine.kind == "event-publisher"` (line 254).
- Recognises **`IntegrationEvent`** and **`BusinessEvent`** attributes only (`publisher_event_kind`, lines 115–123).
- Reads `Isolated` bool from attribute args index 2 (IntegrationEvent) or 1 (BusinessEvent) (`parse_isolated`, lines 129–153).
- `element_name` is set to `None` on real publisher symbols (line 207 — `element_name: None`).
- Produces one `EventSymbol` per publisher routine; parameters come from `routine.parameters`.

### Subscriber side
- Detects subscribers via `routine.kind == "event-subscriber"` (line 269).
- Reads `[EventSubscriber(ObjectType::X, X::"Y", 'EventName', 'ElementName', ...)]` via `parse_subscriber_attribute` (lines 165–182):
  - arg 0 → `target_object_type` (qualified, e.g. `Codeunit`)
  - arg 1 → `target_ref` (qualified, e.g. `MyCodeunit`)
  - arg 2 → `event_name` (string literal)
  - arg 3 → `element_name` (string literal, optional)
- Produces one `EventEdge` per subscriber (lines 349–357): resolution `"resolved"` / `"maybe"` / `"unknown"`.

---

## Gaps (what L3 does NOT model)

| Gap | Location / Note |
|-----|----------------|
| **ManualBinding unread** | `[EventSubscriber(…)]` arg 4 is the `SkipOnMissingLicense` / `ManualBinding` flag; `parse_subscriber_attribute` only reads args 0-3. The `conditions` field on new-model `Route` never gets populated from L3 output. |
| **SkipOnMissingLicense / SkipOnMissingPermission unread** | Same: args beyond index 3 are not parsed. |
| **`element` dropped from `EventEdge`** | The `element_name` parsed from arg 3 ends up on synthesized `EventSymbol` (lines 300, 337) but NOT on the `EventEdge` struct (which has no `element_name` field). Edge-level element tracking is absent. |
| **Only FIRST `[EventSubscriber]` per handler** | `parse_subscriber_attribute` calls `find_attribute` which returns the first match. A procedure carrying two `[EventSubscriber]` attributes (multi-subscription) only emits one edge. |
| **InternalEvent → "unknown"** | L3 `publisher_event_kind` returns `"unknown"` for anything that is neither `IntegrationEvent` nor `BusinessEvent` (line 122). `InternalEvent` publishers are not indexed and subscribers targeting them get resolution `"maybe"` or `"unknown"`. |
| **Publisher resolution NAME-ONLY, no arity** | `encode_event_id` keys on `(publisherObjectId, eventName_lc)` only (line 185–187). Overloaded event names (same name, different parameter lists) are de-duplicated by name only; arity/signature matching is via `signature_hash`, not a separate dispatch step. |

---

## Per-aspect validation decision

| Aspect | Validation approach |
|--------|-------------------|
| **Publisher IntegrationEvent / BusinessEvent detection** | Covered by **dual-run core differential** (existing goldens) — publishers are the indexed backbone of the event graph. |
| **Subscriber attribute parsing (args 0-3)** | Covered by **dual-run core differential** — subscriber resolution "resolved"/"maybe"/"unknown" is in the golden event graph projection. |
| **ManualBinding / SkipOnMissing* conditions** | NOT in L3 oracle; validate via **fixture tests** only (unit tests in `edge.rs` and future `event_graph_fresh.rs`). Do not gate against L3 for these. |
| **`element_name` on edges** | NOT in L3 `EventEdge`; validate via **fixture tests** when fresh event emission is implemented (Task 5). |
| **Multi-subscription (multiple `[EventSubscriber]` attrs)** | **Non-shipping gap** for now — L3 only emits the first. Fresh engine must handle all; validate via fixture with a multi-subscriber procedure. |
| **InternalEvent publishers** | NOT covered by L3 oracle. Validate via **fixture tests** in Task 5; accept "unknown" categorisation in the dual-run harness for these events. |
| **Publisher arity / overload matching** | No per-arity gating needed at Task 0 (name-level resolution only); deferred to the semantic-intelligence charter (Spec 1). |

---

## Decision: what to use L3 as oracle FOR in Tasks 4-5

L3 is a valid oracle for:
- Publisher detection (IntegrationEvent + BusinessEvent) — use dual-run core differential.
- Subscriber attribute parsing for args 0-3 (object type, target ref, event name, element name) — use dual-run core differential.
- Resolution quality ("resolved" vs "maybe" vs "unknown") for the named-event resolution path.

L3 is NOT a valid oracle for:
- ManualBinding / SkipOnMissing* condition population → fixture-only.
- Element-level edge tracking → fixture-only.
- Multi-subscription handlers → fixture-only.
- InternalEvent subscriber resolution → fixture-only.
