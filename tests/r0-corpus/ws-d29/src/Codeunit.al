codeunit 51300 "D29 Demo"
{
    // FLAGGED: subscriber to OnAfterModifyEvent calls Modify on the inbound Rec.
    [EventSubscriber(ObjectType::Table, Database::Customer, 'OnAfterModifyEvent', '', false, false)]
    local procedure OnAfterModifyMutates(var Rec: Record Customer; var xRec: Record Customer; RunTrigger: Boolean)
    begin
        Rec.Modify();
    end;

    // FLAGGED: same shape via Delete.
    [EventSubscriber(ObjectType::Table, Database::Customer, 'OnBeforeDeleteEvent', '', false, false)]
    local procedure OnBeforeDeleteMutates(var Rec: Record Customer; RunTrigger: Boolean)
    begin
        Rec.Delete();
    end;

    // NOT FLAGGED: subscriber doesn't mutate the inbound record.
    [EventSubscriber(ObjectType::Table, Database::Customer, 'OnAfterModifyEvent', '', false, false)]
    local procedure OnAfterModifyReadOnly(var Rec: Record Customer; var xRec: Record Customer; RunTrigger: Boolean)
    var
        Other: Record Customer;
    begin
        Other.Get(Rec."No.");
    end;

    // NOT FLAGGED: subscribed event is not a Modify/Delete event.
    [EventSubscriber(ObjectType::Codeunit, 50, 'OnAfterPost', '', false, false)]
    local procedure OnAfterPostMutates(var Rec: Record Customer)
    begin
        Rec.Modify();
    end;
}
