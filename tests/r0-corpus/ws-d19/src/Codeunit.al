codeunit 50500 "D19 Demo"
{
    // FLAGGED: record-typed `UnusedCustomer` declared but never referenced.
    procedure WithUnusedRecord(Customer: Record Customer; UnusedCustomer: Record Customer)
    begin
        Customer.FindFirst();
    end;

    // NOT FLAGGED: every record param is referenced.
    procedure AllUsed(Customer: Record Customer; Other: Record Customer)
    begin
        Other.SetRange("No.", 'X');
        Customer.FindFirst();
    end;

    // FLAGGED (post-PR-5): scalar `ScalarUnused` declared but never referenced.
    // The structural `identifierReferences` set lets D19 see scalar uses too.
    procedure WithUnusedScalar(Customer: Record Customer; ScalarUnused: Integer)
    begin
        Customer.FindFirst();
    end;

    // NOT FLAGGED: event-subscriber signatures must keep all params (dictated by publisher).
    [EventSubscriber(ObjectType::Codeunit, 50, 'OnFoo', '', false, false)]
    local procedure OnFooSubscriber(Sender: Codeunit "D19 Demo"; UnusedRec: Record Customer)
    begin
    end;
}
