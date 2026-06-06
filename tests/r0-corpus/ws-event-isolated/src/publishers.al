codeunit 52100 "Isolated Event Publisher"
{
    // Isolated=true (3rd positional arg for IntegrationEvent, index 2)
    [IntegrationEvent(false, false, true)]
    procedure OnIsolated()
    begin
    end;

    // Isolated arg absent — AL default false (non-isolated)
    [IntegrationEvent(false, false)]
    procedure OnNormal()
    begin
    end;

    // Isolated=false explicitly
    [IntegrationEvent(true, false, false)]
    procedure OnExplicitFalse()
    begin
    end;

    // BusinessEvent: Isolated=true (2nd positional arg, index 1)
    [BusinessEvent(false, true)]
    procedure OnBizIsolated()
    begin
    end;

    // BusinessEvent: Isolated arg absent
    [BusinessEvent(false)]
    procedure OnBizNormal()
    begin
    end;
}
