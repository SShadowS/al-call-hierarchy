// PageExtension generalization of case (a): `BasePage` declares a
// `local procedure L()`. Mirrors the Rust unit test
// `bare_pageextension_base_local_method_excluded` — proves the Step-2 access
// filter is generalized across extension kinds, not hardcoded to
// TableExtension.
page 52908 "BasePage"
{
    local procedure L()
    begin
    end;
}
