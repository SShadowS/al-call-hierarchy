// 1B.3b Task 2: ApplicEventSubscriber genuinely subscribes to
// ApplicEventPublisher.OnApplicEvent — a real EdgeKind::EventFlow edge.
// route_applicability's ported verify_event_subscriber_route teeth (an
// independent re-parse of the raw [EventSubscriber] attribute) must find
// this route applicable: event_violations stays 0.
codeunit 50706 "ApplicEventPublisher"
{
    [IntegrationEvent(false, false)]
    procedure OnApplicEvent()
    begin
    end;
}

codeunit 50707 "ApplicEventSubscriber"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"ApplicEventPublisher", 'OnApplicEvent', '', false, false)]
    local procedure HandleApplicEvent()
    begin
    end;
}
