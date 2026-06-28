//! Preflight checks: schema-version / alsem-version / app-identity. Port of
//! al-sem `src/diff/diff-preflight.ts`.

use crate::engine::gate::cbor::CborValue;

use super::{CoveragePolicy, DiffDiagnostic, get_array, get_str};

pub struct PreflightResult {
    pub diagnostics: Vec<DiffDiagnostic>,
    pub fatal: bool,
}

/// The snapshot's `schemaVersion` as an integer (the deserialized tree carries it
/// as `Int`).
fn schema_version(snap: &CborValue) -> Option<i64> {
    match snap {
        CborValue::Map(m) => match m.get("schemaVersion") {
            Some(CborValue::Int(n)) => Some(*n),
            _ => None,
        },
        _ => None,
    }
}

/// The primary app guid (`apps[0].appGuid`), if any.
fn primary_app_id(snap: &CborValue) -> Option<String> {
    let apps = get_array(snap, "apps")?;
    let first = apps.first()?;
    get_str(first, "appGuid").map(|s| s.to_string())
}

pub fn run_preflight(
    old_snap: &CborValue,
    new_snap: &CborValue,
    coverage_policy: CoveragePolicy,
) -> PreflightResult {
    let mut diagnostics: Vec<DiffDiagnostic> = Vec::new();
    let mut fatal = false;

    // Schema version mismatch — always fatal.
    let old_sv = schema_version(old_snap);
    let new_sv = schema_version(new_snap);
    if old_sv != new_sv {
        diagnostics.push(DiffDiagnostic {
            kind: "schema-version-mismatch".into(),
            fields: vec![
                (
                    "kind".into(),
                    CborValue::Text("schema-version-mismatch".into()),
                ),
                ("old".into(), CborValue::Int(old_sv.unwrap_or(0))),
                ("new".into(), CborValue::Int(new_sv.unwrap_or(0))),
            ],
        });
        fatal = true;
    }

    // alsemVersion difference — warning loose, error strict.
    let old_av = get_str(old_snap, "alsemVersion").unwrap_or("");
    let new_av = get_str(new_snap, "alsemVersion").unwrap_or("");
    if old_av != new_av {
        let severity = if coverage_policy == CoveragePolicy::Strict {
            "error"
        } else {
            "warning"
        };
        diagnostics.push(DiffDiagnostic {
            kind: "alsem-version-mismatch".into(),
            fields: vec![
                (
                    "kind".into(),
                    CborValue::Text("alsem-version-mismatch".into()),
                ),
                ("old".into(), CborValue::Text(old_av.to_string())),
                ("new".into(), CborValue::Text(new_av.to_string())),
                ("severity".into(), CborValue::Text(severity.into())),
            ],
        });
        if severity == "error" {
            fatal = true;
        }
    }

    // App identity mismatch — always fatal.
    let old_app = primary_app_id(old_snap);
    let new_app = primary_app_id(new_snap);
    if let (Some(o), Some(n)) = (&old_app, &new_app)
        && o != n
    {
        diagnostics.push(DiffDiagnostic {
            kind: "app-identity-mismatch".into(),
            fields: vec![
                (
                    "kind".into(),
                    CborValue::Text("app-identity-mismatch".into()),
                ),
                ("oldAppId".into(), CborValue::Text(o.clone())),
                ("newAppId".into(), CborValue::Text(n.clone())),
            ],
        });
        fatal = true;
    }

    PreflightResult { diagnostics, fatal }
}
