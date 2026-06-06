codeunit 50001 Sub1
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::BigPub, 'OnE', '', false, false)]
    local procedure H1(var C: Record Customer) begin end;
}
