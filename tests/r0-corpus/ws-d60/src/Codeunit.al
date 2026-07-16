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

    // NOT FLAGGED: the loop body COMPUTES the value under a parenthesized,
    // quoted-field `if` — a conditional DataTransfer cannot express. The parens +
    // quoted field mirror the real DO shape that the identifier-only
    // condition_references collection misses; the structural statement-tree walk
    // still catches it.
    procedure UpgradeWithConditional()
    var
        Item: Record "D60 Item";
    begin
        if Item.FindSet() then
            repeat
                if (Item."No." = '') then
                    Item.Name := 'default'
                else
                    Item.Name := 'set';
                Item.Modify();
            until Item.Next() = 0;
    end;

    // NOT FLAGGED: the loop body branches on a `case` over a quoted field — same
    // structural reason (mirrors the DO UpgradeSendCode shape).
    procedure UpgradeWithCase()
    var
        Item: Record "D60 Item";
    begin
        if Item.FindSet() then
            repeat
                case Item."No." of
                    '':
                        Item.Name := 'empty';
                    else
                        Item.Name := 'other';
                end;
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
