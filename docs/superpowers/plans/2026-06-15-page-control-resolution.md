# Page-Control Resolution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve `CurrPage.<Part>.Page.<method>()`, `CurrPage.<Part>.<method>()`, and `CurrPage.<UserControl>.<method>()` member-of-member calls — currently `Unknown{CompoundReceiver}` — by modeling a page's control tree (parts → subpage Page objects, usercontrols → control add-ins).

**Architecture:** Add a `page_controls` list to `L3Object`, populated from BOTH the native AL source (tree-sitter `part_section`/`usercontrol_section`) and dependency `.app` symbols (`Controls[]` with integer `Kind`: 6=Part with `RelatedPagePartId.Id`, 10=UserControl with `RelatedControlAddIn`). At resolution time, a `CurrPage.<ctrl>…` receiver is parsed in `infer_receiver_type`: look the control up on the CALLER's page (PageExtension merges base-page controls), resolve a Part to its subpage Page object and dispatch the method there; resolve a UserControl to its control-add-in object.

**Tech Stack:** Rust, tree-sitter-al V2 grammar, the existing L2→L3 assembly + cross-app projection, `phf` builtin catalogs.

**North-star check:** measure with `aldump --l3-unknown-breakdown-cross-app "U:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud"`. Baseline at plan time: compound-receiver 170 (≈65 `CurrPage.<Part>.Page`, ≈30 `CurrPage.<UserControl>`), realUnknownRate 2.3359%.

---

## File Structure

- `src/engine/l3/l3_workspace.rs` — `L3Object` gains `page_controls: Vec<L3PageControl>`; `L3PageControl` struct; native extraction `extract_page_controls(decl, source)`.
- `src/engine/deps/symbol_reference.rs` — `AbiObject` gains `page_controls`; extract from `Controls[]` (recursive; Kind 6 / 10).
- `src/engine/deps/projection.rs` — `ProjectedObject` gains `page_controls`; forward in the object loop.
- `src/engine/deps/cross_app_l3.rs` — `dep_object_to_l3` forwards `page_controls`.
- `src/engine/l3/symbol_table.rs` — accessor `page_controls_for(object_id)` returning base-page-merged controls for a Page/PageExtension caller.
- `src/engine/l3/receiver_type.rs` — `currpage_control_receiver(...)` parses `CurrPage.<ctrl>[.Page]`, resolves Part→subpage Page method / UserControl→add-in; wired at the top of `infer_receiver_type`.
- `src/engine/l3/member_builtins.rs` — `PagePart` kind + catalog (part-instance intrinsics: `update`, `activate`, `setselectionfilter`, …) for `CurrPage.<Part>.<m>` without `.Page`.
- Tests: `tests/l3cg_page_part_dispatch.rs` (new).

`L3Object` is additive-only (NOT `Serialize`-derived into any gate golden — see the existing `object_subtype` doc comment in `l3_workspace.rs`), so adding `page_controls` touches no R0–R3 golden directly. Resolution changes DO move call-graph edges → regen the affected Rust-owned goldens (Task 8).

---

### Task 1: `L3PageControl` data model + `L3Object` field

**Files:**
- Modify: `src/engine/l3/l3_workspace.rs` (struct `L3Object` ~line 38; add struct near it)

- [ ] **Step 1: Add the struct + field**

```rust
/// A page layout control relevant to member resolution: a `part`/`systempart`
/// (subpage) or a `usercontrol` (control add-in). `name` is the control name used in
/// `CurrPage.<name>`. `target` is the subpage Page reference (NAME from native source,
/// NUMBER string from dep symbols) for `Part`/`SystemPart`, or the control-add-in name
/// for `UserControl`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct L3PageControl {
    pub name: String,
    pub kind: PageControlKind,
    pub target: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageControlKind {
    Part,
    SystemPart,
    UserControl,
}
```

