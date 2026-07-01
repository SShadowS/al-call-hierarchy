// Case (b) CONTROL: `ExtA extends Base`, bare-calling `Pub()` — `Public` is
// always visible, so Step 2 must still resolve this to `Source`, UNCHANGED
// by the Task-1.5 access filter.
//
// AL semantics: compiles.
//
// Fresh-engine route: Evidence::Source (unchanged pre/post-fix).
tableextension 52903 "ExtA" extends Base
{
}
