// Case (a): `ExtA extends Base`, bare-calling `L()` — `resolve_bare`'s Step 2
// ("extension base") resolves a bare call against the caller's extended BASE
// object via `resolve_in_object`, which (pre-Task-1.5) did ZERO access
// filtering. `L` is `local` on `Base`, so it is NOT part of `Base`'s visible
// surface from ANY extension, including this direct one.
//
// AL semantics: DOES NOT COMPILE from `ExtA` (access error — `L` is not
// visible outside `Base` itself).
//
// Fresh-engine route: Evidence::Unknown (post-Task-1.5; pre-fix: false
// Evidence::Source targeting Base.L — verified via TDD, see
// `bare_extension_base_local_method_excluded`).
tableextension 52901 "ExtA" extends Base
{
}