Add to `L3Object` (after `extends_target_name`):
```rust
    /// Page / PageExtension layout controls (`part`/`systempart`/`usercontrol`) — used
    /// to resolve `CurrPage.<control>…` member calls. Empty for non-page objects.
    pub page_controls: Vec<L3PageControl>,
```

- [ ] **Step 2: Fix every `L3Object { … }` initializer.** Run `cargo build --lib 2>&1 | rg "missing field .page_controls|E0063"`. For each, add `page_controls: Vec::new(),` (native path Task 2 + dep path Task 4 set real values; all others — tests, poison fixtures — use empty).

- [ ] **Step 3: Build green**

Run: `cargo build --lib 2>&1 | rg -i "error|Finished"`
Expected: `Finished`, no E0063.

- [ ] **Step 4: Commit** `git add src/engine/l3/l3_workspace.rs && git commit -m "feat(engine-d22): L3PageControl data model + L3Object.page_controls field"`

---

### Task 2: Native page-control extraction

**Files:**
- Modify: `src/engine/l3/l3_workspace.rs` (object-metadata block ~line 986; add `extract_page_controls` helper near `extract_extends_target_name`)
- Test: `tests/l3cg_page_part_dispatch.rs`

Grammar (confirmed): `part_section`/`systempart_section`/`usercontrol_section` each have fields `name` and `source` (`grammar.js` ~1526–1562). They live under the page's `layout_section`.

- [ ] **Step 1: Write the failing test** (`tests/l3cg_page_part_dispatch.rs`)

```rust
//! Page-control extraction + CurrPage.<Part> resolution (Rust-owned).
use al_call_hierarchy::engine::l3::l3_workspace::{
    assemble_workspace_units, resolve, PageControlKind,
};

fn page_with_part() -> &'static str {
    r#"page 50100 "My List Part" { SourceTable = "Item"; layout { area(Content) { } } }
page 50101 "My Card" {
    SourceTable = "Item";
    layout { area(Content) { part(Lines; "My List Part") { } } }
    procedure Foo() begin CurrPage.Lines.Page.Bar(); end;
}
page 50102 "X" { procedure Bar() begin end; }
"#
}

#[test]
fn native_part_control_extracted() {
    let ws = assemble_workspace_units(
        &[("u".to_string(), page_with_part().to_string())],
        "app",
        "mi",
    );
    let card = ws.objects.iter().find(|o| o.name == "My Card").unwrap();
    let lines = card.page_controls.iter().find(|c| c.name == "Lines").unwrap();
    assert_eq!(lines.kind, PageControlKind::Part);
    assert_eq!(lines.target, "My List Part");
}
```

- [ ] **Step 2: Run — expect FAIL** (`page_controls` empty)

Run: `cargo test --test l3cg_page_part_dispatch native_part_control_extracted`
Expected: FAIL (no control named "Lines").

- [ ] **Step 3: Implement `extract_page_controls`**

```rust
/// Walk a Page / PageExtension declaration's layout for `part`/`systempart`/
/// `usercontrol` sections, returning their (name, kind, target). `target` is the
/// `source` field verbatim (unquoted): a subpage Page NAME for parts, a control-add-in
/// NAME for usercontrols. Recurses the layout tree (areas/groups nest controls).
fn extract_page_controls(decl: Node, source: &str) -> Vec<L3PageControl> {
    fn walk(n: Node, source: &str, out: &mut Vec<L3PageControl>) {
        let kind = match n.kind() {
            "part_section" => Some(PageControlKind::Part),
            "systempart_section" => Some(PageControlKind::SystemPart),
            "usercontrol_section" => Some(PageControlKind::UserControl),
            _ => None,
        };
        if let Some(kind) = kind {
            if let (Some(name), Some(src)) =
                (n.child_by_field_name("name"), n.child_by_field_name("source"))
            {
                out.push(L3PageControl {
                    name: strip_quotes(node_text(name, source)).to_string(),
                    kind,
                    target: strip_quotes(node_text(src, source)).to_string(),
                });
            }
        }
        // Don't descend into routine bodies.
        if n.kind() == "code_block" {
            return;
        }
        for c in named_children(n) {
            walk(c, source, out);
        }
    }
    let mut out = Vec::new();
    walk(decl, source, &mut out);
    out
}
```

