codeunit 50204 "Zeta"
{
    // Two qualified (real, resolved) calls from the SAME caller to the SAME
    // target: legacy's `incoming(Delta.OnThingHappened)` emits two ungrouped
    // entries (one per call site); the new engine groups them into one entry
    // with two `fromRanges` — OutgoingCardinality (same underlying
    // caller/target/ranges-multiset, differently grouped item counts).
    procedure CallTwice()
    var
        Delta: Codeunit "Delta";
    begin
        Delta.OnThingHappened();
        Delta.OnThingHappened();
    end;
}
