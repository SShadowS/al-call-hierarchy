// PageExtension generalization of case (a): `ExtA extends BasePage`,
// bare-calling `L()` — same `local`-excluded rule as the TableExtension case,
// generalized via `extension_base_kind`/`object_has_visible_member_
// candidate` (kind-generic, not TableExtension-specific).
//
// AL semantics: DOES NOT COMPILE from `ExtA` (access error).
//
// Fresh-engine route: Evidence::Unknown (post-Task-1.5; pre-fix: false
// Evidence::Source targeting BasePage.L).
pageextension 52909 "ExtA" extends BasePage
{
}
