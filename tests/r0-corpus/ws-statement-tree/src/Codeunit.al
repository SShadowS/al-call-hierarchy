codeunit 50200 "Statement Tree Test"
{
    /// Straight-line: SetLoadFields + FindSet, no branching.
    procedure StraightLine()
    var
        Customer: Record Customer;
    begin
        Customer.SetLoadFields(Name);
        Customer.FindSet();
    end;

    /// If with else.
    procedure IfElse()
    var
        Customer: Record Customer;
        Found: Boolean;
    begin
        if Found then begin
            Customer.FindFirst();
        end else begin
            Customer.FindSet();
        end;
    end;

    /// Case with two branches and an else branch.
    procedure CaseStatement()
    var
        Customer: Record Customer;
        X: Integer;
    begin
        case X of
            1: begin
                Customer.FindFirst();
            end;
            2: begin
                Customer.FindSet();
            end;
            else begin
                Customer.FindLast();
            end;
        end;
    end;

    /// While loop.
    procedure WhileLoop()
    var
        Customer: Record Customer;
        I: Integer;
    begin
        while I > 0 do begin
            Customer.FindSet();
            I := I - 1;
        end;
    end;

    /// Repeat..until loop.
    procedure RepeatLoop()
    var
        Customer: Record Customer;
        I: Integer;
    begin
        repeat
            Customer.FindSet();
            I := I + 1;
        until I > 10;
    end;

    /// For loop.
    procedure ForLoop()
    var
        Customer: Record Customer;
        I: Integer;
    begin
        for I := 1 to 10 do begin
            Customer.FindSet();
        end;
    end;

    /// Foreach loop.
    procedure ForeachLoop()
    var
        Customer: Record Customer;
        Names: List of [Text];
        N: Text;
    begin
        foreach N in Names do begin
            Customer.FindFirst();
        end;
    end;

    /// Exit leaf.
    procedure WithExit()
    var
        Customer: Record Customer;
    begin
        Customer.SetLoadFields(Name);
        exit;
    end;

    /// Error call - AL's Error is a bare function call, not a dedicated statement.
    /// Verify it produces a "call" or "error" leaf (not an "op" leaf).
    procedure WithError()
    begin
        Error('Something went wrong');
    end;

    /// If without else (elseChildren should be absent).
    procedure IfNoElse()
    var
        Customer: Record Customer;
    begin
        if true then begin
            Customer.FindSet();
        end;
    end;

    /// Single-statement then-branch (no begin/end) — exercises buildCFNForBranchBody.
    procedure SingleStmtIfThen()
    var
        Customer: Record Customer;
    begin
        if true then
            Customer.FindSet();
    end;

    /// Single-statement then and else branches (no begin/end on either side).
    procedure SingleStmtIfElse()
    var
        Customer: Record Customer;
    begin
        if true then
            Customer.Get('X')
        else
            Customer.FindFirst();
    end;

    /// Single-statement while body (no begin/end).
    procedure SingleStmtWhile()
    var
        Customer: Record Customer;
    begin
        while true do
            Customer.FindSet();
    end;

    /// Single-statement case branches AND single-statement case else branch.
    procedure SingleStmtCaseElse()
    var
        Customer: Record Customer;
    begin
        case 1 of
            1:
                Customer.FindSet();
            else
                Customer.FindFirst();
        end;
    end;

    /// Nested: single-statement if whose body is itself a while loop with a body block.
    /// The inner FindSet op must remain reachable in the CFN tree.
    procedure NestedIfWhile()
    var
        Customer: Record Customer;
    begin
        if true then
            while true do begin
                Customer.FindSet();
            end;
    end;

    /// Single-statement for-loop body.
    procedure SingleStmtFor()
    var
        Customer: Record Customer;
        I: Integer;
    begin
        for I := 1 to 10 do
            Customer.FindSet();
    end;

    /// Single-statement foreach body.
    procedure SingleStmtForeach()
    var
        Customer: Record Customer;
        Names: List of [Text];
        N: Text;
    begin
        foreach N in Names do
            Customer.FindSet();
    end;

    /// P7.5: record-op in if-condition (expression position).
    procedure IfCondCall()
    var
        Customer: Record Customer;
    begin
        if Customer.FindSet() then
            exit;
    end;

    /// P7.5: record-op in repeat-until condition (post-body iteration).
    procedure RepeatUntilCall()
    var
        Customer: Record Customer;
    begin
        Customer.SetRange("No.", 'C0001');
        if Customer.FindSet() then
            repeat
                Message(Customer.Name);
            until Customer.Next() = 0;
    end;

    /// P7.5: record-op in function-call argument (arg-evaluates-before-call).
    procedure HelperArgCall()
    var
        Customer: Record Customer;
    begin
        if HelperBool(Customer.FindSet()) then
            exit;
    end;

    /// P7.5: record-op in while-condition.
    procedure WhileCondCall()
    var
        Customer: Record Customer;
    begin
        while Customer.Next() > 0 do
            Message(Customer.Name);
    end;

    /// P7.5: record-op in case-value expression.
    procedure CaseValueCall()
    var
        Customer: Record Customer;
    begin
        case Customer.Find('-') of
            true:
                Message(Customer.Name);
        end;
    end;

    /// P7.5: record-op in for-range start expression.
    procedure ForStartCall()
    var
        Customer: Record Customer;
        I: Integer;
    begin
        for I := Customer.Count() to 10 do
            Message(I);
    end;

    /// P7.5 review fix: chained-receiver in if-condition.
    /// The condition `Helper(Customer).FindSet()` is a `call_expression` whose
    /// function-side is a `member_expression` whose object is itself a
    /// `call_expression` (`Helper(Customer)`). Both the inner Helper callsite
    /// AND the outer FindSet record-op must appear as `conditionLeaves` of the
    /// if-node — previously the harvester emitted only FindSet and silently
    /// dropped the receiver call.
    procedure ChainedReceiverInCond()
    var
        Customer: Record Customer;
    begin
        if HelperRec(Customer).FindSet() then
            exit;
    end;

    /// Helper for HelperArgCall.
    local procedure HelperBool(B: Boolean): Boolean
    begin
        exit(B);
    end;

    /// Helper for ChainedReceiverInCond: takes a record by var and returns a record.
    local procedure HelperRec(var C: Record Customer): Record Customer
    begin
        exit(C);
    end;
}
