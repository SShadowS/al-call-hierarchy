// VariableReceiverResolved probe (CDO layer 3 + layer 4): a table with
// procedures called from a CODEUNIT via a `var` PARAMETER receiver and a
// LOCAL variable receiver respectively (see VariableReceiverCaller.al), PLUS
// (layer 4) two SAME-OBJECT counterexamples that proved the layer-3
// predicate's `caller object != callee object` restriction was wrong: a
// `var`-parameter receiver of the table's OWN type (`MergeWithSelf`/
// `Other`, mirroring CDO `Table 6175301 "CDO File"`'s `MergeWithPdf`/
// `PDFDocument.IsPdf()`), and an implicit `Rec.`-qualified call to another
// procedure on the SAME table (`GetPlainText`/`Rec.IsPdf()`, mirroring CDO
// `Table 6175330 "CDO Payment Link Template"`'s `GetPlainText`/
// `Rec.GetHTML()`). Both are same-object, so the OLD cross-object-only
// predicate never claimed them — legacy still fails to resolve either
// (variable_bindings never binds parameters; `Rec` is never a declared
// local var either) even though caller and callee share an object.
table 50315 "Variable Receiver Table"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }
    keys
    {
        key(PK; "No.") { }
    }

    procedure IsPdf(): Boolean
    begin
        exit(true);
    end;

    procedure IsPasswordProtected(): Boolean
    begin
        exit(false);
    end;

    // Same-object `var` PARAMETER receiver of the table's OWN type.
    procedure MergeWithSelf(var Other: Record "Variable Receiver Table")
    begin
        Other.IsPasswordProtected();
    end;

    // Same-object implicit `Rec.`-qualified call — `Rec` is never a
    // declared local variable, so legacy's `lookup_variable_type` can't
    // type it even though the target is on the SAME table.
    procedure GetPlainText(): Boolean
    begin
        exit(Rec.IsPdf());
    end;
}
