codeunit 50102 "XI Caller"
{
    procedure Run()
    var
        Rec: Record "XI Rec";
        Worker: Codeunit "XI Worker";
        Client: HttpClient;
        Response: HttpResponseMessage;
    begin
        Client.Get('https://example.com/blob', Response);
        Worker.DoCommit();
    end;
}
