codeunit 50001 SalesNotifier
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::SalesMgr, 'OnAfterSales', '', false, false)]
    local procedure HNotify(var S: Record Sales)
    begin
        OnNotifyComplete(S);
    end;

    [IntegrationEvent(false, false)]
    procedure OnNotifyComplete(var S: Record Sales)
    begin
    end;
}
