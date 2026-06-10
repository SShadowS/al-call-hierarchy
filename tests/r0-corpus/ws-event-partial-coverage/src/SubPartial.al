codeunit 50001 SubPartial
{
    // Subscriber whose body performs a DYNAMIC Codeunit.Run on a table-field
    // codeunit id. The target cannot be resolved statically, producing an
    // `object-run-unresolved` typed edge → the routine's inherited capability
    // cone is incomplete → coverage.inheritedStatus = "partial". This makes the
    // fanout entry's capabilityComposition dimension report "partial" (glyph ≈).
    [EventSubscriber(ObjectType::Codeunit, Codeunit::PartialPub, 'OnE', '', false, false)]
    local procedure HPartial(var C: Record Customer)
    var
        S: Record "Dispatch Setup";
    begin
        S.Get(C."No.");
        Codeunit.Run(S."Codeunit Id");
    end;
}
