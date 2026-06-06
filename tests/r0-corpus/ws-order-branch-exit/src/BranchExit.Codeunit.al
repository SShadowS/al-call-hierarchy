codeunit 50101 "BE BranchExit"
{
    procedure RunBranchExit(Condition: Boolean)
    var
        Client: HttpClient;
        Response: HttpResponseMessage;
    begin
        if Condition then begin
            Client.Get('https://example.com/notify', Response);
            exit;
        end;
        Commit();
    end;
}
