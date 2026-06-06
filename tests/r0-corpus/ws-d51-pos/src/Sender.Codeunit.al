codeunit 50101 "D51 Sender"
{
    /// <summary>
    /// Positive case: write-direction HTTP POST then bare escaping Error().
    /// IO_BEFORE_ESCAPING_ERROR must fire → D51 emits a LOW finding.
    /// </summary>
    procedure PostThenError()
    var
        Client: HttpClient;
        Content: HttpContent;
        Resp: HttpResponseMessage;
    begin
        Client.Post('https://api.example.test/orders', Content, Resp);
        Error('Something went wrong');
    end;
}
