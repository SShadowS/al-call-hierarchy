// D47-positive: HTTP inside an open write transaction → CRITICAL
codeunit 50200 "PR Summary Sender"
{
    procedure SendAfterModify()
    var
        Rec: Record "PR Summary Rec";
        Client: HttpClient;
        Resp: HttpResponseMessage;
    begin
        Rec.Get(1);
        Rec.Name := 'changed';
        Rec.Modify();
        Client.Get('https://example.test/ping', Resp);
    end;
}

// D34-positive: Commit inside a loop → HIGH
codeunit 50201 "PR Summary Looper"
{
    procedure CommitInLoop()
    var
        i: Integer;
    begin
        for i := 1 to 10 do begin
            DoWork(i);
            Commit();
        end;
    end;

    local procedure DoWork(_n: Integer) begin end;
}

// D48-positive: HTTP in a loop → HIGH
codeunit 50202 "PR Summary IoLooper"
{
    procedure SendInLoop()
    var
        Client: HttpClient;
        Resp: HttpResponseMessage;
        i: Integer;
    begin
        for i := 1 to 5 do begin
            Client.Get('https://example.test/item', Resp);
        end;
    end;
}
