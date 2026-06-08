codeunit 70003 "Host Unlinked Sub"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Dep Mgt", 'OnNeverPublished', '', true, true)]
    local procedure HandleOnNeverPublished()
    begin
    end;
}