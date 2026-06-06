table 70010 Customer
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; Name; Text[100]) { }
    }
    keys { key(PK; "No.") { } }

    trigger OnInsert()
    begin
    end;

    trigger OnModify()
    begin
    end;
}

// Quoted/spaced object name + a trigger.
table 70011 "Sales Header"
{
    fields
    {
        field(1; "Document No."; Code[20]) { }
    }
    keys { key(PK; "Document No.") { } }

    trigger OnDelete()
    begin
    end;
}

table 70012 "Temp Buffer"
{
    fields
    {
        field(1; "Entry No."; Integer) { }
    }
    keys { key(PK; "Entry No.") { } }
}

enum 70013 "Doc Kind"
{
    Extensible = true;
    value(0; Quote) { }
    value(1; "Blanket Order") { }
}
