//! AL global-builtin allowlist (R2b Task 3) — faithful port of al-sem's
//! `globalBuiltinDisposition` from `src/resolve/al-builtins.ts`.
//!
//! A conservative allowlist of no-receiver global builtins. Recognizing them
//! lets the resolver mark their bare callsites as a known terminal (`builtin`)
//! instead of an unresolved member call. Only PURE intrinsics + control-
//! terminating `Error` are listed (effectful builtins are intentionally absent).
//! Matched case-insensitively.

/// Disposition of a recognized no-receiver global builtin, else `None`.
/// The string variants match al-sem ("pure-terminal" | "control-terminating").
pub fn global_builtin_disposition(name: &str) -> Option<&'static str> {
    match name.to_lowercase().as_str() {
        "error" => Some("control-terminating"),
        "copystr" | "maxstrlen" | "strlen" | "strsubstno" | "format" | "lowercase"
        | "uppercase" | "convertstr" | "delchr" | "padstr" | "incstr" | "abs" | "round"
        | "power" | "userid" | "companyname" | "currentdatetime" | "today" | "time"
        | "workdate" | "createguid" | "isnullguid" => Some("pure-terminal"),
        _ => None,
    }
}
