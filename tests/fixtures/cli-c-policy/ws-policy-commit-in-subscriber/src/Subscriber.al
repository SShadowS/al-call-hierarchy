codeunit 50001 SubA
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::PubA, 'OnEvent', '', false, false)]
    local procedure HandleEvent()
    begin
        Commit();
    end;
}
