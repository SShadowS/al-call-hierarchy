codeunit 50101 "BE BranchError"
{
    procedure RunBranchError(Condition: Boolean)
    var
        Client: HttpClient;
        Response: HttpResponseMessage;
    begin
        if Condition then begin
            Client.Get('https://example.com/notify', Response);
            Error('Aborted');
        end;
        Commit();
    end;
}
