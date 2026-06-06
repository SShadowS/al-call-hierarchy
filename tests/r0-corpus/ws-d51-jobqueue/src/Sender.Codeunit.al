codeunit 50101 "D51 Sender"
{
    /// <summary>
    /// Retryable-entrypoint case: write-direction HTTP POST then bare escaping Error().
    /// Declared a job-queue-entrypoint via roots.config.json → D51 escalates the finding
    /// confidence to `confirmed` and uses the definite retry wording.
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
