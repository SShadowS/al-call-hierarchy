codeunit 50000 PubA
{
    procedure Fire()
    begin
        OnEvent();
    end;

    [IntegrationEvent(false, false)]
    procedure OnEvent()
    begin
    end;
}
