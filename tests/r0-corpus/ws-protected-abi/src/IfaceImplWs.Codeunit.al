// Task 1 fixture (e) support: a WORKSPACE implementer of `IProtWorker` whose
// `DoIt` is `public` — the CONTRASTING sibling to the dep's SymbolOnly
// implementer (`Dep IfaceImpl`, `protected DoIt`), so the polymorphic fan-out
// over `IProtWorker`'s two implementers proves per-candidate visibility: this
// route resolves while the dep's route declines.
codeunit 51003 "IfaceImplWs" implements IProtWorker
{
    procedure DoIt()
    begin
    end;
}
