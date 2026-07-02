// beyond-1B.3b Task 4 fixture — target table for the "same-named member on a
// non-framework type" negative (fixture i) and the record-field fixture (j).
// `BlobField` deliberately shares a name with the framework-conversion
// vocabulary used elsewhere in this fixture set (mirrors `Content`/
// `AsObject`-style member names) but is a plain Blob FIELD, not a framework
// method/property — `framework_return_kind`'s table lookup must never see
// it, since the receiver base types `Record`, not `Framework`. Post record-
// field-chains-plan Task 3, `Rec.BlobField` DOES resolve — via the SEPARATE
// `ResolveIndex::field_in_table` mechanism, not this Framework table (see
// fixture j's doc in `CFCaller.Codeunit.al`).
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
