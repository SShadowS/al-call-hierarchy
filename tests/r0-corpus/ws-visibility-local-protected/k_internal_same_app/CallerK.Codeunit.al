// Case (k) same-app `internal`: `CallerK` is a DIFFERENT object than `Foo`,
// but in the SAME app. `internal` (unlike `local`) IS app-scoped — visible
// to any code in the SAME app, regardless of self/extension status.
//
// AL semantics: legal — COMPILES. Fresh-engine expected route:
// Evidence::Source, target = Foo.P (unaffected by this task; `Internal`
// app-scoping was already correct pre-Task-1, and stays correct through the
// restructure).
codeunit 52702 "CallerK"
{
    procedure Test()
    var
        R: Record Foo;
    begin
        R.P();
    end;
}
