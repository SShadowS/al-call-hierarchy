codeunit 50001 CycleB
{
    procedure FireB()
    begin
        OnB();
    end;

    [IntegrationEvent(false, false)]
    procedure OnB()
    begin
    end;

    [EventSubscriber(ObjectType::Codeunit, Codeunit::CycleA, 'OnA', '', false, false)]
    local procedure HandlerA()
    begin
        FireB();
    end;
}
