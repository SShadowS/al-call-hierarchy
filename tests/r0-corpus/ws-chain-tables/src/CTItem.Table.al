// Task 4 fixture — target table for the "same-named member on a non-RecordRef-
// family receiver" negative (N5). `FieldIndex` is a real `RecordRef`/`KeyRef`
// chain-table member name, but a plain `Record` receiver must never engage the
// `recordref_family_return_kind` table (it dispatches only on `ReceiverType::
// {RecordRef, FieldRef, KeyRef}` — `Record{..}` is a different variant
// entirely) — mirrors `ws-compound-framework/src/CFCustomer.Table.al`'s
// "same-named member on a non-framework receiver" precedent.
table 51200 "CT Item"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }
    keys
    {
        key(PK; "No.") { Clustered = true; }
    }
}
