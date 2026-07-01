// Case (g) same-app NON-extension: `CallerG` is a plain Codeunit, same app
// as `Bar`, but NOT an extension of `Bar`.
//
// AL semantics: `protected` is visible to the declaring object AND its
// extensions ONLY. A Codeunit that merely holds a `Record Bar` variable is
// neither. Expected: DOES NOT COMPILE (access error — `P` is not part of
// `Record Bar`'s visible surface here).
//
// Fresh-engine expected route (POST-FIX): Evidence::Unknown. PRE-FIX (the
// bug this task closes): Evidence::Source, wrongly targeting Bar.P — see
// `resolve_member_record_same_app_non_extension_protected_excluded` and
// COMPILER_PROOF.md row (g).
codeunit 52651 "CallerG"
{
    procedure Test()
    var
        R: Record Bar;
    begin
        R.P();
    end;
}
