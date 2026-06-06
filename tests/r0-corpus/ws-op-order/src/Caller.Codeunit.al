codeunit 50100 "OO Caller"
{
    procedure Run(InStr: InStream)
    var
        Rec: Record "OO Rec";
    begin
        Rec.Init();
        if GuardOpen() then begin
            Rec.Insert(true);
            exit;
        end;
        Rec.Modify(true);
    end;

    local procedure GuardOpen(): Boolean
    begin
        exit(true);
    end;
}
