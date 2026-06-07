// Resolution-heavy record-type fixture for Phase R2a.
//
// Exercises the L3 record-type surface IN A SINGLE SOURCE-ONLY WORKSPACE:
//   - a base Table whose implicit Rec resolves to itself (Table trigger)
//   - a TableExtension extending a base table THAT IS IN THIS WORKSPACE, so
//     mergeExtensionFields actually fires (the source-only corpus otherwise
//     extends base-app tables that are absent → no merge)
//   - a quoted-identifier base table name ("Sales Line") for quoted-parity
//   - a COLLISION: two TableExtensions adding the SAME field number to the same
//     base table → mergeExtensionFields is FIRST-wins (the second is dropped).

table 50900 Customer
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; Name; Text[100]) { }
    }
    keys { key(PK; "No.") { } }

    trigger OnInsert()
    begin
        // implicit Rec → resolves to THIS table (50900).
        Rec.Modify();
        Modify();
    end;
}

table 50901 "Sales Line"
{
    fields
    {
        field(1; "Document No."; Code[20]) { }
        field(2; "Line No."; Integer) { }
    }
    keys { key(PK; "Document No.", "Line No.") { } }
}

// TableExtension extending an IN-WORKSPACE base table → fields merge into 50900.
tableextension 50910 "Customer Ext A" extends Customer
{
    fields
    {
        field(50000; "Loyalty Points"; Integer) { }
        field(50001; "Tier"; Text[20]) { }
    }
}

// COLLISION: this extension re-adds field 50000 to the SAME base table. The merge
// dedups by fieldNumber FIRST-wins, so 50000 keeps "Loyalty Points" (from Ext A,
// which is ingested first in file/object order) — "Collides 50000" is DROPPED.
// Field 50002 is new and DOES merge.
tableextension 50911 "Customer Ext B" extends Customer
{
    fields
    {
        field(50000; "Collides 50000"; Integer) { }
        field(50002; "Segment"; Code[10]) { }
    }
}

// Quoted-identifier base table: extends "Sales Line" (50901) → fields merge into it.
tableextension 50912 "Sales Line Ext" extends "Sales Line"
{
    fields
    {
        field(50100; "Custom Note"; Text[50]) { }
    }
}
