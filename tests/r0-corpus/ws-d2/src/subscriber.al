codeunit 64102 "D2 Subscriber"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"D2 Publisher", 'OnProcessLine', '', true, true)]
    local procedure HandleProcessLine()
    var
        Customer: Record Customer;
    begin
        Customer.FindSet();
    end;

    [EventSubscriber(ObjectType::Codeunit, Codeunit::"D2 Publisher", 'OnQuietEvent', '', true, true)]
    local procedure HandleQuietEvent()
    begin
    end;
}
