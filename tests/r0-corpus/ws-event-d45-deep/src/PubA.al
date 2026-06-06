codeunit 50000 SalesMgr
{
    procedure RunSales(var S: Record Sales)
    begin
        OnAfterSales(S);
    end;

    [IntegrationEvent(false, false)]
    procedure OnAfterSales(var S: Record Sales)
    begin
    end;
}
