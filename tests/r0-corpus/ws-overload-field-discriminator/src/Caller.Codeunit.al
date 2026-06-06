codeunit 50132 "FD Caller"
{
    procedure Run(var Rec: Record "FD Rec")
    var
        T: Codeunit "FD Target";
    begin
        T.P(Rec.Amount);
    end;
}
