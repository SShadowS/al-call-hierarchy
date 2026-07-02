// Task 1 fixture (e): interface with a SymbolOnly (dep) implementer whose
// declaring procedure is `protected` (Dep IfaceImpl.DoIt, in the probe
// ProtAbiDep app) AND a workspace implementer whose declaring procedure is
// `public` (IfaceImplWs.DoIt). The polymorphic fan-out over both implementers
// must apply PER-CANDIDATE visibility independently: the dep route declines
// (ProtectedNotVisible), the workspace route resolves.
interface IProtWorker
{
    procedure DoIt();
}
