codeunit 50927 "D57 Events"
{
    [IntegrationEvent(false, false)]
    procedure OnThing()
    begin
    end;
}

codeunit 50928 "D57 Leaky"
{
    SingleInstance = true;

    var
        SeenNames: List of [Text];
        TempLog: Record "D57 Log" temporary;

    // FLAGGED ×2: unbounded growth of session-lifetime state (list Add + temp Insert).
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"D57 Events", 'OnThing', '', false, false)]
    local procedure OnThingSub()
    begin
        SeenNames.Add('x');
        TempLog.Init();
        TempLog.Insert();
    end;
}

codeunit 50929 "D57 Drained"
{
    SingleInstance = true;

    var
        Pending: List of [Text];

    // NOT FLAGGED: a clearing path exists in the same object.
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"D57 Events", 'OnThing', '', false, false)]
    local procedure OnThingSub()
    begin
        Pending.Add('x');
    end;

    procedure Drain()
    begin
        Clear(Pending);
    end;
}

codeunit 50930 "D57 NotSingle"
{
    var
        Names: List of [Text];

    // NOT FLAGGED: object is not SingleInstance.
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"D57 Events", 'OnThing', '', false, false)]
    local procedure OnThingSub()
    begin
        Names.Add('x');
    end;
}
