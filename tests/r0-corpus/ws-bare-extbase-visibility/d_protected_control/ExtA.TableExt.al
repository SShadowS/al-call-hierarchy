// Case (d) CONTROL: `ExtA extends Base`, bare-calling `P()` — `protected` is
// visible to the declaring object AND its extensions; Step 2's caller is BY
// CONSTRUCTION a direct extension of the base it is probing, so the
// self-or-extends check trivially holds. Confirms this incidentally-safe
// path stays correct after the access filter is added.
//
// AL semantics: compiles.
//
// Fresh-engine route: Evidence::Source (unchanged pre/post-fix).
tableextension 52907 "ExtA" extends Base
{
}
