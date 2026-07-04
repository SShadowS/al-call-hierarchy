// beyond-1B.3b Task 3 fixture (f): bare `Update();` collides in name+arity
// between the implicit table's own `Update` procedure
// (`IRPageIntrinsicTable.Table.al`) and `PageInstance`'s catalog entry
// `Update`. Pre pageext-merge-and-final-residual plan Task 2, this failed
// closed to `Unknown` (the PROBE-THEN-DECIDE guard had no compiler-verified
// precedence rule). Task 2 GROUNDED `Update` (and 18 sibling PAGE_INSTANCE
// names) as having NO bare-call form anywhere in AL — always
// receiver-qualified (`CurrPage.Update()`) — so it is no longer a real
// competing reading here: the table's own `Update` procedure now wins
// outright. See `resolver::INSTANCE_ONLY_NEVER_BARE`'s doc for the
// per-name MS Learn citations.
page 50981 "IR Page F"
{
    SourceTable = "IR Page Intrinsic Table";

    layout
    {
        area(Content)
        {
        }
    }

    trigger OnOpenPage()
    begin
        Update();
    end;
}
