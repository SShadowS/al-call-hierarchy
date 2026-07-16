codeunit 50936 "D61 Events"
{
    [IntegrationEvent(false, false)]
    procedure OnBeforePost(var IsHandled: Boolean)
    begin
    end;

    [IntegrationEvent(false, false)]
    procedure OnBeforeLog(var IsHandled: Boolean)
    begin
    end;
}

codeunit 50937 "D61 Poster"
{
    // The guarded critical write: subscriber flipping IsHandled skips the Modify.
    procedure Post()
    var
        Ev: Codeunit "D61 Events";
        Item: Record "D61 Item";
        IsHandled: Boolean;
    begin
        Ev.OnBeforePost(IsHandled);
        if not IsHandled then begin
            Item.FindFirst();
            Item.Posted := true;
            Item.Modify();
        end;
    end;

    // Guard skips only a Message — nothing critical; not flagged.
    procedure Log()
    var
        Ev: Codeunit "D61 Events";
        IsHandled: Boolean;
    begin
        Ev.OnBeforeLog(IsHandled);
        if not IsHandled then
            Message('logged');
    end;
}

codeunit 50938 "D61 Subscribers"
{
    // FLAGGED (with the Post guard): unconditionally claims handled — the
    // publisher-side Modify is silently skipped.
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"D61 Events", 'OnBeforePost', '', false, false)]
    local procedure HandlePost(var IsHandled: Boolean)
    begin
        IsHandled := true;
    end;

    // NOT FLAGGED: subscribes to the Message-only event.
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"D61 Events", 'OnBeforeLog', '', false, false)]
    local procedure HandleLog(var IsHandled: Boolean)
    begin
        IsHandled := true;
    end;
}
