// Case (h) cross-app NON-extension: same reasoning as (g), but `CallerH`
// lives in a DIFFERENT app (a dependency relationship on the app that
// declares `Bar`, not an extension relationship). A fortiori excluded.
//
// AL semantics: DOES NOT COMPILE (access error).
//
// Fresh-engine expected route (POST-FIX): Evidence::Unknown. PRE-FIX (the
// bug this task closes — the ORIGINAL cross-app gap this task's brief
// names): Evidence::Source, wrongly targeting Bar.P (the pre-fix cross-app
// branch filtered only Local/Internal, leaving Protected completely
// unfiltered cross-app too) — see
// `resolve_member_record_cross_app_non_extension_protected_excluded` and
// COMPILER_PROOF.md row (h).
codeunit 52661 "CallerH"
{
    procedure Test()
    var
        R: Record Bar;
    begin
        R.P();
    end;
}
