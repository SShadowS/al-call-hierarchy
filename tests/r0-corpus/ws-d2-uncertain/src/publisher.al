// The d2 counterpart of `ws-d1-uncertain-path`: a d2 finding whose confidence
// carries a NON-EMPTY `evidence` list.
//
// Why this fixture exists: before it, exactly ONE golden in the repository
// (`ws-d1-uncertain-path`) exercised a non-empty `confidence.evidence`, and it
// was a d1 finding. d2 reads the same substrate — `sub_summary_uncertainties`
// borrows straight out of `ctx.uncertainties_by_node` — but nothing pinned its
// output, so a regression in d2's half of that path turned no golden red.
//
// ONE publisher, ONE event, ONE subscriber, ONE db op: a single d2 finding, so
// the golden cannot go flaky on ordering between findings.
codeunit 64201 "D2U Publisher"
{
    procedure RaiseInLoop()
    var
        i: Integer;
    begin
        for i := 1 to 10 do
            OnProcessLine();
    end;

    [IntegrationEvent(false, false)]
    procedure OnProcessLine()
    begin
    end;
}
