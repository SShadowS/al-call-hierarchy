# Compiler proof — `local`/`protected`/`internal` visibility matrix

**Status: SPEC-STATED, NOT COMPILER-RUN.** No AL compiler (`alc`/`ALC_EXE`) was available in this
task's execution environment (checked: no `alc`/`alc.exe` on `PATH`, `ALC_EXE` unset). Every row
below states the AL access-modifier semantics as documented by Microsoft's AL language reference
("Access Modifiers", `dev-itpro/developer/devenv-access-modifiers`, current as of AL/Business
Central runtime 11.0+, unchanged since access modifiers were introduced) rather than an actual
`alc` compile/diagnostic run. This is a documented gap, not a claim of compiler verification —
per the task brief, `genuine_wrong == 0` (the CDO L3 semantic-audit gate) is the REGRESSION
BACKSTOP; this document is the intended CORRECTNESS proof artifact, honestly marked as
spec-derived pending an actual compiler run.

**Optional follow-up (not done here):** an env-gated (`ALC_EXE`) verifier script that shells out to
a real `alc.exe` against each case directory and diffs the compiler's pass/fail against the
"Expected" column below would upgrade every row from SPEC-STATED to COMPILER-VERIFIED. Left as a
roadmap item — nothing here depends on it; the `genuine_wrong == 0` CDO gate remains the executable
regression backstop in the meantime.

## AL access-modifier semantics (source: Microsoft Learn, "Access Modifiers")

| Modifier | Visible from |
|---|---|
| (none) / `public` | Anywhere the object itself is visible (any app that can see the object). |
| `protected` | The declaring object itself, AND any object that `extends` it (TableExtension/PageExtension/ReportExtension/EnumExtension → their base object). NOT visible to unrelated objects, even in the same app. |
| `internal` | Any code within the SAME extension (app/package). NOT visible from a different app (absent the separate, out-of-scope `InternalsVisibleTo` mechanism). |
| `local` | The DECLARING OBJECT ONLY. Not visible to any other object, even in the same app, even to that object's own extensions. |

## Case-by-case matrix

Each row: fixture directory (under this directory) → AL semantics → Expected (compiles /
access-error) → fresh-engine route (pre-fix / post-fix) → backing Rust test(s) in
`src/program/resolve/resolver.rs`.

| Case | Directory | AL rule applied | Expected | Pre-fix fresh route | Post-fix fresh route | Rust test(s) |
|---|---|---|---|---|---|---|
| (a) `local` SELF | `a_local_self/` | `local` visible to declaring object | Compiles | Source (already correct) | Source | `resolve_member_record_local_self_call_resolves_to_source` |
| (b) `local` same-app, DIFFERENT object | `b_local_same_app_cross_object/` | `local` is OBJECT-scoped, not app-scoped | Access error | **Source → `FooExtB.DoWork` (WRONG — the bug)** | Unknown | `resolve_member_record_same_app_extension_local_method_excluded` |
| (c) `local` TableExtension SELF | `c_local_tableext_self/` | `local` visible to declaring object (the extension itself) | Compiles | Source (already correct) | Source | `resolve_member_record_tableext_local_self_call_resolves_to_source` |
| (d) `local` PEER extension | `d_local_peer_extension/` | `local` is OBJECT-scoped; sibling extension is a different object | Access error | **Source → `FooExtA.DoWork` (WRONG — the bug)** | Unknown | `resolve_member_record_peer_extension_local_method_excluded` |
| (e) `local` cross-app | `e_local_cross_app/` | `local` a fortiori excluded outside the declaring app | Access error | Unknown (already correct — beyond-1B.3b Task 2) | Unknown | `resolve_member_record_cross_app_extension_local_method_excluded` (pre-existing) |
| (f) `protected` SELF | `f_protected_self/` | `protected` visible to declaring object | Compiles | Source (already correct) | Source | `resolve_member_record_protected_self_call_resolves_to_source` |
| (g) `protected` same-app, NON-extension | `g_protected_same_app_non_extension/` | `protected` requires self OR extends-relationship; an unrelated same-app object has neither | Access error | **Source → `Bar.P` (WRONG — the bug)** | Unknown | `resolve_member_record_same_app_non_extension_protected_excluded`, `resolve_member_record_same_app_page_non_extension_protected_excluded` |
| (h) `protected` cross-app, NON-extension | `h_protected_cross_app_non_extension/` | Same as (g), a fortiori excluded cross-app | Access error | **Source → `Bar.P` (WRONG — the bug; pre-fix code filtered only Local/Internal cross-app, Protected was UNFILTERED)** | Unknown | `resolve_member_record_cross_app_non_extension_protected_excluded` |
| (i) `protected` valid extension → base (TableExtension) | `i_protected_valid_extension/` | `protected` visible to a DIRECT extension of the declaring object | Compiles | Source (already correct — the pre-fix bug only ADDED false positives, never removed this true positive) | Source | `resolve_member_record_tableext_protected_base_resolves_to_source` |
| (i) `protected` valid extension → base (PageExtension, generalization) | `i_protected_valid_extension/` | Same rule, generalized to a non-Table extension kind | Compiles (identity relationship only — see the file's scope note: the Page member-call PATH is a separate, out-of-scope code path) | N/A (untested path pre-fix) | `object_extends(BasePageExtI, BasePage) == true` | `object_extends_generalizes_to_pageextension_base_page` |
| (j) `protected` PEER-extension bleed | `j_protected_peer_bleed/` | `protected` requires self OR extends-the-SAME-base; `BarExtB` extends `Bar`, not `BarExtA` | Access error | **Source → `BarExtA.P` (WRONG — the biggest bug this task closes)** | Unknown | `resolve_member_record_peer_extension_protected_bleed_excluded` |
| (k) `internal` same-app | `k_internal_same_app/` | `internal` is app-scoped; same app is always visible | Compiles | Source (already correct) | Source | `resolve_member_record_same_app_internal_method_resolves_to_source` |
| (l) `internal` cross-app | `l_internal_cross_app/` | `internal` not visible outside the declaring app (`InternalsVisibleTo` friend-app exception is OUT OF SCOPE) | Access error | Unknown (already correct — beyond-1B.3b Task 2) | Unknown | `resolve_member_record_cross_app_extension_internal_method_excluded`, `resolve_member_record_cross_app_base_table_internal_method_excluded` (pre-existing) |

## Additional direct `ResolveIndex::object_extends` unit tests (identity-contract proofs, not tied
to one lettered case)

