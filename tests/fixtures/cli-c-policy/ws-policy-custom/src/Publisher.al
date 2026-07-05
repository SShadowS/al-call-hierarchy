// Integration event publisher used by the subscriber below.
codeunit 50200 CustomPub
{
    procedure Fire()
    begin
        OnCustomEvent();
    end;

    [IntegrationEvent(false, false)]
    procedure OnCustomEvent()
    begin
    end;
}
