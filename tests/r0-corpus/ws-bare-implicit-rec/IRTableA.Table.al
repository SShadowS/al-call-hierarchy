// beyond-1B.3b Task 3 fixture (a)/(b) shared base: a table with a NON-builtin
// procedure. Used both as the POSITIVE case (a Page with no own `GetDisplayText`
// resolves the bare call through implicit-Rec to THIS table) and as the
// shadow-guard case (b) (a Page that ALSO declares its own `GetDisplayText` must
// shadow this one via Step 1, never reaching Step 3).
table 50970 "IR Table A"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }

    procedure GetDisplayText(): Text
    begin
        exit("No.");
    end;
}
