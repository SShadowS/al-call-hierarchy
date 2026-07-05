codeunit 64101 "D2 Publisher"
{
    procedure RaiseInLoop()
    var
        i: Integer;
    begin
        for i := 1 to 10 do
            OnProcessLine();
    end;

    procedure RaiseQuietInLoop()
    var
        i: Integer;
    begin
        for i := 1 to 10 do
            OnQuietEvent();
    end;

    procedure RaiseTwiceInLoop()
    var
        i: Integer;
    begin
        for i := 1 to 10 do begin
            OnProcessLine();
            OnProcessLine();
        end;
    end;

    [IntegrationEvent(false, false)]
    procedure OnProcessLine()
    begin
    end;

    [IntegrationEvent(false, false)]
    procedure OnQuietEvent()
    begin
    end;
}
