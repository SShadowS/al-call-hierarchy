// Case (b) same-app DIFFERENT object: `CallerA` is a DIFFERENT object than
// `FooExtB` (the declaring object of `DoWork`) even though both live in the
// SAME app.
//
// AL semantics: `local` is OBJECT-scoped, not app-scoped — it is visible
// ONLY within the declaring object itself. `CallerA` is a plain Codeunit,
// unrelated to `Foo`/`FooExtB` by extension. Expected: DOES NOT COMPILE
// (AL0136: "'DoWork' is not accessible ... due to its protection level" /
// the compiler reports `DoWork` as not a member visible on `Record Foo`
// from this context — `local` members of a TableExtension are not part of
// the base table's externally-visible surface).
//
// Fresh-engine expected route (POST-FIX): Evidence::Unknown (honest decline).
// PRE-FIX (the bug this task closes): Evidence::Source, wrongly targeting
// FooExtB.DoWork — see
// `resolve_member_record_same_app_extension_local_method_excluded` in
// `src/program/resolve/resolver.rs`, and COMPILER_PROOF.md row (b).
codeunit 52612 "CallerA"
{
    procedure Test()
    var
        R: Record Foo;
    begin
        R.DoWork();
    end;
}
