// Case (d) PEER-extension: `FooExtA` (see FooExtA.TableExt.al) declares
// `local procedure DoWork()`; sibling `FooExtB` — a DIFFERENT extension of
// the SAME base table — calls `R.DoWork()`.
//
// AL semantics: `local` is OBJECT-scoped. `FooExtB` is not `FooExtA`, so
// `DoWork` is not visible to it, even though both extend the same base and
// live in the same app. Expected: DOES NOT COMPILE (AL0136-class access
// error — `DoWork` is not part of `Record Foo`'s visible surface from
// `FooExtB`'s context).
//
// Fresh-engine expected route (POST-FIX): Evidence::Unknown. PRE-FIX: also
// buggy (false Source to FooExtA.DoWork) — see
// `resolve_member_record_peer_extension_local_method_excluded`.
tableextension 52632 "FooExtB" extends Foo
{
    procedure Wrapper()
    var
        R: Record Foo;
    begin
        R.DoWork();
    end;
}
