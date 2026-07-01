// Case (c): `Base` lives in the DEPENDENCY app (AppB) and declares an
// `internal procedure I()`. `internal` is app-scoped — visible only within
// the DECLARING app. Mirrors the Rust unit test
// `bare_extension_base_cross_app_internal_method_excluded`.
table 52904 "Base"
{
    internal procedure I()
    begin
    end;
}
