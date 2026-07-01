codeunit 53940 "SelfLocalCaller"
{
    local procedure LocalProc()
    begin
    end;

    procedure Trigger()
    begin
        this.LocalProc();
    end;
}
