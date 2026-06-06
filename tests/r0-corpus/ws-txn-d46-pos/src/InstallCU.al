table 50000 "My Setup"
{
    Caption = 'My Setup';
    DataClassification = SystemMetadata;

    fields
    {
        field(1; "Primary Key"; Code[10]) { DataClassification = SystemMetadata; }
        field(2; Description; Text[100]) { DataClassification = CustomerContent; }
    }

    keys
    {
        key(PK; "Primary Key") { Clustered = true; }
    }
}

codeunit 50001 "Install Handler"
{
    Subtype = Install;

    trigger OnInstallAppPerCompany()
    begin
        DoSetup();
    end;

    local procedure DoSetup()
    var
        Setup: Record "My Setup";
    begin
        if not Setup.Get() then begin
            Setup.Init();
            Setup.Insert();
        end;
        Commit();
    end;
}
