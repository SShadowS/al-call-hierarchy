table 50941 "D64 Item"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }
    keys { key(PK; "No.") { } }
}

// FLAGGED (shape A, low): declared read-only but write operations not disabled.
page 50941 "D64 ReadOnly Leaky"
{
    PageType = API;
    SourceTable = "D64 Item";
    Editable = false;

    layout
    {
        area(Content)
        {
            field(No; Rec."No.") { }
        }
    }
}

// FLAGGED (shape B, info): no explicit write-surface declaration at all.
page 50942 "D64 Undeclared"
{
    PageType = API;
    SourceTable = "D64 Item";

    layout
    {
        area(Content)
        {
            field(No; Rec."No.") { }
        }
    }
}

// NOT FLAGGED: write surface explicitly closed.
page 50943 "D64 Closed"
{
    PageType = API;
    SourceTable = "D64 Item";
    Editable = false;
    InsertAllowed = false;
    ModifyAllowed = false;
    DeleteAllowed = false;

    layout
    {
        area(Content)
        {
            field(No; Rec."No.") { }
        }
    }
}

// NOT FLAGGED: not an API page.
page 50944 "D64 Card"
{
    PageType = Card;
    SourceTable = "D64 Item";

    layout
    {
        area(Content)
        {
            field(No; Rec."No.") { }
        }
    }
}

// NOT FLAGGED: temp-sourced API page. Shares shape A's trigger condition
// (Editable = false, no InsertAllowed/ModifyAllowed/DeleteAllowed) but
// SourceTableTemporary = true means Rec is a per-session TEMPORARY buffer —
// accepted OData writes persist nothing, so the write-surface hazard is
// materially void. Measured on Microsoft Base Application 28.0: `page 5462
// "API Routes"` and `page 5460 "Webhook Supported Resources"`.
page 50945 "D64 Temp Buffer"
{
    PageType = API;
    SourceTable = "D64 Item";
    SourceTableTemporary = true;
    Editable = false;

    layout
    {
        area(Content)
        {
            field(No; Rec."No.") { }
        }
    }
}
