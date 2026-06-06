codeunit 50001 "D47 Evt Subscriber"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"D47 Evt Publisher", 'OnAfterProcess', '', false, false)]
    local procedure HandleAfterProcess()
    var
        Client: HttpClient;
        Content: HttpContent;
        Resp: HttpResponseMessage;
    begin
        Client.Post('https://example.test/notify', Content, Resp);
    end;
}
