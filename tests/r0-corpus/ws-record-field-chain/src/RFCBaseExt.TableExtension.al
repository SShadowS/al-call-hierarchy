// TableExtension fields — fold into "RFC Base"'s field scope via
// `ResolveIndex::field_in_table` (mirrors the established
// `ResolveIndex::table_extensions_of` routine-folding pattern, Task 3).
tableextension 51502 "RFC Base Ext" extends "RFC Base"
{
    fields
    {
        // (c) POSITIVE: an extension-declared field resolves through the
        // SAME record-field chain arm as a base field (folding).
        field(51500; "Ext Blob"; Blob) { }

        // (g) NEGATIVE (paired with RFCBase's own "Dup Field"): same field
        // NAME as the base table's field 5, different type — a genuine
        // base+extension name collision. Real AL would reject this at
        // compile time (duplicate field name); this engine has no compiler
        // in its execution environment (see PROOF.md) and tree-sitter does
        // not validate cross-object semantic uniqueness, so this fixture
        // exercises OUR OWN fail-closed duplicate-decline logic directly —
        // `field_in_table` must decline rather than pick either candidate.
        field(51501; "Dup Field"; Text[50]) { }
    }
}
