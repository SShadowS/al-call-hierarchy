// 1B.3b Task 2: a Page-typed variable's RunModal() call has no declared
// RunModal procedure on ApplicPage, so resolve_member falls through to the
// PageInstance instance-builtin catalog (Evidence::Catalog, BuiltinId
// "PageInstance::runmodal"). route_applicability's independent catalog
// re-check (instance_builtin_route_applicable) must pass it:
// instance_builtin_violations stays 0.
page 50702 "ApplicPage"
{
}

codeunit 50703 "ApplicPageCaller"
{
    procedure Go()
    var
        MyPage: Page "ApplicPage";
    begin
        MyPage.RunModal();
    end;
}
