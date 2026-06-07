// R2c fixture: `maybe` resolution + a subscriber carrying a non-empty elementName.
//
// `Target Engine` exists in the workspace but declares NO event-publisher routine
// named `OnSomethingHappened`. A subscriber targeting that (existing object, no real
// publisher routine) → resolution "maybe" + a synthesized "unknown"-kind EventSymbol
// whose publisherObjectId is the REAL target object id (conforming, NOT a sentinel).
//
// The Table subscriber carries an elementName ('Amount') on a field-bound event so
// the corpus exercises the elementName projection branch.

codeunit 50100 "Target Engine"
{
    // A NON-event procedure with the same shape — proves "maybe" is about the
    // [IntegrationEvent]/[BusinessEvent] publisher attribute, not just name match.
    procedure OnSomethingHappened()
    begin
    end;

    [IntegrationEvent(false, false)]
    procedure OnRealEvent()
    begin
    end;
}

table 50000 "Target Record"
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; Amount; Decimal) { }
    }
    keys { key(PK; "No.") { } }
}

codeunit 50101 "R2c Listener"
{
    // Existing target object, but `OnSomethingHappened` is NOT an event publisher
    // routine → resolution "maybe".
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Target Engine", 'OnSomethingHappened', '', false, false)]
    local procedure HandleMaybe()
    begin
    end;

    // Real publisher in the workspace → resolution "resolved".
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Target Engine", 'OnRealEvent', '', false, false)]
    local procedure HandleResolved()
    begin
    end;

    // Subscriber with a non-empty elementName on a field-bound table event.
    [EventSubscriber(ObjectType::Table, Database::"Target Record", 'OnAfterValidateEvent', 'Amount', false, false)]
    local procedure HandleFieldEvent()
    begin
    end;
}
