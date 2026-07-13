// LegacyIdentityCollapse DIAGNOSTICS-axis probe, shape 2 (CDO re-run
// finding, Finding A): a same-FILE, same-OBJECT overload set (two
// procedures sharing the bare name "Compute", differing only in arg
// count) where only ONE overload is ever called (see
// OverloadProbeCaller.al). Legacy's `definitions` map is keyed by bare
// (object, procedure) NAME TEXT ONLY — no signature component — so both
// overloads collapse into ONE slot, and the 0-arg overload's real caller
// makes legacy think BOTH are "used"; new's `RoutineNodeId` (incl.
// `params_count`/`sig_fp`) keeps them distinct and correctly flags the
// 1-arg overload as unused. Mirrors real CDO source: `Table 6175301 "CDO
// File"`.`SetBackgroundPDF`'s two overloads (1-arg at line 261, 2-arg at
// line 266), one of which is genuinely uncalled.
table 50313 "Overload Probe Table"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }
    keys
    {
        key(PK; "No.") { }
    }

    procedure Compute(): Integer
    begin
        exit(1);
    end;

    // Genuinely never called by anyone.
    procedure Compute(Extra: Integer): Integer
    begin
        exit(Extra);
    end;
}
