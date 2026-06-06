codeunit 50100 "Probe Caller"
{
    procedure RunWithCallExprArg(InStr: InStream; Acc: Code[20])
    var
        Sha: Codeunit "Probe Sha";
    begin
        Sha.InsertWithSha1(GetLog(), InStr, Acc);
    end;

    procedure RunWithPlainArg(InStr: InStream; Acc: Code[20])
    var
        Sha: Codeunit "Probe Sha";
        Tok: Integer;
    begin
        Sha.M(Tok, InStr, Acc);
    end;

    local procedure GetLog(): Interface "Probe ILog"
    var
        Log: Codeunit "Probe Log";
    begin
        exit(Log);
    end;
}
