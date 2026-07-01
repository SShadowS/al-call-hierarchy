// (e) Qualified-intrinsic bypass: this codeunit declares its OWN bare
// `CreateGuid()` (shadows the global `createguid` intrinsic on the bare call),
// but the FULLY-QUALIFIED `System.CreateGuid()` call must still bind to the
// `System::createguid` Catalog entry — a qualified platform call is dispatched
// via the `System` Framework-singleton receiver, which never consults source
// candidates (structurally distinct from the bare-call path), so the local
// declaration does NOT shadow it.
codeunit 50955 "ShadowCallerE"
{
    procedure CallE()
    var
        G: Guid;
    begin
        G := CreateGuid();
        G := System.CreateGuid();
    end;

    procedure CreateGuid(): Guid
    begin
    end;
}
