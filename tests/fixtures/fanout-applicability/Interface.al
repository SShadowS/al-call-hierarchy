// 1B.3b Task 2: drives a real DispatchShape::Polymorphic EdgeKind::Call edge
// through resolve_full_program (NOT hand-constructed) — ApplicFooImpl
// implements "IApplicFoo" and is dispatched via an `Interface "IApplicFoo"`
// -typed variable. route_applicability's ported interface_route_applicable
// teeth must find this route applicable: interface_applicability_violations
// stays 0.
//
// Task 2 review fix: the caller `Go` is deliberately PARAMETERIZED
// (`params_count >= 1`), not zero-arity. `build_fan_out_site_context` keys
// its context map by `SiteId { caller: RoutineNodeId, .. }`, and
// `RoutineNodeId::sig_fp` participates in `Eq`/`Hash`. A zero-param routine's
// `source_param_sig_fp` is always 0 regardless of construction path, so a
// zero-param-only fixture cannot distinguish a correctly-computed real
// `sig_fp` from a hardcoded `0` — exactly the gap that let
// `build_fan_out_site_context`'s missed 6th-site migration ship silently
// (see CHANGELOG "Task 2 review fix"). A parameterized caller forces the two
// `RoutineNodeId`s (the one on `Edge::site` from `resolve_full_program`, and
// the one `build_fan_out_site_context` re-derives) to actually agree on a
// non-zero `sig_fp` for the map lookup to hit.
codeunit 50700 "ApplicFooImpl" implements "IApplicFoo"
{
    procedure Bar()
    begin
    end;
}

codeunit 50701 "ApplicIfaceCaller"
{
    procedure Go(Dummy: Integer)
    var
        MyIFoo: Interface "IApplicFoo";
    begin
        MyIFoo.Bar();
    end;
}
