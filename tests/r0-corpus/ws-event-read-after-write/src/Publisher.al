codeunit 50000 InvMgr
{
    procedure Adjust(var Inv: Record Inventory)
    begin
        OnAfterAdjust(Inv);
    end;

    [IntegrationEvent(false, false)]
    procedure OnAfterAdjust(var Inv: Record Inventory)
    begin
    end;
}
