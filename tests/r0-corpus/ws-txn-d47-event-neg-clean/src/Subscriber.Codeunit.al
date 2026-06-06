codeunit 50001 "D47 Clean Subscriber"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"D47 Clean Publisher", 'OnAfterNotify', '', false, false)]
    local procedure HandleAfterNotify()
    var
        Client: HttpClient;
        Content: HttpContent;
        Resp: HttpResponseMessage;
    begin
        Client.Post('https://example.test/notify', Content, Resp);
    end;
}
