// ⟨task-4-review.md finding M-2⟩ Two sibling member-trigger bodies of ONE object
// (action(Alpha) / action(Bravo), each declaring `trigger OnAction()`) — the shape
// Task 4's stable-id member discriminator exists for. Before Task 4, both OnAction
// bodies folded to ONE stableRoutineId (their findings shared a fingerprint); after,
// each carries a DISTINCT stableRoutineId. No fixture in the corpus carried this
// shape, so nothing in a byte-compared golden proved the discriminator end to end —
// see `cli_b_snapshot_differential.rs`'s `SNAPSHOT_CORPUS` for the golden coverage
// this fixture was added for.
page 50920 "M2 Sibling Triggers"
{
    PageType = Card;

    actions
    {
        area(Processing)
        {
            action(Alpha)
            {
                trigger OnAction()
                begin
                end;
            }
            action(Bravo)
            {
                trigger OnAction()
                begin
                end;
            }
        }
    }
}
