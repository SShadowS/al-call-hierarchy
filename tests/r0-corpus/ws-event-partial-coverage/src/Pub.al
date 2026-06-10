codeunit 50000 PartialPub
{
    procedure Fire(var C: Record Customer) begin OnE(C); end;

    [IntegrationEvent(false, false)]
    procedure OnE(var C: Record Customer) begin end;
}
