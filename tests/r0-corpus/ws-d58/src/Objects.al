query 50931 "D58 Items"
{
    elements
    {
        dataitem(Item; "D58 Item")
        {
            column(No; "No.") { }
        }
    }
}

codeunit 50932 "D58 Demo"
{
    // FLAGGED: filter applied after Open is ignored by the open dataset.
    procedure FilterAfterOpen()
    var
        Q: Query "D58 Items";
    begin
        Q.Open();
        Q.SetFilter(No, '1000..');
    end;

    // NOT FLAGGED: filter before Open.
    procedure FilterBeforeOpen()
    var
        Q: Query "D58 Items";
    begin
        Q.SetFilter(No, '1000..');
        Q.Open();
    end;

    // NOT FLAGGED: Close re-arms filtering; filter lands before the re-Open.
    procedure CloseThenFilter()
    var
        Q: Query "D58 Items";
    begin
        Q.Open();
        Q.Close();
        Q.SetFilter(No, '1000..');
        Q.Open();
    end;
}

table 50931 "D58 Item"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }
    keys { key(PK; "No.") { } }
}
