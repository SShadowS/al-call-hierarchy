codeunit 66102 "E2E Listener"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"E2E Engine", 'OnAfterRunIteration', '', true, true)]
    local procedure HandleIteration()
    var
        Customer: Record Customer;
    begin
        Customer.FindFirst();
    end;
}
