// 1B.3b Task 2: ApplicTriggerCaller.Go() inserts into a `Record "ApplicTable"`
// variable, firing ApplicTable's OnInsert trigger — a real
// DispatchShape::Multicast EdgeKind::ImplicitTrigger edge.
// route_applicability's ported implicit_trigger_route_applicable teeth must
// find this route applicable: implicit_trigger_violations stays 0.
//
// Task 2 review fix: `Go` is deliberately PARAMETERIZED — see Interface.al's
// header comment for why a zero-arity-only fixture can't catch a
// `build_fan_out_site_context` `RoutineNodeId`/`sig_fp` construction
// mismatch.
table 50704 "ApplicTable"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }

    trigger OnInsert()
    begin
    end;
}

codeunit 50705 "ApplicTriggerCaller"
{
    procedure Go(Dummy: Integer)
    var
        MyRec: Record "ApplicTable";
    begin
        MyRec.Insert();
    end;
}
