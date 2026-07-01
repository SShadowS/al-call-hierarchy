// Case (e) cross-app `local` — the DEPENDENCY app (AppB). Mirrors the
// EXISTING (pre-Task-1) regression test
// `resolve_member_record_cross_app_extension_local_method_excluded`
// (`src/program/resolve/resolver.rs`), which already covers this case and
// stays green across this task's refactor (`local` is a fortiori excluded
// cross-app, same as it now is same-app-cross-object — case (b)).
table 52410 "LocFoo"
{
}
