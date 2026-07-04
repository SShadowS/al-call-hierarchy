// T3 (pageext-merge-and-final-residual plan) POSITIVE fixture — the
// PrintPDFFile shape (`Page 6175389`'s `PrintPDFFile(DOTempBlob.
// ToBase64String(), PrinterName)`, grounded in the real CDO corpus):
// `T.P(Rec.ToBase64String())` — the arg is a MEMBER-function call-result
// (`Rec.ToBase64String()`), not a bare identifier. `ToBase64String` returns
// `Text`, exact-eliminating the Record-typed sibling overload.
codeunit 50122 "MCD Caller"
{
    procedure Run()
    var
        T: Codeunit "MCD Target";
        Rec: Record "MCD Rec";
    begin
        T.P(Rec.ToBase64String());
    end;
}
