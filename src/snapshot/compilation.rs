//! Per-app compilation context: each app's own preprocessor symbols + version
//! basis, so dependency `#if` branches are evaluated with THAT app's context,
//! never the workspace's (phantom-edge prevention, charter C3).

use std::collections::BTreeSet;

#[derive(Clone, Debug, Default)]
pub struct CompilationContext {
    pub preproc_symbols: BTreeSet<String>,
    pub runtime: Option<String>,
    pub platform: Option<String>,
    pub application: Option<String>,
}

pub fn context_from_app_json(app_json: &serde_json::Value) -> CompilationContext {
    let get = |k: &str| app_json.get(k).and_then(|v| v.as_str()).map(str::to_string);
    CompilationContext {
        preproc_symbols: BTreeSet::new(),
        runtime: get("runtime"),
        platform: get("platform"),
        application: get("application"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_runtime_and_platform() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"runtime":"15.0","platform":"28.0.0.0","application":"28.0.0.0"}"#,
        )
        .unwrap();
        let c = context_from_app_json(&v);
        assert_eq!(c.runtime.as_deref(), Some("15.0"));
        assert_eq!(c.platform.as_deref(), Some("28.0.0.0"));
        assert!(c.preproc_symbols.is_empty());
    }
}
