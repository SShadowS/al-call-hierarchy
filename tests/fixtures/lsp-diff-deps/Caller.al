codeunit 50000 "Caller"
{
    // Variable names match the dependency object's REAL name exactly
    // (quoted, with the space) — DELIBERATELY, not by accident. Legacy's
    // `outgoing_calls` resolves a qualified call by looking up its call
    // site's OWN raw object text directly (`graph.get_external_definition`
    // keyed on `call.callee_object`) — it never re-resolves a differently-
    // named LOCAL VARIABLE to its declared type the way `graph.rs`'s
    // internal `resolve_call` (used for `incoming_calls`) does. A variable
    // named differently from its type (the overwhelmingly common real-world
    // AL style, e.g. `WidgetMgt: Codeunit "Widget Mgt"`) would make legacy
    // fall through to arm 3 ("totally unresolved external call", `data:
    // None`) instead of arm 2 ("external definition found") — a DIFFERENT,
    // real legacy limitation from the one this fixture exists to probe
    // (AbiSymbolShape/DepSourceSpan need arm 2 specifically). Matching the
    // variable name to the real object name sidesteps that unrelated gap so
    // this fixture stays a clean, single-purpose probe.
    procedure CallSymbolOnlyDep()
    var
        "Widget Mgt": Codeunit "Widget Mgt";
    begin
        "Widget Mgt".Compute(5);
    end;

    procedure CallEmbeddedSourceDep()
    var
        "Source Mgt": Codeunit "Source Mgt";
    begin
        "Source Mgt".DoWork(3);
    end;
}
