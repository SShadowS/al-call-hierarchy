// PostAndCommit's own `commit` fact is DIRECT for this routine, but reaches
// Table.al's OnInsert trigger only through the call graph — i.e. INHERITED
// from OnInsert's point of view. `internal` (not `local`) so Table.al can call
// it, and so root-classification's public-procedure catch-all does not also
// pick it up (it has an explicit access modifier).
codeunit 50105 "Posting Helper"
{
    internal procedure PostAndCommit()
    begin
        Commit();
    end;
}
