// beyond-1B.3b Task 1 review-fix regression fixture (Finding 1): `R.Foo(5)`
// (arity 1) must resolve to `BaseTableExt.Foo`, not the base table's
// wrong-arity `Foo()` (arity 0), and NOT Unresolved/Unknown.
codeunit 51002 "ArityFallthroughCaller"
{
    procedure CallFoo()
    var
        R: Record BaseTable;
    begin
        R.Foo(5);
    end;
}
