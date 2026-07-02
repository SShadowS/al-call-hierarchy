// plan v2.1 Task 3 fixture — (d) single-implementer interface prefix SUCCESS
// control: exactly ONE implementer (`CC Foo Impl`) in the closure, so
// `resolve_member`'s Interface fan-out yields exactly 1 route — the
// route-count guard accepts, and the chain types by the interface's own
// declared signature (preferred when modeled — see
// `interface_own_routine_node`).
interface ICCFoo
{
    procedure GetHelper(): Codeunit "CC Helper";
}
