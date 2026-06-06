// POSITIVE: PostDocument (matches POSTING_NAME_RE) calls RunWorkerChecked,
// which does a checked Codeunit.Run. The checked-run-implicit span walks back
// from RunWorkerChecked and includes PostDocument → D50 fires.
codeunit 50200 "D50 PostingManager"
{
    procedure PostDocument()
    var
        Hdr: Record "D50 Header";
        Line: Record "D50 Line";
        Entry: Record "D50 Entry";
    begin
        Hdr.Get(10000);
        Hdr.Status := 1;
        Hdr.Modify();
        Line.SetRange("Doc No.", Hdr."No.");
        Line.ModifyAll(Status, 1);
        Entry.Init();
        Entry.Insert();
        RunWorkerChecked();
    end;

    local procedure RunWorkerChecked()
    begin
        // checked Run → implicit commit on success → §B seed
        if Codeunit.Run(Codeunit::"D50 Worker") then;
    end;
}

codeunit 50201 "D50 Worker"
{
    trigger OnRun()
    begin
        // intentionally empty
    end;
}

table 50200 "D50 Header"
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; Status; Integer) { }
    }
}

table 50201 "D50 Line"
{
    fields
    {
        field(1; "Doc No."; Code[20]) { }
        field(2; Status; Integer) { }
    }
}

table 50202 "D50 Entry"
{
    fields
    {
        field(1; "Entry No."; Integer) { }
    }
}
