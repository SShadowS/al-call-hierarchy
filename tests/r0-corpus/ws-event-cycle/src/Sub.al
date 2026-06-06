codeunit 50002 P3
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::P2, 'OnB', '', false, false)]
    local procedure H2(var C: Record Customer)
    var
        X: Codeunit P1;
    begin
        X.Fire1(C);
    end;
}
