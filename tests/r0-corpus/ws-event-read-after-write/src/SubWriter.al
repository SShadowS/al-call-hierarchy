codeunit 50001 InvWriter
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::InvMgr, 'OnAfterAdjust', '', false, false)]
    local procedure HW(var Inv: Record Inventory)
    begin
        Inv.Qty := Inv.Qty + 1;
        Inv.Modify(true);
    end;
}
