// plan v2.1 Task 3 fixture — (N1) polymorphic-prefix NEGATIVE control: TWO
// implementers in the closure, so `resolve_member`'s Interface fan-out
// yields 2 routes — the route-count guard must decline (conservative,
// never a guessed pick).
interface ICCBar
{
    procedure GetHelper(): Codeunit "CC Helper";
}
