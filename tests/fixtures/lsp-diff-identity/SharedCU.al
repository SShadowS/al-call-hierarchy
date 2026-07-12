// LegacyIdentityCollapse probe, shape 1: a CODEUNIT and a PAGE (see
// SharedPage.al) sharing the IDENTICAL display name "Shared Name", each
// declaring their OWN "GetRecipients" procedure. Legacy's
// `object_types`/`definitions` maps are keyed by bare NAME TEXT only — no
// object KIND component at all — so these two, entirely different objects
// collide into ONE slot (last-write-wins).
codeunit 50303 "Shared Name"
{
    procedure GetRecipients(): Text
    begin
        exit('from codeunit');
    end;
}
