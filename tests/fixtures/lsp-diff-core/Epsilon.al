codeunit 50203 "Epsilon"
{
    // Well-formed subscriber: a real EventFlow edge exists. Both engines
    // must agree this is NOT unused (MATCH, not a divergence — confirms the
    // two engines' unused-procedure R2 mechanisms agree on the well-formed
    // case even though they arrive at it differently: legacy via a blanket
    // `[EventSubscriber]`-attribute exclusion, new via a real resolved edge).
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Delta", 'OnThingHappened', '', false, false)]
    local procedure Handle()
    begin
    end;

    // Misdirected subscriber: 'NoSuchEvent' is never declared anywhere, so no
    // EventFlow edge resolves to it. Legacy's blanket attribute exclusion
    // still hides this from `unused-procedure` (it never checks whether the
    // subscription actually resolves); the new engine correctly flags it —
    // R2Precision.
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Delta", 'NoSuchEvent', '', false, false)]
    local procedure Misdirected()
    begin
    end;
}
