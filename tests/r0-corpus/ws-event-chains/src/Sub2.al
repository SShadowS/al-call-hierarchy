codeunit 50002 SubB
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::PubA, 'OnE', '', false, false)]
    local procedure H2(var Cust: Record Customer) begin Cust.Insert(true); end;
}
