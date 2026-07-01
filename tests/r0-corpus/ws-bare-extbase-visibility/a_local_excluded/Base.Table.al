// Case (a): `Base` declares a `local procedure L()`. AL `local` is
// OBJECT-scoped — visible ONLY to `Base` itself, never to any of its
// extensions (even a direct one). Mirrors the Rust unit test
// `bare_extension_base_local_method_excluded` (`src/program/resolve/
// resolver.rs`).
table 52900 "Base"
{
    local procedure L()
    begin
    end;
}
