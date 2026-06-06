codeunit 50151 "CH Caller"
{
    procedure Run(C: Char)
    var
        T: Codeunit "CH Target";
    begin
        T.P(C);
    end;
}
