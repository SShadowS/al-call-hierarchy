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

    // NOT FLAGGED: the textbook success/failure idiom — LogUsage in the
    // success arm, Error() in the MUTUALLY EXCLUSIVE else arm of the same
    // if. Source-text order alone would see Error() "after" LogUsage, but
    // the two arms can never both execute in the same run.
    procedure IfSuccessLogElseError(Success: Boolean)
    var
        FeatureTelemetry: Codeunit "Feature Telemetry";
    begin
        if Success then
            FeatureTelemetry.LogUsage('D62', 'Demo', 'if-else')
        else
            Error('D62 failed');
    end;

    // NOT FLAGGED: the case analog — LogUsage in one case branch, Error() in
    // the mutually exclusive else branch of the same case statement.
    procedure CaseLogElseError(Choice: Integer)
    var
        FeatureTelemetry: Codeunit "Feature Telemetry";
    begin
        case Choice of
            1:
                FeatureTelemetry.LogUsage('D62', 'Demo', 'case-arm');
            else
                Error('D62 failed');
        end;
    end;

    // STILL FLAGGED (control): LogUsage followed sequentially by a fallible
    // write in the SAME arm — not mutually exclusive, so the finding must
    // keep firing.
    procedure LogThenWriteSameArm(Cond: Boolean)
    var
        FeatureTelemetry: Codeunit "Feature Telemetry";
        Item: Record "D62 Item";
    begin
        if Cond then begin
            FeatureTelemetry.LogUsage('D62', 'Demo', 'same-arm');
            Item.Init();
            Item.Insert();
        end;
    end;
}
