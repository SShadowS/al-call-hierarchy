// Dataitem-receivers plan, Task 1 — report-dataitem receivers end to end.
// Real CDO grounding (grepped `CDO_WS`, see the plan): `Report 6175283 "CDO
// Update Output Profile"` declares 16 dataitems in 3 report files, ~29
// quoted-name receiver uses; the measured ~27-site UntrackedReceiver
// subpopulation this task resolves. This fixture mirrors that shape:
// `Cust` (plain name) / `"Sales Cr.Memo Header Filter"` (a real dot-bearing
// dataitem name, the quote-guard fix's own grounding).
report 51700 "RD Base Report"
{
    dataset
    {
        // (a)+(c): a plain-name dataitem. `OnAfterGetRecord` (a real
        // dataitem TRIGGER) proves the routine-contextual implicit-Rec
        // threading (`RoutineDecl.dataitem_source_table`); `TestBareCustName`
        // proves the dataitem-NAME-as-receiver lookup (Step 2b) is
        // routine-independent (a dataitem name is in scope as a record var
        // across ALL the report's routines).
        dataitem(Cust; "RD Customer")
        {
            trigger OnAfterGetRecord()
            begin
                Rec.GetDisplayName();
            end;
        }

        // (b): a QUOTED dataitem name with an EMBEDDED PERIOD — the naive
        // dot-substring quote-guard fix's own real-CDO grounding. No trigger
        // here (a second `OnAfterGetRecord` under a different dataitem in
        // the SAME report is legal AL but not distinguishable by this
        // fixture set's `edges_for_object_routine` test helper, which keys
        // only on `(object, routine name)` — the dot-bearing NAME lookup is
        // tested via `TestBareDotBearingName` below instead, equally
        // exercising the quote-guard fix).
        dataitem("Sales Cr.Memo Header Filter"; "RD Sales Header")
        {
        }

        // (f) NEGATIVE (collision guard): a dataitem name that is ALSO a
        // report procedure name (below) — must decline (fail closed), never
        // guess between "the dataitem record" and "a parens-less call to the
        // procedure".
        dataitem("RD Collide"; "RD Customer")
        {
        }
    }

    requestpage
    {
        // REQUESTPAGE ISOLATION (binding, round-1 addendum): even with a
        // dataitem-bearing dataset above, a requestpage trigger's implicit
        // Rec must NEVER bind a dataitem's table — must stay honest Unknown.
        trigger OnOpenPage()
        begin
            Rec.GetDisplayName();
        end;
    }

    // The colliding procedure for the "RD Collide" dataitem above.
    procedure "RD Collide"()
    begin
    end;

    // (f) NEGATIVE (collision guard, fail-closed): calling through the
    // colliding name must decline — AL's parens are optional on a zero-arg
    // call, so a bare quoted name is structurally ambiguous between "the
    // dataitem record" and "a parens-less call to the same-named procedure".
    procedure TestCollisionDeclines()
    begin
        "RD Collide".GetDisplayName();
    end;

    // NEGATIVE (var shadows dataitem, AL scoping): a LOCAL var named
    // identically to the "Cust" dataitem, of a DIFFERENT table, must win —
    // Step 2 (var lookup) runs strictly before Step 2b (dataitem lookup).
    procedure TestVarShadowsDataitem()
    var
        Cust: Record "RD Sales Header";
    begin
        Cust.GetFilters();
    end;

    // POSITIVE (Step 2b): a bare dataitem-NAME receiver, called from a
    // routine with NO enclosing dataitem context at all — proves the lookup
    // is routine-independent (the dataitem name is in scope report-wide).
    procedure TestBareCustName()
    begin
        Cust.GetDisplayName();
    end;

    // POSITIVE (Step 2b, quoted + embedded period — the naive dot-guard
    // fix): a QUOTED dataitem-NAME receiver with an embedded period, exactly
    // like the real CDO `"Sales Cr.Memo Header Filter"` shape.
    procedure TestBareDotBearingName()
    begin
        "Sales Cr.Memo Header Filter".GetFilters();
    end;

    // NEGATIVE (genuinely compound receiver stays compound): an unquoted
    // `A.B` shaped receiver — even though no such dataitem/var exists, this
    // proves the atomic-token guard never mis-routes a real multi-segment
    // chain into the dataitem-name lookup.
    procedure TestGenuinelyCompoundReceiverStaysUnknown()
    begin
        NoSuchDataitem.NoSuchField.Foo();
    end;
}
