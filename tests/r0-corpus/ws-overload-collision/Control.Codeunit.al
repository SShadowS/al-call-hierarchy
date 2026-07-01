// beyond-1B.3b Task 2 control fixture: a SINGLE overload of `Solo` — no
// same-arity collision. Exists to prove the ambiguity guard added for
// `Ambiguous.Resolve` does NOT over-fire on ordinary, unambiguous procedures;
// a call to `Solo` must still resolve cleanly to `Evidence::Source`.
codeunit 50961 "Overload Collision Control"
{
    procedure Solo(X: Integer)
    begin
        Message('solo');
    end;
}
