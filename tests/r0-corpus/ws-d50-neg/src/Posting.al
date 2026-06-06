// NEGATIVE: PostDocument calls RunWorkerUnchecked, which does an UNCHECKED
// Codeunit.Run (resultConsumed = false → no checked-run-implicit seed → no D50).
codeunit 50210 "D50 Neg PostingManager"
{
    procedure PostDocument()
    var
        Hdr: Record "D50N Header";
        Line: Record "D50N Line";
        Entry: Record "D50N Entry";
    begin
        Hdr.Get(10000);
        Hdr.Status := 1;
        Hdr.Modify();
        Line.SetRange("Doc No.", Hdr."No.");
        Line.ModifyAll(Status, 1);
        Entry.Init();
        Entry.Insert();
        RunWorkerUnchecked();
    end;

    local procedure RunWorkerUnchecked()
    begin
        // UNCHECKED Run — result not consumed → no §B implicit-commit seed
        Codeunit.Run(Codeunit::"D50N Worker");
    end;
}

codeunit 50211 "D50N Worker"
{
    trigger OnRun()
    begin
        // intentionally empty
    end;
}

table 50210 "D50N Header"
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; Status; Integer) { }
    }
}

table 50211 "D50N Line"
{
    fields
    {
        field(1; "Doc No."; Code[20]) { }
        field(2; Status; Integer) { }
    }
}

table 50212 "D50N Entry"
{
    fields
    {
        field(1; "Entry No."; Integer) { }
    }
}
