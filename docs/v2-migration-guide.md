# V2 Grammar Migration Guide — al-call-hierarchy

The tree-sitter-al grammar was rewritten (V1→V2). This document lists all code changes needed in this repo.

## Breaking Changes

### 1. `procedure name: (name)` → `name: (identifier)`

The intermediate `name` wrapper node is removed. The `name` field now directly holds an `identifier` or `quoted_identifier`.

**Files to update:**
- `src/language.rs:23` — `name: (name) @proc.name)` → `name: (identifier) @proc.name`
- `src/language.rs:111` — `name: (name) @proc.name) @subscriber` → `name: (identifier) @proc.name`
- `tree-sitter-al/queries/highlights.scm:127` — `name: (name) @function.definition` → `name: [(identifier) (quoted_identifier)] @function.definition`
- `tree-sitter-al/queries/tags.scm:98` — `name: (name) @name` → `name: [(identifier) (quoted_identifier)] @name`

**Rust code impact:** `parser.rs:541` calls `n.child_by_field_name("name")` — in V2 this returns `identifier` directly (no wrapper), so `.text()` works without unwrapping.

### 2. `trigger_declaration name: (trigger_name)` → `name: (identifier)`

The `trigger_name` wrapper node is removed.

**Files to update:**
- `src/language.rs:27` — `name: (trigger_name) @trigger.name` → `name: [(identifier) (quoted_identifier)] @trigger.name`
- `tree-sitter-al/queries/highlights.scm:130` — same pattern
- `tree-sitter-al/queries/tags.scm:102` — same pattern

### 3. `parameter parameter_name: (name ...)` → `name: (identifier)`

Field renamed from `parameter_name` to `name`. The `name` wrapper node is removed.

**Files to update:**
- `tree-sitter-al/queries/highlights.scm:145` — `parameter_name: (name) @variable.parameter` → `name: [(identifier) (quoted_identifier)] @variable.parameter`
- `tree-sitter-al/queries/tags.scm:110-115` — `parameter_name: (name (identifier) @name)` → `name: (identifier) @name`
- `tree-sitter-al/queries/locals.scm:49-51` — `parameter_name: (name (identifier) @local.definition)` → `name: (identifier) @local.definition`

### 4. `member_expression property:` → `member:`

Field renamed from `property` to `member`.

**Files to update:**
- `src/language.rs:93` — `property: (_) @call.method` → `member: (_) @call.method`
- `tree-sitter-al/queries/highlights.scm:159` — `property: (identifier) @function.method.call` → `member: (identifier) @function.method.call`
- `tree-sitter-al/queries/highlights.scm:164-165` — `property: (identifier) @property` → `member: (identifier) @property`
- `tree-sitter-al/queries/tags.scm:191` — `property: (identifier) @name` → `member: (identifier) @name`

### 5. Individual property nodes → single `property` node

All 291 individual property nodes (`caption_property`, `editable_property`, etc.) are replaced by one `property` node with `name: (property_name)` and `value: (expression)`.

**Files to update:**
- `src/handlers.rs:556` — `if kind.ends_with("_property")` → `if kind == "property"`
- `src/handlers.rs:558` — `property_display_name(kind)` → read the `name` child field's text instead
- `src/handlers.rs:574-605` — `property_display_name()` and `extract_property_value()` — rewrite to:
  1. Match `node.kind() == "property"`
  2. Read `node.child_by_field_name("name")` for the property name (it's a `property_name` scanner token)
  3. Read `node.child_by_field_name("value")` for the value

### 6. `EVENT_SUBSCRIBERS` query — attribute_item is now sibling

In V2, `attribute_item` nodes are siblings of `procedure`, not children. The query must use the adjacent sibling operator (`.`).

**File to update:**
- `src/language.rs:103-112` — Restructure from nested to sibling pattern:

```scheme
; V1 (BROKEN — attribute nested inside procedure):
(procedure
  (attribute_item
    (attribute_content
      name: (identifier) @attr.name
      arguments: (attribute_arguments) @attr.args))
  name: (name) @proc.name) @subscriber

; V2 (attribute as sibling):
(attribute_item
  attribute: (attribute_content
    name: (identifier) @attr.name
    (#eq? @attr.name "EventSubscriber")
    arguments: (attribute_arguments) @attr.args))
.
(procedure
  name: (identifier) @proc.name) @subscriber
```

## Verify / Investigate

### 7. `field_access` node

V2 may have merged `field_access` (for `Rec."Field Name"`) into `member_expression` with a `quoted_identifier` as the member. Check if `field_access` still exists in V2's `node-types.json`. If not:

- `src/language.rs:97-99` — `field_access` query patterns need to become `member_expression` patterns
- `tree-sitter-al/queries/highlights.scm:167-168` — same
- `tree-sitter-al/queries/tags.scm:193-195` — same

### 8. `named_trigger` / `onrun_trigger` node types

These may have been unified into `trigger_declaration` in V2. Check V2's `node-types.json`.

If removed:
- `src/parser.rs:550` — `"named_trigger" | "onrun_trigger"` → `"trigger_declaration"`
- `src/parser.rs:589` — `extract_trigger_name` function
- `src/main.rs:286, 331` — kind checks
- `src/language.rs:30, 33` — query patterns
- `tree-sitter-al/queries/locals.scm:32-33` — scope definitions

### 9. `bad_delete.scm` query

Already uses V2 sibling structure (`.`) but still uses V1 field names:
- Line 16: `name: (name) @proc_name` → `name: (identifier) @proc_name`
- Line 19: `parameter_name: (name) @rec_param` → `name: (identifier) @rec_param`
- Line 21: `parameter_name: (name)` → `name: (identifier)`

## Low Risk (likely unchanged)

- `variable_declaration` `name` field — likely same in V2
- `call_expression` / `argument_list` — unchanged
- `code_block` — unchanged
- All `*_declaration` object node type names — unchanged
- `qualified_enum_value`, `database_reference` — unchanged
- `attribute_arguments` — unchanged
