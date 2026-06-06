codeunit 51100 "D35 Demo"
{
    // FLAGGED (high, direct): subscriber commits directly in its body.
    [EventSubscriber(ObjectType::Codeunit, 50, 'OnAfterPost', '', false, false)]
    local procedure OnAfterPostDirect()
    begin
        Commit();
    end;

    // FLAGGED (high, transitive): subscriber reaches Commit via a callee.
    [EventSubscriber(ObjectType::Codeunit, 50, 'OnAfterValidate', '', false, false)]
    local procedure OnAfterValidateTransitive()
    begin
        PersistAudit();
    end;

    // NOT FLAGGED: subscriber doesn't commit (summary.commits == "no").
    [EventSubscriber(ObjectType::Codeunit, 50, 'OnBeforePrint', '', false, false)]
    local procedure OnBeforePrintSafe()
    begin
        Message('hi');
    end;

    // NOT FLAGGED: non-subscriber routine — out of scope (D34/D8 cover other commit cases).
    procedure NonSubscriberCommit()
    begin
        Commit();
    end;

    local procedure PersistAudit()
    begin
        Commit();
    end;
}