Wire into the object-metadata block (~line 986, after `extends_target_name`):
```rust
        let page_controls = if object_type == "Page" || object_type == "PageExtension" {
            extract_page_controls(decl, source)
        } else {
            Vec::new()
        };
```
Add `page_controls,` to the native `L3Object { … }` initializer (~line 1060).

- [ ] **Step 4: Run — expect PASS**

Run: `cargo test --test l3cg_page_part_dispatch native_part_control_extracted`
Expected: PASS.

- [ ] **Step 5: Commit** `git add src/engine/l3/l3_workspace.rs tests/l3cg_page_part_dispatch.rs && git commit -m "feat(engine-d22): extract page controls from native AL source"`

---

### Task 3: Dependency page-control extraction

**Files:**
- Modify: `src/engine/deps/symbol_reference.rs` (`AbiObject` ~line 90; page branch in `parse_symbol_reference` ~line 671)

Confirmed dep shape: page `Controls[]` (recursive via nested `Controls`), integer `Kind`: **6** = Part (`RelatedPagePartId: {Name, Id}` — `Id` is the subpage page NUMBER; `Name` usually empty), **10** = UserControl (`RelatedControlAddIn` = add-in name). `Name` is the control name.

- [ ] **Step 1: Add field to `AbiObject`**

```rust
    pub page_controls: Vec<(String, String, String)>, // (name, kind: "part"/"systempart"/"usercontrol", target)
```
(Reuse a tuple to avoid leaking the L3 enum into the deps layer; the projection maps it.)

- [ ] **Step 2: Add a `RawControl` deserialize struct + recursive extractor**

```rust
#[derive(Debug, Clone, Deserialize, Default)]
struct RawControl {
    #[serde(rename = "Kind")] kind: Option<i64>,
    #[serde(rename = "Name")] name: Option<String>,
    #[serde(rename = "RelatedPagePartId")] related_page_part_id: Option<RawRelatedId>,
    #[serde(rename = "RelatedControlAddIn")] related_control_addin: Option<String>,
    #[serde(rename = "Controls")] controls: Option<Vec<RawControl>>,
}
#[derive(Debug, Clone, Deserialize, Default)]
struct RawRelatedId { #[serde(rename = "Id")] id: Option<i64> }

fn collect_page_controls(controls: &[RawControl], out: &mut Vec<(String, String, String)>) {
    for c in controls {
        let name = c.name.clone().unwrap_or_default();
        match c.kind {
            Some(6) => {
                if let Some(id) = c.related_page_part_id.as_ref().and_then(|r| r.id) {
                    out.push((name, "part".into(), id.to_string()));
                }
            }
            Some(10) => {
                if let Some(addin) = &c.related_control_addin {
                    out.push((name, "usercontrol".into(), addin.clone()));
                }
            }
            _ => {}
        }
        if let Some(sub) = &c.controls {
            collect_page_controls(sub, out);
        }
    }
}
```
Add `#[serde(rename = "Controls")] controls: Option<Vec<RawControl>>` to `RawObject`. In the `Pages` branch (~line 671), populate `abi_object.page_controls` via `collect_page_controls`.

- [ ] **Step 3: Test against a real dep page** — add to `tests/l3cg_page_part_dispatch.rs` a `#[test]` gated on the CDO `.alpackages` existing (skip if absent, like other cross-app tests) asserting at least one Part control is extracted from `Microsoft_Base Application*.app`. Pattern: mirror an existing cross-app test's path-guard.

- [ ] **Step 4: Build + run** `cargo test --test l3cg_page_part_dispatch`

