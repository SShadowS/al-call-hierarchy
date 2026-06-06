codeunit 50002 Sub2
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::Publisher, 'OnAfterPost', '', false, false)]
    local procedure HandleAfterPost2(var W: Record Widget)
    begin
        W.Modify(false);
    end;
}
