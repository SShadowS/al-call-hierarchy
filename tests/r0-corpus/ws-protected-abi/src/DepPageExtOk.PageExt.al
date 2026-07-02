// Task 1 fixture (c) POSITIVE: a GENUINE workspace `PageExtension` of the
// dep's "Dep Page" — AL lets an extension call its base's `protected`
// members, so the bare call to `P` (extension-base fallback, `resolve_bare`
// Step 2) must RESOLVE, carrying `Access::Protected` rather than dropping it.
pageextension 51001 "DepPageExtOk" extends "Dep Page"
{
    procedure CallProtected()
    begin
        P();
    end;
}
