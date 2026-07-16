codeunit 50934 "D60 Upgrade"
{
    Subtype = Upgrade;

    // FLAGGED: row-by-row rewrite in an upgrade codeunit — DataTransfer territory.
    trigger OnUpgradePerCompany()
    var
        Item: Record "D60 Item";
    begin
        if Item.FindSet() then
            repeat
                Item.Name := 'migrated';
                Item.Modify();
            until Item.Next() = 0;
    end;
}

codeunit 50935 "D60 Normal"
{
    // NOT FLAGGED: same loop outside an upgrade/install codeunit (d5/d10 territory).
    procedure RegularLoop()
    var
        Item: Record "D60 Item";
    begin
        if Item.FindSet() then
            repeat
                Item.Name := 'x';
                Item.Modify();
            until Item.Next() = 0;
    end;
}
