codeunit 51101 "Writer CU"
{
    procedure WriteIt()
    var
        Item: Record Item;
    begin
        Item.Modify(true);
    end;

    procedure ValidateIt()
    var
        Item: Record Item;
    begin
        Item.Validate(Description);
    end;

    procedure UnknownIt()
    var
        Ledger: Record "G/L Entry";
    begin
        Ledger.Modify(true);
    end;
}
