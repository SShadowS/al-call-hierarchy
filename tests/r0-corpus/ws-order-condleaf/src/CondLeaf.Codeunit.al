codeunit 50101 "CL CondLeaf"
{
    procedure RunCondLeaf()
    var
        Client: HttpClient;
        Response: HttpResponseMessage;
    begin
        if Client.Get('https://example.com/check', Response) then
            Commit();
    end;
}
