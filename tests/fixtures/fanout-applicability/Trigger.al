// 1B.3b Task 2: ApplicTriggerCaller.Go() inserts into a `Record "ApplicTable"`
// variable, firing ApplicTable's OnInsert trigger — a real
// DispatchShape::Multicast EdgeKind::ImplicitTrigger edge.
// route_applicability's ported implicit_trigger_route_applicable teeth must
// find this route applicable: implicit_trigger_violations stays 0.
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
    procedure Go()
    var
        MyRec: Record "ApplicTable";
    begin
        MyRec.Insert();
    end;
}
