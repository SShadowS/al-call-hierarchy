codeunit 50200 "May Facts"
{
    procedure LoadsFromDb(var Cust: Record Customer)
    begin
        Cust.Get('C0001');
    end;

    procedure Initialises(var Cust: Record Customer)
    begin
        Cust.Init();
    end;

    procedure PersistsCurrent(var Cust: Record Customer)
    begin
        Cust.Modify();
    end;

    procedure SetBasedWrites(var Cust: Record Customer)
    begin
        Cust.ModifyAll(Name, 'X');
    end;

    procedure Validates(var Cust: Record Customer)
    begin
        Cust.Validate(Name, 'Y');
    end;

    procedure CopiesInto(var Cust: Record Customer; var Other: Record Customer)
    begin
        Cust.Copy(Other);
    end;

    procedure ResetsFilter(var Cust: Record Customer)
    begin
        Cust.Reset();
    end;

    procedure Neutral(var Cust: Record Customer)
    begin
        Cust.SetRange("No.", 'C0001');
    end;

    procedure ForwardsToValidator(var Cust: Record Customer)
    begin
        Validates(Cust);
    end;

    procedure ForwardsByValue(Cust: Record Customer)
    begin
        Validates(Cust);
    end;
}
