codeunit 50003 Sub3
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::BigPub, 'OnE', '', false, false)]
    local procedure H3(var C: Record Customer) begin end;
}
