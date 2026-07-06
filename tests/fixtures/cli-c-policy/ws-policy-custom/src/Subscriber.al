// Event subscriber that performs a Commit — targets the all/in rules.
codeunit 50201 CustomSub
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::CustomPub, 'OnCustomEvent', '', false, false)]
    local procedure HandleCustomEvent()
    begin
        Commit();
    end;
}
