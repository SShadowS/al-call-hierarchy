// Publisher codeunit — exposes:
//   • OnAfterPost()          0-param IntegrationEvent (overload A)
//   • OnAfterPost(n: Integer) 1-param IntegrationEvent (overload B)
//   • OnBeforePost()          0-param BusinessEvent
//   • OnInternalEvent()       0-param InternalEvent
codeunit 50100 "EventPublisher"
{
    [IntegrationEvent(false, false)]
    procedure OnAfterPost()
    begin
    end;

    [IntegrationEvent(false, false)]
    procedure OnAfterPost(n: Integer)
    begin
    end;

    [BusinessEvent(false)]
    procedure OnBeforePost()
    begin
    end;

    [InternalEvent(false, false)]
    procedure OnInternalEvent()
    begin
    end;
}
