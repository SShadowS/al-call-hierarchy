codeunit 50000 P1
{
    procedure Fire1(var C: Record Customer) begin OnA(C); end;
    [IntegrationEvent(false, false)]
    procedure OnA(var C: Record Customer) begin end;
}
