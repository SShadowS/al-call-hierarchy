// D34-positive: Commit inside a loop
codeunit 50000 "Preset Looper"
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

// D47-positive: HTTP inside an open write transaction
codeunit 50001 "Preset Sender"
{
    procedure SendAfterModify()
    var
        Rec: Record "Preset Rec";
        Client: HttpClient;
        Resp: HttpResponseMessage;
    begin
        Rec.Get(1);
        Rec.Name := 'changed';
        Rec.Modify();
        Client.Get('https://example.test/ping', Resp);
    end;
}
