codeunit 50939 "D62 Demo"
{
    // FLAGGED: LogUsage before the fallible write — failure after the log
    // overcounts the feature.
    procedure LogThenWork()
    var
        FeatureTelemetry: Codeunit "Feature Telemetry";
        Item: Record "D62 Item";
    begin
        FeatureTelemetry.LogUsage('D62', 'Demo', 'started');
        Item.Init();
        Item.Insert();
    end;

    // NOT FLAGGED: log after the last fallible operation.
    procedure WorkThenLog()
    var
        FeatureTelemetry: Codeunit "Feature Telemetry";
        Item: Record "D62 Item";
    begin
        Item.Init();
        Item.Insert();
        FeatureTelemetry.LogUsage('D62', 'Demo', 'done');
    end;
}
