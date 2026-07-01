// Case (l) cross-app `internal`: `IntCallerA` lives in a DIFFERENT app than
// `IntFoo`/`IntFooExtB`. `internal` is app-scoped — NOT visible outside its
// declaring app, even though the app is a reachable dependency.
//
// AL semantics: DOES NOT COMPILE (access error — `DoWork` is `internal` to
// the OTHER app and not visible here; this is the classic
// `InternalsVisibleTo` gap — friend-app visibility is OUT OF SCOPE for this
// task, documented as a false-`Unknown`/recall cost, not a soundness hole).
//
// Fresh-engine expected route: Evidence::Unknown (already correct
// PRE-Task-1 too — existing guard from beyond-1B.3b Task 2, unaffected by
// this task's restructure). See
// `resolve_member_record_cross_app_extension_internal_method_excluded` and
// `resolve_member_record_cross_app_base_table_internal_method_excluded`.
codeunit 52402 "IntCallerA"
{
    procedure Test()
    var
        R: Record IntFoo;
    begin
        R.DoWork();
    end;
}
