codeunit 50300 D3MethodStmt
{
    // Parameterless method calls without parens are statement-position
    // member_expressions. They MUST NOT be captured as field accesses (they
    // would otherwise turn into spurious "field(s) [setrecfilter, next]"
    // hits in D3's findings on real workspaces).
    procedure ParenlessMethodCalls()
    var Customer: Record Customer;
    begin
        if Customer.FindSet() then
            repeat
                Customer.SetRecFilter;
            until Customer.Next() = 0;
    end;

    // Real field access in an expression position — must still flow through
    // as a fieldAccess so D3 can flag the missing SetLoadFields.
    procedure RealFieldAccess()
    var Customer: Record Customer;
        s: Text;
    begin
        Customer.Get('X');
        s := Customer."No.";
    end;

    // `if Customer.FindSet then` — the member_expression's parent is `if_statement`,
    // but it occupies the parent's `condition` field, so it's an expression position
    // (NOT a statement). Must be treated as a field-access here, not a record-op.
    // (The parens-form below IS a real FindSet record-op via call_expression.)
    procedure FindSetInIfCondition()
    var Customer: Record Customer;
    begin
        if Customer.FindSet then begin end;
        if Customer.FindSet() then begin end;
    end;

    // `for i := 1 to L.Count do …` — L.Count is in the `end` field of for_statement,
    // an expression position. The body field, `Customer.SetRecFilter`, is a statement.
    // Without per-field disambiguation this would emit a phantom in-loop Count
    // record-op on L (and L isn't even a record — it's an XmlNodeList).
    procedure ForLoopBound()
    var Customer: Record Customer;
        L: List of [Integer];
        i: Integer;
    begin
        for i := 1 to L.Count do Customer.SetRecFilter;
    end;
}

table 18 Customer
{
    fields {
        field(1; "No."; Code[20]) { }
        field(50; "Last Date Modified"; Date) { }
    }
}
