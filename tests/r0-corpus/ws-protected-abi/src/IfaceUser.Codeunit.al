// Task 1 fixture (e): a caller holding an `Interface IProtWorker` variable —
// `resolve_member`'s `Interface` arm fans out POLYMORPHICALLY to every known
// implementer (`Dep IfaceImpl`, SymbolOnly + protected; `IfaceImplWs`,
// source + public). Both routes must apply the SAME per-candidate visibility
// discipline `resolve_in_object` now enforces uniformly across tiers.
codeunit 51004 "IfaceUser"
{
    var
        Worker: Interface IProtWorker;

    procedure TestInterfaceFanOut()
    begin
        Worker.DoIt();
    end;
}
