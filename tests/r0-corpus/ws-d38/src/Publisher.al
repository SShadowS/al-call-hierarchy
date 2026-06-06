codeunit 51400 "D38 Publisher"
{
    // Publisher routine carrying [Obsolete(Pending)] — subscribers should warn (info).
    [IntegrationEvent(false, false)]
    [Obsolete('Use OnAfterDoStuffV2; this will be removed in 25.0.', '24.0')]
    procedure OnAfterDoStuffPending(Arg: Integer)
    begin
    end;

    // Publisher routine carrying [Obsolete(Removed)] — subscribers will stop firing (high).
    [IntegrationEvent(false, false)]
    [Obsolete('Removed; use OnAfterDoStuffV2.', '25.0', ObsoleteState::Removed)]
    procedure OnAfterDoStuffRemoved(Arg: Integer)
    begin
    end;

    // Fresh publisher routine — subscribers should NOT be flagged.
    [IntegrationEvent(false, false)]
    procedure OnAfterFresh(Arg: Integer)
    begin
    end;

    procedure DoStuff()
    begin
        OnAfterDoStuffPending(1);
        OnAfterDoStuffRemoved(2);
        OnAfterFresh(3);
    end;
}
