codeunit 50001 P2
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::P1, 'OnA', '', false, false)]
    local procedure H1(var C: Record Customer) begin OnB(C); end;

    [IntegrationEvent(false, false)]
    procedure OnB(var C: Record Customer) begin end;
}