- [ ] **Step 5: Commit** `git add src/engine/deps/symbol_reference.rs tests/l3cg_page_part_dispatch.rs && git commit -m "feat(engine-d22): extract page controls from dep .app symbols (Kind 6/10)"`

---

### Task 4: Projection threading

**Files:**
- Modify: `src/engine/deps/projection.rs` (`ProjectedObject` ~line 114; object loop ~line 343)
- Modify: `src/engine/deps/cross_app_l3.rs` (`dep_object_to_l3` ~line 90)

- [ ] **Step 1:** Add `pub page_controls: Vec<(String, String, String)>` to `ProjectedObject`; in the object loop set `page_controls: o.page_controls.clone()`.

- [ ] **Step 2:** In `dep_object_to_l3`, map the tuples to `L3PageControl`:
```rust
        page_controls: o.page_controls.iter().map(|(n, k, t)| L3PageControl {
            name: n.clone(),
            kind: match k.as_str() {
                "systempart" => PageControlKind::SystemPart,
                "usercontrol" => PageControlKind::UserControl,
                _ => PageControlKind::Part,
            },
            target: t.clone(),
        }).collect(),
```

- [ ] **Step 3: Build green** `cargo build --lib 2>&1 | rg -i "error|Finished"`

- [ ] **Step 4: Commit** `git add src/engine/deps/projection.rs src/engine/deps/cross_app_l3.rs && git commit -m "feat(engine-d22): thread page_controls through dep projection"`

---

### Task 5: SymbolTable accessor (base-page-merged controls)

**Files:**
- Modify: `src/engine/l3/symbol_table.rs`

A PageExtension's `CurrPage` sees its OWN added controls PLUS the base page's controls. The caller object id → its controls; if it's a PageExtension, union the `extends_target` base page's controls.

- [ ] **Step 1: Accessor**

```rust
/// Page controls visible to `CurrPage` inside `object_id` — the object's own controls,
/// plus (for a PageExtension) the extended base page's controls.
pub fn page_controls_for(&self, object_id: &str) -> Vec<&L3PageControl> {
    let Some(obj) = self.object_by_id(object_id) else { return Vec::new(); };
    let mut out: Vec<&L3PageControl> = obj.page_controls.iter().collect();
    if obj.object_type.eq_ignore_ascii_case("pageextension") {
        if let Some(base) = obj.extends_target_name.as_deref()
            .and_then(|n| self.object_by_type_name("Page", n))
        {
            out.extend(base.page_controls.iter());
        }
    }
    out
}
```

- [ ] **Step 2: Build green; Commit** `git commit -m "feat(engine-d22): SymbolTable::page_controls_for (PageExtension base merge)"`

---

### Task 6: Resolve `CurrPage.<Part>[.Page].<method>`

**Files:**
- Modify: `src/engine/l3/receiver_type.rs` (top of `infer_receiver_type` ~line 162; new helper near `compound_blob_media_field_kind`)

`infer_receiver_type` receives the receiver expr (`"CurrPage.Lines.Page"` / `"CurrPage.Lines"`). Handle BEFORE `simple_receiver_name` (these are compound).

- [ ] **Step 1: Write the failing test** (extend `tests/l3cg_page_part_dispatch.rs`) — using `page_with_part()` from Task 2, assert the call graph resolves `CurrPage.Lines.Page.Bar()` to page 50102's `Bar` (`resolve` the workspace, find the edge from `Foo`, assert `to` is `Bar`'s id, `resolution == Resolved`). Mirror an existing `l3cg_*` test's resolve+assert shape.

- [ ] **Step 2: Run — expect FAIL** (currently CompoundReceiver unknown).

- [ ] **Step 3: Implement.** Add at the very top of `infer_receiver_type`:

```rust
    // CurrPage.<control>[.Page] — page-part / usercontrol member receiver.
    if let Some(inferred) = currpage_control_receiver(receiver_expr, routine, symbols) {
        return inferred;
    }
```

