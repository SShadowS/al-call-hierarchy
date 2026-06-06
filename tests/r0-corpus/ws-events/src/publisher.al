codeunit 51200 "Work Engine"
{
    procedure DoWork()
    begin
        OnAfterDoWork();
    end;

    [IntegrationEvent(false, false)]
    procedure OnAfterDoWork()
    begin
    end;
}
