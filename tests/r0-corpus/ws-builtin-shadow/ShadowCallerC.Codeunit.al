// (c) Genuine-builtin regression: NO source competitor exists anywhere in this
// workspace for "fieldcaption" (a Record builtin), "message" (a global
// builtin), or JsonObject's "add" (a framework-member builtin) — all three
// must STAY Catalog after the source-shadows-catalog precedence fix.
//
// NOTE: deliberately NOT "SetRange"/"Insert"/"Modify"/etc. — those 28 names
// are classified as `CalleeShape::RecordOp` (extract.rs `record_op_names`)
// and resolved via the SEPARATE implicit-trigger path, not `resolve_member`'s
// Record arm; "FieldCaption" stays a plain member call so this fixture
// actually exercises the Record-arm catalog fallback.
codeunit 50953 "ShadowCallerC"
{
    procedure CallC()
    var
        R: Record Acme;
        J: JsonObject;
    begin
        R.FieldCaption(1);
        Message('hi');
        J.Add('k', 'v');
    end;
}
