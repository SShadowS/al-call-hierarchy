// App: DepAppDirB, depends on PrimaryAppDirA. DirBCaller -> DirATarget.SecretA()
// resolves Source (A trusts B). A hypothetical reverse call FROM an object in
// PrimaryAppDirA TO DirBTarget.SecretB() would stay Unknown, since
// DepAppDirB names no friends -- friendship is never inferred from the
// reverse direction.
codeunit 53976 "DirBCaller"
{
    procedure Trigger()
    begin
    end;
}
