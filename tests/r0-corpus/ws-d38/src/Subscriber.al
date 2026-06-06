codeunit 51401 "D38 Subscriber"
{
    // FLAGGED (info): bound to a Pending-obsolete publisher event.
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"D38 Publisher", 'OnAfterDoStuffPending', '', false, false)]
    local procedure HandlePending(Arg: Integer)
    begin
    end;

    // FLAGGED (high): bound to a Removed-obsolete publisher event.
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"D38 Publisher", 'OnAfterDoStuffRemoved', '', false, false)]
    local procedure HandleRemoved(Arg: Integer)
    begin
    end;

    // NOT FLAGGED: publisher event is not obsolete.
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"D38 Publisher", 'OnAfterFresh', '', false, false)]
    local procedure HandleFresh(Arg: Integer)
    begin
    end;

    // NOT FLAGGED (skipped: unresolved): publisher codeunit does not exist in the
    // workspace, so the edge resolves to "unknown" and never reaches obsolete check.
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"D38 NonExistent", 'OnSomethingUnknown', '', false, false)]
    local procedure HandleUnknown()
    begin
    end;
}
