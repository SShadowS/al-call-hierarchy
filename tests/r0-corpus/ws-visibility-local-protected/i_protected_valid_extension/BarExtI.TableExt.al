// Case (i) valid extension → base protected (TableExtension sub-case):
// `BarExtI` DIRECTLY extends `Bar` and calls `Bar`'s `protected procedure`.
//
// AL semantics: legal — `protected` is visible to the declaring object AND
// its extensions. Expected: COMPILES. Fresh-engine expected route:
// Evidence::Source, target = Bar.P.
//
// See `resolve_member_record_tableext_protected_base_resolves_to_source`.
tableextension 52671 "BarExtI" extends Bar
{
    procedure Wrapper()
    var
        R: Record Bar;
    begin
        R.P();
    end;
}