Helper:
```rust
/// Resolve a `CurrPage.<control>[.Page]` receiver against the caller page's controls.
/// A `Part`/`SystemPart` with a trailing `.Page` (or bare — page-level methods live on
/// the subpage object) yields an `Object{Page}` receiver typed to the subpage Page, so
/// Phase B dispatches the method as a normal page procedure. A `UserControl` yields the
/// control-add-in object receiver (Task 7). `None` when the prefix/control doesn't match.
fn currpage_control_receiver(
    receiver_expr: &str,
    routine: &L3Routine,
    symbols: &SymbolTable,
) -> Option<InferredReceiver> {
    let rest = receiver_expr
        .strip_prefix("CurrPage.")
        .or_else(|| receiver_expr.strip_prefix("currpage."))?;
    // `<ctrl>` or `<ctrl>.Page` (case-insensitive trailing `.Page`).
    let ctrl_seg = rest
        .strip_suffix(".Page")
        .or_else(|| rest.strip_suffix(".page"))
        .unwrap_or(rest);
    let ctrl_name = simple_receiver_name(ctrl_seg)?; // lowercased, unquoted
    let controls = symbols.page_controls_for(&routine.object_id);
    let ctrl = controls.into_iter().find(|c| c.name.to_lowercase() == ctrl_name)?;
    match ctrl.kind {
        PageControlKind::Part | PageControlKind::SystemPart => {
            // Resolve the subpage Page object by NAME (native) or NUMBER (dep).
            let page = if let Ok(num) = ctrl.target.trim().parse::<i64>() {
                symbols.object_by_type_number("Page", num)
            } else {
                symbols.object_by_type_name("Page", &ctrl.target)
            }?;
            Some(InferredReceiver {
                ty: ReceiverType::Object {
                    kind: ObjectKind::Page,
                    name: page.name.clone(),
                },
                declared_type: format!("Page {}", page.name),
                receiver_shape: None,
            })
        }
        PageControlKind::UserControl => currpage_usercontrol_receiver(&ctrl.target, symbols), // Task 7
    }
}
```

Note: `dispatch_object` for `ObjectKind::Page` must resolve a method by name+arity against that page's procedures. Verify the existing `dispatch_object` path does this for `Page` (it handles Object kinds); if a Page receiver currently only does object-run, add a procedure-lookup arm. CHECK during implementation and adjust.

- [ ] **Step 4: Run — expect PASS.** Then `cargo test` (full) — expect only call-graph golden shifts (handled in Task 8).

- [ ] **Step 5: Commit** `git commit -m "feat(engine-d22): resolve CurrPage.<Part>.Page member calls to subpage Page procedures"`

---

### Task 7: Resolve `CurrPage.<UserControl>.<method>`

**Files:**
- Modify: `src/engine/l3/receiver_type.rs`; `src/engine/l3/member_builtins.rs`

Control-add-in methods (`CurrPage.Body.SetContent(...)`) target the control-add-in's declared procedures/events. Two cases: (a) the add-in is an in-model `ControlAddIn` object with procedures → resolve there; (b) external/platform add-in (no AL procedures) → classify `builtin` (the method is a platform/JS intrinsic, not a real AL target — honest, not a failure).

- [ ] **Step 1:** Implement `currpage_usercontrol_receiver(addin_name, symbols)`:
  - If `symbols.object_by_type_name("ControlAddIn", addin_name)` exists AND has a matching procedure → return an `Object{ kind: ControlAddIn }` receiver (add `ControlAddIn` to `ObjectKind` / `dispatch_object` if absent — CHECK `type_ref.rs`).
  - Else return a `Framework{ ControlAddIn }` receiver (new `ReceiverBuiltinKind::ControlAddIn`) whose dispatch always yields `builtin` (platform add-in method). Add the kind + an always-hit disposition (or a permissive catalog) in `member_builtins.rs`.

  DECISION POINT for the implementer/human: whether external add-in methods should be `builtin` or `dynamic`. Default to `builtin` (a control-add-in method is a real platform call, not runtime-typed) unless review says otherwise.

