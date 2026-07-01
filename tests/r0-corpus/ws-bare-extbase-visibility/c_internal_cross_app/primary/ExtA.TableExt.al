// Case (c): `ExtA extends Base` lives in the PRIMARY app (AppA, depends on
// AppB). `ExtA` bare-calls `I()`, an `internal` procedure declared on
// `Base` (AppB) — cross-app `internal` is not visible outside its declaring
// app (`InternalsVisibleTo`/friend-app is out of scope, documented; fails
// closed to `Unknown`).
//
// This is the CDO-confirmed real-world pattern this task's fix closes: see
// `CDOConnecteCandidates.PageExt.al` (app "Continia Document Output")
// bare-calling `internal procedure`s (`GetIsSingleConnect`/
// `GeteCandidatesFiltered`/`GetIsVendor`) declared on the base Page
// `"CTS-CDN Connect eCandidates"` in app "Continia Delivery Network" — a
// genuinely different dependency app.
//
// AL semantics: DOES NOT COMPILE from `ExtA` (access error — `I` is not
// visible outside AppB).
//
// Fresh-engine route: Evidence::Unknown (post-Task-1.5; pre-fix: false
// Evidence::Source targeting Base.I — verified via TDD, see
// `bare_extension_base_cross_app_internal_method_excluded`).
tableextension 52905 "ExtA" extends Base
{
}
