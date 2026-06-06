codeunit 50002 "Upgrade Handler"
{
    Subtype = Upgrade;

    trigger OnUpgradePerCompany()
    begin
        DoUpgrade();
    end;

    local procedure DoUpgrade()
    var
        Setup: Record "My Setup";
    begin
        if Setup.Get() then begin
            Setup.Description := 'migrated';
            Setup.Modify();
        end;
        Commit();
    end;
}
