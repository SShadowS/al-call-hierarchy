codeunit 50933 "D59 Events"
{
    // FLAGGED: writable security-guard boolean on a public integration event.
    [IntegrationEvent(false, false)]
    procedure OnCheckAccess(UserId: Code[50]; var HasAccess: Boolean)
    begin
    end;

    // FLAGGED: skip-style guard.
    [IntegrationEvent(false, false)]
    procedure OnBeforeValidate(var SkipValidation: Boolean)
    begin
    end;

    // NOT FLAGGED: IsHandled is the sanctioned extensibility handshake.
    [IntegrationEvent(false, false)]
    procedure OnBeforePost(var IsHandled: Boolean)
    begin
    end;

    // NOT FLAGGED: non-var boolean (subscribers cannot write it).
    [IntegrationEvent(false, false)]
    procedure OnAfterCheck(HasAccess: Boolean)
    begin
    end;

    // NOT FLAGGED: var boolean without a guard-ish name.
    [IntegrationEvent(false, false)]
    procedure OnCollect(var Found: Boolean)
    begin
    end;
}
