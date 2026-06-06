codeunit 50000 PubA
{
    procedure Fire(var Cust: Record Customer) begin OnE(Cust); end;
    [IntegrationEvent(false, false)]
    procedure OnE(var Cust: Record Customer) begin end;
}
