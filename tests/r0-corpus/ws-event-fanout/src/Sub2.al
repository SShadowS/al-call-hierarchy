codeunit 50002 Sub2
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::BigPub, 'OnE', '', false, false)]
    local procedure H2(var C: Record Customer) begin end;
}
