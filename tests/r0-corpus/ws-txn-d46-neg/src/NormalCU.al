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

// Normal (non-Install/Upgrade) codeunit — Commit must NOT be flagged by D46.
codeunit 50001 "Normal Handler"
{
    Subtype = Normal;

    procedure RunSetup()
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
