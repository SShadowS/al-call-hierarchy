// (b) POSITIVE: the SAME bare implicit-Rec quoted-field shape, inside a
// TableExtension's own procedure — proves `ResolveIndex::field_in_table`'s
// base+own-extension folding applies identically to the bare (Task 4) arm,
// not only the explicit `Rec."Field"` (Task 3) arm.
tableextension 51521 "RBF Base Ext" extends "RBF Base"
{
    fields
    {
        field(51520; "Ext Blob"; Blob) { }
    }

    // (b) own-extension field, bare implicit-Rec.
    procedure TestBareOwnExtField()
    var
        S: InStream;
    begin
        "Ext Blob".CreateInStream(S);
    end;

    // (b) BASE table's field, folded into this extension's own scope.
    procedure TestBareBaseFieldFromExtension()
    var
        S: InStream;
    begin
        "File Blob".CreateInStream(S);
    end;
}
