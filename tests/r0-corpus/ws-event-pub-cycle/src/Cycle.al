// ws-event-pub-cycle: fixture for testing cycle detection in event chain walks.
//
// Setup: A → publishes OnA → B subscribes; B → publishes OnB → A subscribes.
// A.OnA is the publisher of E1; A.HandlerB subscribes to E2.
// B.OnB is the publisher of E2; B.HandlerA subscribes to E1.
//
// In the event-graph model (walkEventChain only follows event-graph edges,
// not call-graph relays), A.OnA and B.OnB are the publisher routineIds.
// A.HandlerB and B.HandlerA are subscriber routineIds.
// These are DIFFERENT routine IDs, so no cycle fires via walkEventChain.
// The cycle is a call-graph cycle, not an event-graph cycle.
//
// Cycle detection (cycleDetected:true) in walkEventChain only fires when a
// subscriber routine ID equals a publisher routine ID already on the walk
// path. That requires the same AL procedure to be both publisher and
// subscriber, which is not expressible in a single kind= assignment via the
// current routine-indexer. This fixture documents the attempt.
codeunit 50000 CycleA
{
    procedure FireA()
    begin
        OnA();
    end;

    [IntegrationEvent(false, false)]
    procedure OnA()
    begin
    end;

    [EventSubscriber(ObjectType::Codeunit, Codeunit::CycleB, 'OnB', '', false, false)]
    local procedure HandlerB()
    begin
        FireA();
    end;
}
