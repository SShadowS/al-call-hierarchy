// Task 6 fixture (opaque-apps empty-ABI exemption): the workspace declares a
// dependency on a symbol-only "Application" umbrella app whose
// SymbolReference.json has ZERO objects (mirrors Microsoft's real
// Microsoft_Application_*.app, present in ~every BC 24+ workspace). There is
// nothing to call on it -- an empty ABI surface provably hides no bodies, so
// this dep must NOT appear in `FreshCoverage.opaque_apps`.
codeunit 50100 "WS Empty Abi Caller"
{
    procedure Run()
    begin
    end;
}
