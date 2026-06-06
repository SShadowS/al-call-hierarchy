codeunit 50000 "D47 Sender"
{
    /// <summary>
    /// The Insert() targets a temporary record — no physical write transaction is
    /// opened, so there is nothing pending at the external IO point → ZERO findings.
    /// </summary>
    procedure SendAfterTempInsert()
    var
        TempRec: Record "D47 Rec" temporary;
        Client: HttpClient;
        Resp: HttpResponseMessage;
    begin
        TempRec.Init();
        TempRec."No." := 1;
        TempRec.Insert();
        Client.Get('https://example.test/ping', Resp);
    end;
}
