codeunit 50200 Processor
{
    procedure Compute(Input: Integer): Integer
    var
        Result: Integer;
        Client: HttpClient;
        Response: HttpResponseMessage;
    begin
        Result := Input * 2;
        Client.Get('https://example.com/api', Response);
        exit(Result);
    end;
}
