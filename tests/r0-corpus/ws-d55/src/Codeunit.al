codeunit 50924 "D55 Events"
{
    [IntegrationEvent(false, false)]
    procedure OnRowProcessed()
    begin
    end;
}

codeunit 50925 "D55 Demo"
{
    // FLAGGED: publish per iteration — every subscriber runs once per row.
    procedure PublishInLoop()
    var
        Item: Record "D55 Item";
        Ev: Codeunit "D55 Events";
    begin
        if Item.FindSet() then
            repeat
                Ev.OnRowProcessed();
            until Item.Next() = 0;
    end;

    // NOT FLAGGED: publish once after the loop.
    procedure PublishAfterLoop()
    var
        Item: Record "D55 Item";
        Ev: Codeunit "D55 Events";
    begin
        if Item.FindSet() then
            repeat
            until Item.Next() = 0;
        Ev.OnRowProcessed();
    end;
}
