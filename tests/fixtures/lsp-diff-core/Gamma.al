codeunit 50201 "Gamma" implements "IShape"
{
    procedure Area(): Decimal
    begin
        exit(0);
    end;

    // Unqualified same-object calls: `callee()` (lower-case call text) targets
    // the `Callee` procedure declared below (different case — H-11 CaseFoldHit
    // probe for `incoming(Callee)`), and `Message(...)` is a bareword global
    // builtin call (UnqualifiedCallPlaceholder probe for `outgoing(Caller)`:
    // legacy's `outgoing_calls` renders EVERY unqualified call — resolved
    // local target or not — via its unconditional "(local)" placeholder arm,
    // never actually calling `get_definition`).
    procedure Caller()
    begin
        callee();
        Message('hi');
    end;

    procedure Callee()
    begin
    end;
}
