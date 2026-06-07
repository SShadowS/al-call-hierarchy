// R2b opaque fixture (member-call branch).
//
// `Helper` is a Codeunit-typed var naming an object NOT in this workspace's source
// (it lives in the declared-but-unfetched dependency "UnfetchedDep"). When
// `hasUnfetchedDeclaredDependency` is TRUE (the declared dep's appGuid is absent
// from index.apps), the missing member object is classified `opaque` (the method
// MIGHT live in the unfetched dep) rather than `external-target`.
//
// NOTE: in the bare `indexWorkspace → resolveModel` capture path,
// `index.identity.primaryDependencies` is EMPTY (it is only populated later in
// analyzeWorkspace), so this fixture would resolve to `external-target` there. The
// vector generator sets `primaryDependencies` before resolveModel to exercise the
// member-opaque branch deterministically.
codeunit 50300 OpaqueCaller
{
    var
        Helper: Codeunit "Opaque Dep Helper";

    procedure DoWork()
    begin
        Helper.RunIt();
    end;
}