These assert the THREE independent guards `object_extends` combines (kind-compatible, direct-not-
transitive, identity-resolved-not-name-only) without going through `resolve_member` end-to-end:

- `object_extends_is_kind_compatible_not_name_only` — a `Table` and a `Page` sharing the literal
  name `"Shared"`; a `TableExtension "extends Shared"` must resolve against the `Table`, and
  `object_extends` against the `Page` identity must be `false` despite the name match.
- `object_extends_never_reverse` — a base object must never be considered to extend its own
  extension (the relationship is directional, not symmetric).
- `object_extends_never_peer` — two sibling extensions of the same base must never be considered to
  extend each other.

## Why (b)/(g)/(h)/(j) are the exact pre-fix wrong routes named in the task brief

Verified empirically during this task's TDD Step 2 by temporarily reverting
`object_has_visible_member_candidate` to its pre-fix body (same-app branch returns `true`
unconditionally; cross-app branch filters only `Local`/`Internal`) and re-running the new tests.
All 6 affected tests failed with `assert_eq!` showing the EXACT wrong `RouteTarget::Routine(..)`
predicted by the brief:

- (b) `resolve_member_record_same_app_extension_local_method_excluded` → wrongly resolved to
  `TableExtension` id `52611` (`FooExtB`) `DoWork`.
- (d) `resolve_member_record_peer_extension_local_method_excluded` → wrongly resolved to
  `TableExtension` id `52631` (`FooExtA`) `DoWork` (a same-app peer-`local` bug the brief did not
  separately name but which the SAME same-app blanket-`true` bug also caused).
- (g) `resolve_member_record_same_app_non_extension_protected_excluded` /
  `resolve_member_record_same_app_page_non_extension_protected_excluded` → wrongly resolved to
  `Table` id `52650`/`52652` (`Bar`) `P`.
- (h) `resolve_member_record_cross_app_non_extension_protected_excluded` → wrongly resolved to
  `Table` id `52660` (`Bar`) `P`.
- (j) `resolve_member_record_peer_extension_protected_bleed_excluded` → wrongly resolved to
  `TableExtension` id `52699` (`BarExtA`) `P`.

The fix (Step 3) restructures `object_has_visible_member_candidate` to be caller-identity-aware;
re-running the same 6 tests against the fixed code, all pass (honest `Evidence::Unknown`). See
`.superpowers/sdd/task-1-report.md` for the full before/after test run transcript.
