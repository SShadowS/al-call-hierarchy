// Case (e) cross-app `local`: `LocCallerA` lives in the PRIMARY app (AppA,
// depends on AppB). `local` is even MORE restrictive than same-app
// cross-object (b) — not visible outside the declaring OBJECT at all, so a
// fortiori not visible outside the declaring APP.
//
// AL semantics: DOES NOT COMPILE (access error — `DoWork` is not part of
// `Record LocFoo`'s visible surface at all from another app).
//
// Fresh-engine expected route: Evidence::Unknown (already correct
// PRE-Task-1 too — existing guard, unaffected by this task's restructure).
codeunit 52412 "LocCallerA"
{
    procedure Test()
    var
        R: Record LocFoo;
    begin
        R.DoWork();
    end;
}
