// App: SoloFriendApp (no dependencies, no friends declared). Same-app
// `internal` is unaffected by friend modeling -- it was already visible via
// the pre-existing same-app check.
codeunit 53977 "SameAppTarget"
{
    internal procedure DoWork()
    begin
    end;
}
