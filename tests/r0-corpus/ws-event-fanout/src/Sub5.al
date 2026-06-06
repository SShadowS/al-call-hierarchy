codeunit 50005 Sub5
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::BigPub, 'OnE', '', false, false)]
    local procedure H5(var C: Record Customer) begin end;
}
