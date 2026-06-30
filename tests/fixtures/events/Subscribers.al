// Codeunit 50200 — Manual subscriber (EventSubscriberInstance = Manual).
// Subscribes to OnAfterPost() (0-param).
// Fresh links to the 0-param overload; L3 (last-wins) links to the 1-param
// overload → l3_false_positive_arity_mismatch.
codeunit 50200 "ManualSub"
{
    EventSubscriberInstance = Manual;

    [EventSubscriber(ObjectType::Codeunit, Codeunit::"EventPublisher", 'OnAfterPost', '', false, false)]
    local procedure HandleOnAfterPost()
    begin
    end;
}

// Codeunit 50201 — SkipOnMissingLicense subscriber.
// Subscribes to OnBeforePost() (0-param BusinessEvent).
codeunit 50201 "SkipLicenseSub"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"EventPublisher", 'OnBeforePost', '', true, false)]
    local procedure HandleOnBeforePost()
    begin
    end;
}

// Codeunit 50202 — Multiple [EventSubscriber] handler.
// L3 reads only the FIRST attribute; fresh reads BOTH.
// • OnAfterPost edge — matched on both sides (Stage-1 match).
// • OnBeforePost edge — pair_fresh_only → multiple_attr_l3_gap.
codeunit 50202 "MultiAttrSub"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"EventPublisher", 'OnAfterPost', '', false, false)]
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"EventPublisher", 'OnBeforePost', '', false, false)]
    local procedure HandleBoth()
    begin
    end;
}

// Codeunit 50203 — InternalEvent subscriber.
// L3 classifies InternalEvent publishers as procedure (not event-publisher)
// → resolution = "maybe" or "unknown", NOT "resolved".
// Fresh (is_event_publisher returns Internal) emits an EventFlow edge.
// Result: pair_fresh_only → internal_event_non_shipping.
codeunit 50203 "InternalSub"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"EventPublisher", 'OnInternalEvent', '', false, false)]
    local procedure HandleOnInternalEvent()
    begin
    end;
}
