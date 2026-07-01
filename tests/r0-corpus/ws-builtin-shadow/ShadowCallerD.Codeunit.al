// (d) Near-miss name (NOT a real catalog member, despite being textually
// adjacent to real builtins): must NOT be classified `builtin` — falls through
// to honest Unknown.  The catalog is an exact-string lookup (no
// fingerprint/hash digest — see `builtins.rs`/`member_catalog.rs` module
// docs), so a coincidental "fingerprint collision" cannot surface as a false
// `builtin`; this fixture locks that in at the call-site level.
codeunit 50954 "ShadowCallerD"
{
    procedure CallD()
    begin
        ZzNotARealBuiltinFp('x');
    end;
}
