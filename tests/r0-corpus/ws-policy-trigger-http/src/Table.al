table 50103 "Outbound Event"
{
    fields { field(1; "No."; Integer) { } }
    keys { key(PK; "No.") { Clustered = true; } }

    trigger OnInsert()
    var
        Client: HttpClient;
        Response: HttpResponseMessage;
    begin
        Client.Get('https://example.com/notify', Response);
    end;
}
