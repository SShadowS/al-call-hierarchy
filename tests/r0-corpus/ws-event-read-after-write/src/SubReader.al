codeunit 50002 InvReader
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::InvMgr, 'OnAfterAdjust', '', false, false)]
    local procedure HR(var Inv: Record Inventory)
    var
        Other: Record Inventory;
    begin
        if Other.Get(Inv."No.") then;
    end;
}
