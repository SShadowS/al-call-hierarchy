table 50121 "MCD Rec"
{
    fields { field(1; "Entry No."; Integer) { } }
    keys { key(PK; "Entry No.") { Clustered = true; } }

    procedure ToBase64String(): Text
    begin
        exit('');
    end;
}
