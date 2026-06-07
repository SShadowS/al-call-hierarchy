// R2b external-target fixture (member-call branch).
//
// `Helper` is a Codeunit-typed var naming an object NOT in this workspace's source,
// and this app declares NO dependencies — so `hasUnfetchedDeclaredDependency` is
// FALSE. With every declared dep fetched (here: none) and the object still missing,
// the missing member object is classified `external-target` (genuinely not in any
// known dep), carrying its `externalTypeRef` { kind: "Codeunit", name: "External Dep Helper" }.
codeunit 50350 ExternalCaller
{
    var
        Helper: Codeunit "External Dep Helper";

    procedure DoWork()
    begin
        Helper.RunIt();
    end;
}
