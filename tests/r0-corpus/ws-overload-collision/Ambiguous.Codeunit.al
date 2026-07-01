// beyond-1B.3b Task 2 fixture: two SOURCE overloads sharing the same name AND
// arity, differing only by parameter TYPE. `RoutineNodeId` does not carry a
// type-derived `sig_fp` for source-bearing routines (`sig_fp` is always `0`
// for source; see node.rs), so these two DISTINCT declarations collide onto
// one `RoutineNodeId`. No arg-type evidence exists to pick between them
// (full arg-type dispatch is explicitly out of scope for this task) — a
// caller invoking `Resolve(...)` at this arity must NOT get a confident
// pick-first `Source` route to whichever overload happened to survive.
codeunit 50960 "Overload Collision Target"
{
    procedure Resolve(X: Integer)
    begin
        Message('int');
    end;

    procedure Resolve(X: Code[20])
    begin
        Message('code');
    end;
}
