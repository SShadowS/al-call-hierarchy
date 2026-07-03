// (e) NEGATIVE: a quoted-field-shaped bare receiver in a NON-Table/
// TableExtension object — a Codeunit has no implicit-Rec FIELD surface
// reachable this way (Step 3a's `ObjectKind` guard), even though "File
// Blob" happens to be a real field name on "RBF Base" elsewhere in this
// same app. Proves the OBJECT-KIND gate, not merely "no such field".
codeunit 51522 "RBF Caller"
{
    procedure TestBareFieldReceiverNonTableScope()
    var
        S: InStream;
    begin
        "File Blob".CreateInStream(S);
    end;
}
