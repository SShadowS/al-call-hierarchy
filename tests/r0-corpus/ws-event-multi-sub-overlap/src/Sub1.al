codeunit 50001 Sub1
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::Publisher, 'OnAfterPost', '', false, false)]
    local procedure HandleAfterPost1(var W: Record Widget)
    begin
        W.Modify(true);
    end;
}
