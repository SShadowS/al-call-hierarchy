codeunit 50500 "Public Worker"
{
    // Default-access procedure → AST classifies as public-procedure.
    procedure DoWork()
    begin
    end;
}

codeunit 50501 "Install Handler"
{
    Subtype = Install;

    trigger OnInstallAppPerCompany()
    begin
        DoInstallWork();
    end;

    procedure DoInstallWork()
    begin
    end;
}

page 50502 "Sales Order API"
{
    PageType = API;
    SourceTable = Integer;

    procedure ApiHelper()
    begin
    end;
}
