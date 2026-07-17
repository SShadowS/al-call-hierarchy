codeunit 50100 "Løbenr Mgt."
{
    // Danish Ø/ø: the codeunit's own display name. `CallCrossCaseObjectName`
    // (UnicodeFoldCaller.Codeunit.al) declares a variable typed `Codeunit
    // "LØBENR MGT."` — same letters, ALL-CAPS, including the non-ASCII `Ø` —
    // which must resolve to THIS object under a Unicode-aware fold (the two
    // spellings' `to_ascii_lowercase()` forms differ byte-for-byte: `Ø`
    // U+00D8 stays untouched by an ASCII-only fold, `ø` U+00F8 also stays
    // untouched, and the two are different codepoints).
    procedure Beregn()
    begin
    end;

    // German Ü/ü: `CallCrossCaseMemberName` calls this procedure spelled
    // `PRÜFUNG()` (all-caps, non-ASCII `Ü`) against a receiver declared with
    // this object's OWN exact-case name — isolating the MEMBER/routine-name
    // fold axis from the object-name axis above.
    procedure Prüfung()
    begin
    end;
}
