codeunit 50001 "D47 Isolated Subscriber"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"D47 Isolated Publisher", 'OnAfterProcessIsolated', '', false, false)]
    local procedure HandleAfterProcessIsolated()
    var
        Client: HttpClient;
        Content: HttpContent;
        Resp: HttpResponseMessage;
    begin
        Client.Post('https://example.test/notify', Content, Resp);
    end;
}
