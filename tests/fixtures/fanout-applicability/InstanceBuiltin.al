// 1B.3b Task 2: a Page-typed variable's Close() call has no declared
// Close procedure on ApplicPage, so resolve_member falls through to the
// PageInstance instance-builtin catalog (Evidence::Catalog, BuiltinId
// "PageInstance::close"). route_applicability's independent catalog
// re-check (instance_builtin_route_applicable) must pass it:
// instance_builtin_violations stays 0.
//
// NOT RunModal (T1.3, deep-review-remediation plan): `Run`/`RunModal` are now
// special-cased to dispatch UNCONDITIONALLY to the target's entry trigger
// (`OnOpenPage`/`OnPreReport`), bypassing the PageInstance/ReportInstance
// instance-builtin catalog entirely — they no longer produce a `Catalog`
// route at all, so they can no longer exercise this soundness check. `Close`
// has no entry-trigger special case, so it keeps this fixture's non-vacuity
// proof meaningful.
page 50702 "ApplicPage"
{
}

codeunit 50703 "ApplicPageCaller"
{
    procedure Go()
    var
        MyPage: Page "ApplicPage";
    begin
        MyPage.Close();
    end;
}
