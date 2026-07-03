// Task 2 review fix (Finding 1) fixture: two SOURCE overloads sharing
// name+arity, differing only by parameter TYPE — the discriminating
// position a bare-identifier argument's (possibly with-rebound) type would
// otherwise pick between.
codeunit 50960 "WS Overload Target"
{
    procedure Foo(X: Decimal)
    begin
        Message('decimal');
    end;

    procedure Foo(X: Text)
    begin
        Message('text');
    end;
}
