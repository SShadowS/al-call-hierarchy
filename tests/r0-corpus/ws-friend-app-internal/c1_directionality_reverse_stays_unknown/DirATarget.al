// App: PrimaryAppDirA. Declares "DepAppDirB" a friend via
// <InternalsVisibleTo> -> DirBCaller (in DepAppDirB) may reach DirATarget's
// internal member.
codeunit 53974 "DirATarget"
{
    internal procedure SecretA()
    begin
    end;
}
