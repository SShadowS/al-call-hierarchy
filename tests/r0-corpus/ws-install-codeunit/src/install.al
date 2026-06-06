codeunit 50300 "Install Handler"
{
    Subtype = Install;

    trigger OnInstallAppPerCompany()
    begin
        DoInstallWork();
    end;

    procedure DoInstallWork()
    begin
    end;

    local procedure InternalHelper()
    begin
    end;
}
