codeunit 50101 "OI IoBeforeCommit"
{
    procedure SendThenPost()
    var
        Rec: Record "OI Rec";
        Client: HttpClient;
        Response: HttpResponseMessage;
    begin
        Client.Get('https://example.com/notify', Response);
        Rec.Init();
        Rec.Insert(true);
        Commit();
    end;
}
