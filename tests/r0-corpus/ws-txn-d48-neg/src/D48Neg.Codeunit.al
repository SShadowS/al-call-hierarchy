codeunit 50000 "D48 Sender Neg"
{
    /// <summary>
    /// Sends one HTTP POST outside any loop — D48 must NOT fire.
    /// </summary>
    procedure SendOnce()
    var
        Client: HttpClient;
        Req: HttpRequestMessage;
        Resp: HttpResponseMessage;
    begin
        Req.Method := 'POST';
        Client.Send(Req, Resp);
    end;
}
