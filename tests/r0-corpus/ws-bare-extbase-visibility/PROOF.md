# Fixture matrix — `resolve_bare` Step 2 ("extension base") access filtering

**Status: SPEC-STATED, NOT COMPILER-RUN** — same caveat as
`tests/r0-corpus/ws-visibility-local-protected/COMPILER_PROOF.md`, whose access-modifier
semantics table (`local`/`internal`/`protected`/`public`, sourced from Microsoft's AL
"Access Modifiers" language reference) applies unchanged here; not re-derived. No AL compiler
(`alc`/`ALC_EXE`) was available in this task's execution environment. The `genuine_wrong == 0`
CDO L3 semantic-audit gate is the executable regression backstop.

## What's different here vs. `ws-visibility-local-protected/`

That fixture set covers `resolve_in_table_scope` (the `Rec.Method()` member-call path and the
bare-implicit-`Rec` fallback, both Task 1). **This set covers a genuinely separate code path:**
`resolve_bare`'s Step 2 ("extension base", `resolver.rs`), which resolves a BARE (unqualified)
call made *from inside* a `*Extension` object against that extension's BASE object — e.g. a bare
`L();` written directly in a `TableExtension`'s procedure body, resolving against the base
`Table`'s own members. Pre-Task-1.5, this path went through `resolve_in_object` with **zero**
access filtering — a strictly separate (and, pre-fix, strictly less sound) route than Task 1's
`resolve_in_table_scope`, even though both ultimately consult the same base object.

## Case-by-case matrix

| Case | Directory | AL rule applied | Expected | Pre-fix fresh route | Post-fix fresh route | Rust test |
|---|---|---|---|---|---|---|
| (a) `local` excluded | `a_local_excluded/` | `local` is OBJECT-scoped; not visible to any extension, even a direct one | Access error | **Source → `Base.L` (WRONG — the bug)** | Unknown | `bare_extension_base_local_method_excluded` |
| (b) `public` CONTROL | `b_public_control/` | Default visibility always visible | Compiles | Source (already correct) | Source | `bare_extension_base_public_method_control_resolves_to_source` |
| (c) cross-app `internal` excluded | `c_internal_cross_app/` | `internal` is app-scoped; not visible outside the declaring app | Access error | **Source → `Base.I` (WRONG — the bug; CDO-confirmed real pattern, see CHANGELOG)** | Unknown | `bare_extension_base_cross_app_internal_method_excluded` |
| (d) `protected` CONTROL | `d_protected_control/` | `protected` visible to the declaring object and its extensions; Step 2's caller is BY CONSTRUCTION a direct extension of the base being probed, so self-or-extends trivially holds | Compiles | Source (already correct — incidentally safe) | Source | `bare_extension_base_protected_method_control_resolves_to_source` |
| (e) PageExtension `local` excluded | `e_pageext_local_excluded/` | Same rule as (a), generalized to a non-Table extension kind | Access error | **Source → `BasePage.L` (WRONG — the bug)** | Unknown | `bare_pageextension_base_local_method_excluded` |
| (f) PageExtension `public` CONTROL | `f_pageext_public_control/` | Same rule as (b), generalized | Compiles | Source (already correct) | Source | `bare_pageextension_base_public_method_control_resolves_to_source` |

All 6 Rust tests live in `src/program/resolve/resolver.rs`'s `#[cfg(test)] mod tests`, directly
below `bare_extension_base_object_proc_is_resolved` (the pre-existing Step-2 `Public` positive
case this task's controls extend). TDD-verified: cases (a)/(c)/(e) were run against the pre-fix
code first and failed with the EXACT wrong route recorded above (`RouteTarget::Routine` pointing
at the base object's inaccessible member, `Evidence::Source`) before the Step-2 access filter was
implemented; controls (b)/(d)/(f) already passed pre-fix and stayed green post-fix.

## CDO real-world confirmation (case (c) pattern)

The CDO corpus (`CDO_WS`) independently exercises exactly the case-(c) pattern:
`Al/Extensions/eCandidates/CDOConnecteCandidates.PageExt.al` (PageExtension 6175296, app
"Continia Document Output") bare-calls `internal procedure`s `GetIsSingleConnect`/
`GeteCandidatesFiltered`/`GetIsVendor`, all declared on the base Page `"CTS-CDN Connect
eCandidates"` (id 6252183) in app "Continia Delivery Network" — a genuinely different dependency
app (confirmed via `app.json`'s `dependencies` GUIDs and by extracting that dependency's embedded
ShowMyCode source directly). 10 such call sites flipped from false `Source` to honest `Unknown`
after this fix — see `CHANGELOG.md` for the full accounting.
