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

    // NOT FLAGGED: deliberate best-effort fallback — the SAME try is consumed
    // (checked) first, so the trailing statement-position retry is intentional
    // (its failure is acceptably ignored). The DO false-positive shape.
    procedure FallbackRetry()
    begin
        if not TryStep() then
            TryStep();
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
