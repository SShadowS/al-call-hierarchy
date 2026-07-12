codeunit 50101 "Beta"
{
    procedure Process()
    begin
    end;

    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Alpha", 'OnAfterWork', '', false, false)]
    local procedure HandleAfterWork()
    begin
    end;
}
