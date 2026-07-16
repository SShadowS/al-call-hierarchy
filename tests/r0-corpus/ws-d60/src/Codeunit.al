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

    // NOT FLAGGED: the loop body CALLS a routine per row — DataTransfer cannot
    // invoke code per row.
    procedure UpgradeWithCall()
    var
        Item: Record "D60 Item";
    begin
        if Item.FindSet() then
            repeat
                Item.Name := Compute();
                Item.Modify();
            until Item.Next() = 0;
    end;

    // NOT FLAGGED: the loop body reads ANOTHER record (cross-table lookup) —
    // not a set-based copy DataTransfer can express.
    procedure UpgradeWithOtherRecord()
    var
        Item: Record "D60 Item";
        Ref: Record "D60 Ref";
    begin
        if Item.FindSet() then
            repeat
                Ref.Get(Item."No.");
                Item.Modify();
            until Item.Next() = 0;
    end;

    // NOT FLAGGED: the loop body COMPUTES the value under an if/case — a
    // conditional DataTransfer cannot express.
    procedure UpgradeWithConditional()
    var
        Item: Record "D60 Item";
    begin
        if Item.FindSet() then
            repeat
                if Item.Name = '' then
                    Item.Name := 'default'
                else
                    Item.Name := 'set';
                Item.Modify();
            until Item.Next() = 0;
    end;

    local procedure Compute(): Text
    begin
        exit('x');
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
