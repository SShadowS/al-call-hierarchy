table 50101 "MC Customer"
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; Name; Text[100]) { }
    }
    keys { key(PK; "No.") { } }
}

codeunit 50101 "D1 Multi Caller"
{
    // The target — a record-modifying helper. Three distinct ancestor routines below
    // each wrap calls to this helper inside a loop, so all three loops reach the
    // SAME terminal Modify. The detector must collapse these to ONE Finding with
    // the other two paths in additionalPaths.
    procedure ModifyHelper(var Cust: Record "MC Customer")
    begin
        Cust.Modify();
    end;

    procedure CallerA()
    var Cust: Record "MC Customer"; i: Integer;
    begin
        Cust.FindSet();
        for i := 1 to 10 do
            ModifyHelper(Cust);
    end;

    procedure CallerB()
    var Cust: Record "MC Customer"; i: Integer;
    begin
        Cust.FindSet();
        for i := 1 to 5 do
            ModifyHelper(Cust);
    end;

    procedure CallerC()
    var Cust: Record "MC Customer"; i: Integer;
    begin
        Cust.FindSet();
        for i := 1 to 3 do
            ModifyHelper(Cust);
    end;
}
