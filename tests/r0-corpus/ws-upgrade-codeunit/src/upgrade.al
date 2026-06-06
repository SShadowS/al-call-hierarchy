codeunit 50301 "Upgrade Handler"
{
    Subtype = Upgrade;

    trigger OnUpgradePerCompany()
    begin
        DoUpgradeWork();
    end;

    procedure DoUpgradeWork()
    begin
    end;
}
