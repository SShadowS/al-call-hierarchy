// The subscriber does BOTH things d2's confidence needs:
//
//   * it touches the database (`FindSet`), which is what makes it count as a
//     db subscriber at all (`any_db_subscriber`); and
//   * it dispatches through an interface, which is what makes its uncertainty
//     set non-empty.
//
// A RESOLVED interface dispatch produces TWO uncertainties at one call site —
// `interface-open-world` (callsite-local, stays on this routine) and
// `opaque-callee` (propagates up the call graph). See
// `ws-d1-uncertain-path/src/handlers.al`, which documents that asymmetry; this
// fixture reuses the mechanism rather than re-deriving it.
//
// The interface call sits AFTER the db op so `FindSet` stays operation 0.
codeunit 64202 "D2U Subscriber"
{
    var
        Alpha: Interface "D2U IAlpha";

    [EventSubscriber(ObjectType::Codeunit, Codeunit::"D2U Publisher", 'OnProcessLine', '', true, true)]
    local procedure HandleProcessLine()
    var
        Customer: Record "D2U Customer";
    begin
        Customer.FindSet();
        Alpha.RunAlpha();
    end;
}
