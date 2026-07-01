// Case (g) supplemental: the brief's literal example shape — a same-app
// PAGE whose `SourceTable` IS `Bar`, but which does NOT `extend` `Bar`. A
// page's `SourceTable` establishes its RECORD, not an extension
// relationship — visibility must gate on the CALLING object's own
// identity/kind, never the receiver's kind.
//
// AL semantics: DOES NOT COMPILE, same reasoning as CallerG.Codeunit.al.
//
// Fresh-engine expected route (POST-FIX): Evidence::Unknown. PRE-FIX: also
// buggy (false Source to Bar.P) — see
// `resolve_member_record_same_app_page_non_extension_protected_excluded`.
page 52653 "CallerGPage"
{
    SourceTable = Bar;

    procedure Test()
    var
        R: Record Bar;
    begin
        R.P();
    end;
}
