// beyond-1B.3b Task 3 fixture (e), NEGATIVE — Rec/builtin-shadow: bare
// `Update()` used as a compound-receiver PREFIX (`Update().Bar()`) collides
// between the implicit-Rec table's own `Update` procedure ("CR Customer",
// non-scalar `Record "CR Customer"` return — see `CRCustomer.Table.al`) and
// the bare-callable `PageInstance` intrinsic `Update`. The prefix's
// type-query goes through `resolve_bare`, which fails closed to
// `Unresolved{BuiltinPrecedenceCollision}` on this UNPROVEN precedence
// (never assumes the table wins) — so `infer_call_result_receiver` declines
// (no `RouteTarget::Routine`), and `Update().Bar()` stays honest `Unknown`.
page 51004 "CallResultPage"
{
    SourceTable = "CR Customer";

    layout
    {
        area(Content)
        {
        }
    }

    trigger OnOpenPage()
    begin
        Update().Bar();
    end;
}
