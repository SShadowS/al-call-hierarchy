// beyond-1B.3b Task 1 review-fix regression fixture (Finding 1): declares the
// arity-1 overload of `Foo` that the base table does NOT have. `Rec.Foo(x)`
// (arity 1) must fall through the base table's wrong-arity `Foo()` (arity 0)
// and resolve to THIS extension's `Foo`, with `Evidence::Source` — never
// `Unresolved`/`Unknown`, and never a pick-first guess against the base
// table's non-matching overload.
tableextension 51001 "BaseTableExt" extends BaseTable
{
    procedure Foo(X: Integer)
    begin
    end;
}
