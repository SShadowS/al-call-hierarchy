# Compiler proof — `internalsVisibleTo` friend-app modeling (Task 1.5)

**Status: SPEC-STATED, NOT COMPILER-RUN**, same caveat as
`tests/r0-corpus/ws-object-interface-visibility/PROOF.md`: no AL compiler (`alc`/`ALC_EXE`) was
available in this task's execution environment. Every row below states the AL access-modifier
semantics as documented by Microsoft's AL language reference ("Access Modifiers" +
"InternalsVisibleTo") rather than an actual `alc` compile/diagnostic run. The
`cdo_l3_semantic_audit_no_fresh_wrong` gate (`genuine_wrong == 0`) is the REGRESSION BACKSTOP; this
document is the intended CORRECTNESS proof artifact.

## What this task closes

Task 1 made `resolve_in_object`'s (and `object_has_visible_member_candidate`'s) `Access::Internal`
rule strictly app-scoped: `obj_id.app == from_object.app`, else `Unknown`. Measuring the resulting
CDO `InternalNotVisible` bucket (60 sites total — the +51 Task 1 added plus 9 pre-existing) found
**100% of them** are calls from CDO ("Continia Document Output") to `internal` members of its
CTS-CDN dependency ("Continia Delivery Network"), whose `NavxManifest.xml` contains:

```xml
<InternalsVisibleTo>
  ...
  <Module Id="f4b69b55-c90d-4937-8f53-2742898fa948" Name="Continia Document Output" Publisher="Continia Software" />
  ...
</InternalsVisibleTo>
```

AL: an `internal` member is visible within its declaring app AND to any app the declaring app's
manifest lists in `<InternalsVisibleTo>` (a "friend" app). CDO is an explicitly-declared friend of
CTS-CDN — every one of those 60 calls is AL-LEGAL. The strict same-app-only rule was an
OVER-DECLINE, not a soundness win. This task models the friend list (already fully parsed —
`<InternalsVisibleTo>` sits right next to `<Dependencies>` in the same manifest) so the
`Access::Internal` rule becomes: same-app, OR the declaring app's manifest names the caller's app a
friend.

## AL access-modifier semantics (source: Microsoft Learn, "Access Modifiers" + "InternalsVisibleTo")

| Modifier | Visible from |
|---|---|
| `internal` | Any code within the SAME app, AND any app the declaring app's manifest lists in `<InternalsVisibleTo>` (a friend app). NOT visible from any other app. |

Friendship is declared **by the app exposing the internals**, not by the caller, and is
**one-directional**: app B naming app A a friend does not make app B's own internals visible to A
— A would need its own `<InternalsVisibleTo>` entry naming B for that.

## Case-by-case matrix

| Case | Directory | AL rule applied | Expected | Pre-fix (Task 1 only) fresh route | Post-fix (Task 1.5) fresh route | Rust test(s) |
|---|---|---|---|---|---|---|
| (a) cross-app `internal`, declaring app lists caller as friend | `a1_friend_authorized_cross_app_internal/` | `internal` + friend declaration → visible | Compiles | Unknown (`InternalNotVisible`) — **the over-decline this task fixes** | Source | `resolve_member_object_cross_app_internal_friend_authorized_resolves_to_source` |
| (b) CONTROL: cross-app `internal`, declaring app names NO friends (true stranger) | `b1_stranger_cross_app_internal_control/` | `internal` not visible outside declaring app absent a friend declaration | Access error | Unknown (`InternalNotVisible`) | Unknown (`InternalNotVisible`) — unchanged, proves the model doesn't over-grant | `resolve_member_object_cross_app_internal_stranger_control_stays_unknown` |
| (c) DIRECTIONALITY: A declares B a friend (B→A resolves); the reverse relationship is never inferred | `c1_directionality_reverse_stays_unknown/` | Friendship is declared BY the exposing app; B naming no friends of its own means B's internals stay app-scoped even though A trusts B | B→A compiles; a hypothetical reverse A-object→B internal call would still be an access error | B→A: Unknown pre-fix (Task 1 has no friend concept yet) | B→A: Source (A trusts B); B's own internals remain unaffected — no bidirectional shortcut exists in the implementation | `resolve_member_object_cross_app_internal_friendship_not_bidirectional` |
| (d) same-app `internal`, zero friends declared anywhere | `d1_same_app_internal_unaffected/` | Same-app visibility is unconditional, independent of any friend list | Compiles | Source (already correct, Task 1 unaffected same-app) | Source (unchanged) | `resolve_member_object_same_app_internal_unaffected_by_friend_modeling` |

## Why (a) and (c) are the exact pre-fix over-declines named in the task brief

Verified empirically during this task's TDD Step 2 by temporarily hardcoding
`internal_visible_across` to `exposing_app == caller_app` (simulating Task-1-only, pre-friend-model
behavior) and re-running the new tests:

- (a) `resolve_member_object_cross_app_internal_friend_authorized_resolves_to_source` — FAILED,
  `assert!(matches!(routes[0].target, RouteTarget::Routine(_)))` got `Unresolved` (i.e. the
  pre-fix code declined a call the friend declaration should have authorized).
- (c) `resolve_member_object_cross_app_internal_friendship_not_bidirectional` — FAILED on its first
  assertion (B→A), same `Unresolved` shape, for the same reason (this fixture's whole point is that
  B→A is friend-authorized).
- (b) the stranger CONTROL and (d) the same-app control both PASSED unchanged under the
  hardcoded-same-app-only simulation — confirming those two scenarios are genuinely unaffected by
  the fix (regression guards, not "was broken" tests).

The fix (`internal_visible_across` consulting `graph.friends`, populated in
`build_program_graph`'s Step 3b from each dependency `.app`'s parsed `<InternalsVisibleTo>`) closes
the over-decline; re-running the same tests against the fixed code, all 4 pass. See
`.superpowers/sdd/task-1.5-report.md` for the full transcript and the CDO delta.
