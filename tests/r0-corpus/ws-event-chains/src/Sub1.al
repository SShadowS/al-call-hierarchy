codeunit 50001 SubA
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::PubA, 'OnE', '', false, false)]
    local procedure H1(var Cust: Record Customer) begin Cust.Modify(true); end;
}
