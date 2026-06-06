codeunit 50142 "EN Caller"
{
    procedure Run()
    var
        T: Codeunit "EN Target";
    begin
        T.P("Probe Kind"::Open);
    end;
}
