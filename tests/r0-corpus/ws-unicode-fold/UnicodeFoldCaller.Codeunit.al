codeunit 50101 "Unicode Fold Caller"
{
    // Axis 1 — OBJECT NAME fold: the type reference spells the target
    // codeunit `"LØBENR MGT."` (all-caps, non-ASCII `Ø`), while
    // LobenrMgt.Codeunit.al declares it `"Løbenr Mgt."` (title case, `ø`).
    // These two spellings are DIFFERENT under `to_ascii_lowercase()` (`Ø`
    // and `ø` both pass through unfolded and are different bytes) but the
    // SAME under a simple 1:1 Unicode fold (both fold to `ø`). The member
    // call itself (`Beregn`) is same-case, isolating this axis.
    procedure CallCrossCaseObjectName()
    var
        LM: Codeunit "LØBENR MGT.";
    begin
        LM.Beregn();
    end;

    // Axis 2 — MEMBER (routine) NAME fold: the receiver is declared with the
    // object's exact-case name (`"Løbenr Mgt."`, same spelling as the
    // declaration), but the call site spells the procedure `PRÜFUNG()`
    // (all-caps, non-ASCII `Ü`) against the declared `Prüfung` (`ü`). Same
    // ASCII-fold-diverges/Unicode-fold-converges shape as axis 1, but on the
    // routine-name fold instead of the object-name fold.
    procedure CallCrossCaseMemberName()
    var
        LM: Codeunit "Løbenr Mgt.";
    begin
        LM.PRÜFUNG();
    end;
}
