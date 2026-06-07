// R2c fixture: mixed publisher kinds + isolated variants + mixed-case event name +
// an unknown-target subscriber (sentinel pseudo-id) — a single workspace touching
// most projection branches.

codeunit 50100 "Mixed Publishers"
{
    // Integration, isolated=true (index 2).
    [IntegrationEvent(false, false, true)]
    procedure OnIsolatedInt()
    begin
    end;

    // Integration, isolated explicit-false → field OMITTED.
    [IntegrationEvent(true, false, false)]
    procedure OnPlainInt()
    begin
    end;

    // Business, isolated=true (index 1).
    [BusinessEvent(false, true)]
    procedure OnIsolatedBiz()
    begin
    end;

    // Business, isolated absent.
    [BusinessEvent(false)]
    procedure OnPlainBiz()
    begin
    end;

    // Mixed-case name — the raw eventId lowercases it; the stable id uses the
    // case-preserved symbol eventName.
    [IntegrationEvent(false, false)]
    procedure OnMixedCaseEvent()
    begin
    end;
}

codeunit 50101 "Mixed Listener"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Mixed Publishers", 'OnIsolatedInt', '', false, false)]
    local procedure HandleIsoInt()
    begin
    end;

    // Subscribe with a different SOURCE casing than the publisher declared —
    // resolves through the lowercased raw eventId.
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Mixed Publishers", 'onmixedcaseevent', '', false, false)]
    local procedure HandleMixedCase()
    begin
    end;

    // Target object NOT in the workspace → "unknown" + sentinel pseudo-id symbol.
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Out Of World", 'OnGone', '', false, false)]
    local procedure HandleUnknown()
    begin
    end;
}
