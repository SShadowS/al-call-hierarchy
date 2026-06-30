// 1B.3b Task 2: drives a real DispatchShape::Polymorphic EdgeKind::Call edge
// through resolve_full_program (NOT hand-constructed) — ApplicFooImpl
// implements "IApplicFoo" and is dispatched via an `Interface "IApplicFoo"`
// -typed variable. route_applicability's ported interface_route_applicable
// teeth must find this route applicable: interface_applicability_violations
// stays 0.
codeunit 50700 "ApplicFooImpl" implements "IApplicFoo"
{
    procedure Bar()
    begin
    end;
}

codeunit 50701 "ApplicIfaceCaller"
{
    procedure Go()
    var
        MyIFoo: Interface "IApplicFoo";
    begin
        MyIFoo.Bar();
    end;
}
