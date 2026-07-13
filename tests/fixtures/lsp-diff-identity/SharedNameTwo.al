// LegacyIdentityCollapse DIAGNOSTICS-axis probe, shape 1 (CDO re-run
// finding, Finding A): a CODEUNIT and a PAGE sharing the IDENTICAL display
// name "Shared Name Two", each declaring their OWN "DoSomething" procedure
// — UNLIKE SharedCU.al/SharedPage.al (where BOTH versions are called,
// exercising only the incoming/outgoing/codeLens axes), only the
// CODEUNIT's version is ever called here (see IdentityCallerTwo.al).
// Legacy's collapsed (object, name) slot credits the PAGE's own
// "DoSomething" with the codeunit's real caller too, staying silent on
// the diagnostics axis; new correctly distinguishes the two declarations
// and flags the page's as unused. Mirrors real CDO source: `Page 6175343
// "CDO E-Mail"` vs. `Codeunit 6175280 "CDO E-Mail"`.
codeunit 50310 "Shared Name Two"
{
    procedure DoSomething()
    begin
    end;
}
