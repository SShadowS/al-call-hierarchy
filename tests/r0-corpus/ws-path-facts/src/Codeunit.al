codeunit 50400 "Path Facts"
{
    procedure ReadsBeforeLoad(var Cust: Record Customer)
    begin
        Message(Cust.Name);
    end;

    procedure MutatesBeforeLoad(var Cust: Record Customer)
    begin
        Cust.Modify();
    end;

    procedure LoadThenRead(var Cust: Record Customer)
    begin
        Cust.Get('C0001');
        Message(Cust.Name);
    end;

    procedure InitThenModify(var Cust: Record Customer)
    begin
        Cust.Init();
        Cust."No." := 'C0002';
        Cust.Insert();
    end;

    procedure ConditionalLoad(var Cust: Record Customer)
    begin
        // `if`-gated load: on the false branch Cust is never loaded but Modify still runs.
        // Phase 6 walker proves the no-load branch reaches Modify => requiresLoaded = "yes"
        // and mutatesBeforeLoad = "yes".
        if true then
            Cust.Get('C0001');
        Cust.Modify();
    end;

    procedure StraightForward(var Cust: Record Customer)
    begin
        MutatesBeforeLoad(Cust);
    end;

    procedure IfValidateElseModify(var Cust: Record Customer; X: Boolean)
    begin
        Cust.Get('C0001');
        if X then
            Cust.Validate(Name, 'X')
        else
            Cust.Modify();
        exit;
    end;

    procedure ValidateThenConditionalModify(var Cust: Record Customer; X: Boolean)
    begin
        Cust.Get('C0001');
        Cust.Validate(Name, 'X');
        if X then
            Cust.Modify();
        exit;
    end;

    procedure ValidateThenEarlyExit(var Cust: Record Customer; AbortNow: Boolean)
    begin
        Cust.Get('C0001');
        Cust.Validate(Name, 'X');
        if AbortNow then
            exit;
        Cust.Modify();
    end;

    procedure ModifyAllAfterValidate(var Cust: Record Customer)
    begin
        Cust.Get('C0001');
        Cust.Validate(Name, 'X');
        Cust.ModifyAll(Address, 'Y'); // set-based: does NOT clear dirty
    end;

    procedure SetLoadFieldsThenFindThenCall(var Cust: Record Customer)
    begin
        Cust.SetLoadFields("No.", Name);
        Cust.FindFirst();
        ReadsName(Cust);
    end;

    local procedure ReadsName(var Cust: Record Customer)
    begin
        Message(Cust.Name);
    end;
}
