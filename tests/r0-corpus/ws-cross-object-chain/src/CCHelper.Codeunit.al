// plan v2.1 Task 3 fixture — SOURCE-tier prefix procedures for
// `Var.Method().X()` cross-object call-result chains.
codeunit 51201 "CC Helper"
{
    // (a) POSITIVE prefix: `GetCustomer(No)` (unique arity-1, `Record "CC
    // Customer"` return) types the chain receiver `Record{table:
    // Some(CCCustomer)}`.
    procedure GetCustomer(No: Code[20]): Record "CC Customer"
    var
        Cust: Record "CC Customer";
    begin
        exit(Cust);
    end;

    // (d) leaf target for the single-implementer-interface positive chain
    // (`IFoo.GetHelper().DoWork()`).
    procedure DoWork()
    begin
    end;

    // (N4a) NEGATIVE prefix: scalar (`Integer`) return — nothing to
    // dispatch a member call on.
    procedure GetCount(): Integer
    begin
        exit(1);
    end;

    // (N4b) NEGATIVE prefix: no declared return type at all.
    procedure DoNothing()
    begin
    end;
}
