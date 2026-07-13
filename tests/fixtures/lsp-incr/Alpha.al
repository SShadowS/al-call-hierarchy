// Unicode smoke test (Task 10 fixture requirement): æøå
codeunit 50100 "Alpha"
{
    procedure DoWork()
    var
        Beta: Codeunit "Beta";
    begin
        Beta.Process();
        Calc(1);
        Calc('x');
        Løbenr();
    end;

    procedure Calc(X: Integer)
    begin
    end;

    procedure Calc(X: Text)
    begin
    end;

    procedure Løbenr()
    begin
    end;

    [IntegrationEvent(false, false)]
    procedure OnAfterWork()
    begin
    end;
}
