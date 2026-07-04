// T3 (pageext-merge-and-final-residual plan) — the addenda-MANDATORY
// pageextension-merge call-result fixture: an inner call-result arg types
// via `arg_dispatch::type_one_arg`'s Member-Call arm ONLY when Task 1's
// PageExtension merge yields a SINGLE visible route; two visible
// extensions declaring the SAME viable member is a genuine ambiguity
// (aggregate-then-adjudicate, never first-wins) and declines.
codeunit 50152 "PCR Caller"
{
    // POSITIVE: "PCR Base Page" is extended by exactly ONE visible
    // PageExtension ("PCR Ext1") declaring `GetCount(): Integer` — the T1
    // merge yields a single route, so `PageVar.GetCount()` types as
    // `Integer`, exact-matching `P(N: Integer)` and eliminating `P(S: Text)`.
    procedure RunSingleExtension()
    var
        T: Codeunit "PCR Target";
        PageVar: Page "PCR Base Page";
    begin
        T.P(PageVar.GetCount());
    end;

    // NEGATIVE: "PCR Base Page 2" is extended by TWO visible PageExtensions
    // ("PCR Ext2A"/"PCR Ext2B"), BOTH declaring `GetCount(): Integer` — the
    // merge's aggregate-then-adjudicate contract feeds both candidates to
    // the SAME ambiguity machinery (never a first-wins pick), so
    // `resolve_member` yields >1 routes and this call-result arg declines to
    // untyped, degrading the WHOLE outer call to honest `AmbiguousResolved`.
    procedure RunTwoExtensions()
    var
        T: Codeunit "PCR Target";
        PageVar2: Page "PCR Base Page 2";
    begin
        T.P(PageVar2.GetCount());
    end;
}
