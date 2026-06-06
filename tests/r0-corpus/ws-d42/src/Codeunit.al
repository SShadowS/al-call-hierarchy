codeunit 50430 "D42 Demo"
{
    // FLAGGED: caller narrowed load to "No.", Name and forwards to a helper that
    // reads Address — the runtime issues an extra round-trip to fetch Address.
    procedure NarrowedThenForwardToReader()
    var
        Customer: Record Customer;
    begin
        Customer.SetLoadFields("No.", Name);
        Customer.FindFirst();
        ReadsAddress(Customer);
    end;

    // NOT FLAGGED: helper loads its own fields before reading them, so the callee's
    // requiredLoadedFieldsAtEntry is empty — calleeRequiresNone skip.
    procedure NarrowedThenForwardToSelfLoader()
    var
        Customer: Record Customer;
    begin
        Customer.SetLoadFields("No.", Name);
        Customer.FindFirst();
        SelfLoadsAddress(Customer);
    end;

    // NOT FLAGGED: caller didn't narrow (no SetLoadFields/AddLoadFields), so the
    // forwarded record carries the full load — callerFull skip.
    procedure FullThenForward()
    var
        Customer: Record Customer;
    begin
        Customer.FindFirst();
        ReadsAddress(Customer);
    end;

    // NOT FLAGGED: caller's narrow already covers the field the callee reads.
    procedure NarrowCoversCalleeRequirement()
    var
        Customer: Record Customer;
    begin
        Customer.SetLoadFields("No.", Name, Address);
        Customer.FindFirst();
        ReadsAddress(Customer);
    end;

    local procedure ReadsAddress(var Cust: Record Customer)
    begin
        Message(Cust.Address);
    end;

    local procedure SelfLoadsAddress(var Cust: Record Customer)
    begin
        Cust.SetLoadFields(Address);
        Cust.FindFirst();
        Message(Cust.Address);
    end;
}
