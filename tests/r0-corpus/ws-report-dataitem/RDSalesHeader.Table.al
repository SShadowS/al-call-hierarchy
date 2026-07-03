// Dataitem-receivers plan, Task 1 — the table backing the DOT-BEARING
// dataitem name fixture. Real CDO grounding: `Report 6175283 "CDO Update
// Output Profile"`, `dataitem("Sales Cr.Memo Header Filter"; "Sales
// Header")`, referenced bare as `"Sales Cr.Memo Header Filter".GetFilters()`.
table 51711 "RD Sales Header"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }

    procedure GetFilters(): Text
    begin
        exit("No.");
    end;
}
