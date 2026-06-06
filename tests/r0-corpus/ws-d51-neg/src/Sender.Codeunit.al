codeunit 50201 "D51 NegSender"
{
    /// <summary>
    /// Negative case: read-direction HTTP GET then Error().
    /// GET is read-direction — gradeGuarantee suppresses IO_BEFORE_ESCAPING_ERROR
    /// for read-direction IO → D51 emits ZERO findings.
    /// </summary>
    procedure GetThenError()
    var
        Client: HttpClient;
        Resp: HttpResponseMessage;
    begin
        Client.Get('https://api.example.test/orders', Resp);
        Error('Something went wrong');
    end;
}
