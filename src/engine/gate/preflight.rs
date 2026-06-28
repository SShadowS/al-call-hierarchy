//! Dependency-coverage preflight — port of al-sem `src/cli/preflight.ts`.
//!
//! `degraded` = the analysis cone is incomplete:
//!   - one or more call edges could not be resolved (`unresolved_callsites > 0`), OR
//!   - one or more dependency apps were only available as opaque symbol packages
//!     (`opaque_apps` non-empty).
//!
//! `failed` = `degraded && required` (the `--require-dependencies` flag). Default is
//! fail-OPEN: always surface the signal to stderr, never block by default.

/// `PreflightResult` (al-sem `preflight.ts`).
pub struct PreflightResult {
    pub degraded: bool,
    pub failed: bool,
    pub message: String,
    pub unresolved_callsites: usize,
    /// Sorted (determinism contract — `opaqueApps` is not canonically sorted at source).
    pub opaque_apps: Vec<String>,
}

/// `evaluatePreflight(coverage, required)`.
///
/// `unresolved_callsites` is the COUNT of `coverage.unresolvedCallsites` (al-sem reads
/// `.length`); `opaque_apps` is `coverage.opaqueApps` (sorted here for a deterministic
/// message).
pub fn evaluate_preflight(
    unresolved_callsites: usize,
    opaque_apps: &[String],
    required: bool,
) -> PreflightResult {
    let mut opaque: Vec<String> = opaque_apps.to_vec();
    opaque.sort();
    let degraded = unresolved_callsites > 0 || !opaque.is_empty();
    let message = if degraded {
        let opaque_part = if !opaque.is_empty() {
            format!(", {} opaque app(s): {}", opaque.len(), opaque.join(", "))
        } else {
            String::new()
        };
        format!(
            "analysis coverage degraded — {unresolved_callsites} unresolved callsite(s){opaque_part}"
        )
    } else {
        "dependency coverage complete".to_string()
    };
    PreflightResult {
        degraded,
        failed: degraded && required,
        message,
        unresolved_callsites,
        opaque_apps: opaque,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_not_degraded() {
        let pf = evaluate_preflight(0, &[], true);
        assert!(!pf.degraded);
        assert!(!pf.failed);
        assert_eq!(pf.message, "dependency coverage complete");
    }

    #[test]
    fn unresolved_degraded_failed_only_when_required() {
        let pf = evaluate_preflight(3, &[], false);
        assert!(pf.degraded);
        assert!(!pf.failed);
        let pf = evaluate_preflight(3, &[], true);
        assert!(pf.failed);
        assert_eq!(
            pf.message,
            "analysis coverage degraded — 3 unresolved callsite(s)"
        );
    }

    #[test]
    fn opaque_apps_sorted_in_message() {
        let pf = evaluate_preflight(1, &["b".to_string(), "a".to_string()], true);
        assert!(pf.failed);
        assert_eq!(
            pf.message,
            "analysis coverage degraded — 1 unresolved callsite(s), 2 opaque app(s): a, b"
        );
    }
}
