codeunit 50122 "CE Caller"
{
    procedure Run()
    var
        T: Codeunit "CE Target";
    begin
        T.P(GetCount());
    end;

    local procedure GetCount(): Integer
    begin
        exit(5);
    end;
}
