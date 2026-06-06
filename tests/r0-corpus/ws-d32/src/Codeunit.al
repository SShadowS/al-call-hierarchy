codeunit 51500 "D32 Demo"
{
    // Two callers, both passing `true` — `AlwaysTrue` should be flagged.
    procedure DriverA()
    begin
        Helper(true);
        Helper(true);
    end;

    // FLAGGED target: Boolean param always passed `true` across all primary callers.
    local procedure Helper(AlwaysTrue: Boolean)
    begin
        if AlwaysTrue then
            Message('hi');
    end;

    // Two callers passing different literals — `Toggle` is genuinely variable, not flagged.
    procedure DriverB()
    begin
        Toggle(true);
        Toggle(false);
    end;

    local procedure Toggle(Flag: Boolean)
    begin
        if Flag then
            Message('on')
        else
            Message('off');
    end;

    // NOT FLAGGED: only one caller — D32 requires >=2 to claim "always".
    procedure DriverC()
    begin
        SingleCallSite(false);
    end;

    local procedure SingleCallSite(Flag: Boolean)
    begin
        if Flag then Message('x');
    end;

    // NOT FLAGGED: non-Boolean parameter is out of scope.
    procedure DriverD()
    begin
        Counter(5);
        Counter(5);
    end;

    local procedure Counter(N: Integer)
    begin
        Message(Format(N));
    end;
}
