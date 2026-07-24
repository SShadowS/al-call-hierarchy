// Unlike ws-policy-commit-in-trigger (a DIRECT commit in the trigger body),
// OnInsert here never calls Commit() itself — it only calls a helper codeunit
// procedure. `no-commit-in-triggers` (root.kinds: trigger-table, capability.op:
// commit) can therefore only match via the INHERITED half of OnInsert's
// capability cone. See Helper.al and PROVENANCE.md.
table 50105 "Sales Header"
{
    fields { field(1; "No."; Code[20]) { } }
    keys { key(PK; "No.") { Clustered = true; } }

    trigger OnInsert()
    var
        Helper: Codeunit "Posting Helper";
    begin
        Helper.PostAndCommit();
    end;
}