- [ ] **Step 2:** Test `CurrPage.<UserControl>.M()` resolves to non-`CompoundReceiver` (builtin or resolved).

- [ ] **Step 3: Commit** `git commit -m "feat(engine-d22): resolve CurrPage.<UserControl> member calls to control add-in"`

---

### Task 8: Measure, regen goldens, full green

- [ ] **Step 1: Kill stale binaries + release build**

Run: `taskkill //F //IM alsem.exe //IM aldump.exe 2>/dev/null; cargo build --release --bin aldump`

- [ ] **Step 2: Measure**

Run: `./target/release/aldump --l3-unknown-breakdown-cross-app "U:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud"`
Expected: `compound-receiver` drops from 170 by ≈65 (parts) + ≈30 (usercontrols); realUnknownRate ≈2.34% → ≈1.6–1.7%. Record exact numbers.

- [ ] **Step 3: Full suite + regen**

Run: `cargo test 2>&1 | rg "FAILED|test result"` — expect call-graph (`l3cg`/`l3cov`) byte-exact goldens + r1a/r2a/r2b/r2d differentials to shift where a fixture has page parts.
Run: `REGEN_TEMP_GOLDENS=1 cargo test` then `git diff --stat tests/` — INSPECT every diff is the intended part/usercontrol resolution (a previously-`unknown` member edge becoming `resolved`/`builtin`), NOT a regression.
Update matrix-oracle manifests (`tests/r2b-goldens/manifest.json`, `tests/r2d-goldens/manifest.json`, etc.) to the new Rust values if their aggregates changed. Run the separate r3a1/r3a2/r4f regen paths if those legacy differentials touch a part-bearing fixture.

- [ ] **Step 4: Confirm green** `cargo test 2>&1 | rg "FAILED"` → empty.

- [ ] **Step 5: CHANGELOG + commit** Update `CHANGELOG.md` (Added: page-control model; Changed: CurrPage.<Part>/<UserControl> resolution + metrics). `git commit -m "feat(engine-d22): CurrPage page-control resolution — measure + regen goldens"`

---

## Self-Review Checklist

- **Name vs number:** native part `source` = subpage NAME; dep part `RelatedPagePartId.Id` = NUMBER. `currpage_control_receiver` handles both (parse-int branch). ✓
- **PageExtension base merge:** `page_controls_for` unions base-page controls so a CDO PageExtension calling `CurrPage.<baseControl>` resolves. ✓
- **Caller is the page:** `CurrPage` = the object the routine lives in (`routine.object_id`), not a declared variable. ✓
- **No golden leak from the data model:** `L3Object` is not `Serialize`-derived into goldens; only resolution edge changes regen (Task 8). ✓
- **Type consistency:** `PageControlKind` defined in `l3_workspace.rs`, mapped from the deps tuple in `cross_app_l3.rs`; deps layer never imports the L3 enum. ✓
- **Open verification (do during impl, don't assume):** (1) does `dispatch_object` resolve a `Page` receiver's method by name+arity, or only object-run? (2) is `ControlAddIn` an `ObjectKind` / handled in `dispatch_object`? Adjust Tasks 6/7 to whatever the code actually does.

## Execution Handoff

Plan saved to `docs/superpowers/plans/2026-06-15-page-control-resolution.md`. Two execution options:

1. **Subagent-Driven (recommended)** — fresh subagent per task, two-stage review between tasks.
2. **Inline Execution** — batch with checkpoints.

Tasks 1–5 are mechanical (data plumbing). Tasks 6–7 carry the resolution judgment + the two open verifications. Task 8 is the regen/measure gate.
