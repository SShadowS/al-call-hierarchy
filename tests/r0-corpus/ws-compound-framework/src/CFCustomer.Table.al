// beyond-1B.3b Task 4 fixture — target table for the "same-named member on a
// non-framework type" negative (fixture e) and the DEFERRED record-field
// negative (fixture f). `BlobField` deliberately shares a name with the
// framework-conversion vocabulary used elsewhere in this fixture set (mirrors
// `Content`/`AsObject`-style member names) but is a plain Blob FIELD, not a
// framework method/property — the table lookup must never see it, since the
// receiver base types `Record`, not `Framework`.
table 51100 "CF Customer"
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; "BlobField"; Blob) { }
    }
    keys
    {
        key(PK; "No.") { Clustered = true; }
    }
}
