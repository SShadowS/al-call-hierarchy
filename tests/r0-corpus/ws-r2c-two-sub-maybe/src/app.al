// R2c fixture: the FIXED two-subscribers-both-maybe invariant (al-sem ≥9eb9c55).
//
// `Hub` exists in the workspace but has NO event-publisher routine named
// `OnPing`. TWO subscribers target `Hub::OnPing`. Pre-fix, the 2nd subscriber
// falsely upgraded to "resolved" (it consulted `eventById`, which by then held the
// synthesized "maybe" symbol). FIXED: `resolved` consults `realPublisherEventIds`
// ONLY, so BOTH subscribers stay "maybe" and share ONE synthesized EventSymbol.

codeunit 50100 Hub
{
    // No [IntegrationEvent]/[BusinessEvent] named OnPing — a plain procedure.
    procedure OnPing()
    begin
    end;
}

codeunit 50101 "Sub One"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::Hub, 'OnPing', '', false, false)]
    local procedure HandleA()
    begin
    end;
}

codeunit 50102 "Sub Two"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::Hub, 'OnPing', '', false, false)]
    local procedure HandleB()
    begin
    end;
}
