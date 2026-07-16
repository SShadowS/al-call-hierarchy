codeunit 50921 "D53 Demo"
{
    [TryFunction]
    procedure TryStep()
    begin
        Error('boom');
    end;

    // FLAGGED: statement-position call — a TryFunction failure is silently swallowed.
    procedure IgnoresResult()
    begin
        TryStep();
    end;

    // NOT FLAGGED: result consumed by the if.
    procedure ChecksResult()
    begin
        if not TryStep() then
            Error('step failed');
    end;

    // NOT FLAGGED: under asserterror (deliberate negative-path assertion).
    procedure UnderAssert()
    begin
        asserterror TryStep();
    end;

    // NOT FLAGGED: plain (non-Try) procedure called in statement position.
    procedure PlainStep()
    begin
    end;

    procedure CallsPlain()
    begin
        PlainStep();
    end;
}
