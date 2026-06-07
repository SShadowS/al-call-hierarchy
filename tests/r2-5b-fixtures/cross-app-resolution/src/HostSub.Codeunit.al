codeunit 70001 "Host Sub"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Dep Mgt", 'OnBeforeCompute', '', true, true)]
    local procedure HandleOnBeforeCompute(var Cust: Record "Dep Customer"; var Handled: Boolean)
    begin
    end;
}