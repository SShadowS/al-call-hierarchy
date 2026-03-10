//! Configuration for diagnostic thresholds.
//!
//! Loads settings from `.al-call-hierarchy.json` in the workspace root.
//! Missing values use defaults.

use log::{info, warn};
use serde::Deserialize;
use std::path::Path;

/// All configurable diagnostic thresholds
#[derive(Debug, Clone)]
pub struct DiagnosticConfig {
    pub complexity_enabled: bool,
    pub complexity_warning: u32,
    pub complexity_critical: u32,
    pub length_enabled: bool,
    pub length_warning: u32,
    pub length_critical: u32,
    pub params_enabled: bool,
    pub params_warning: u32,
    pub params_critical: u32,
    pub fan_in_enabled: bool,
    pub fan_in_warning: usize,
    pub unused_procedures: bool,
}

impl Default for DiagnosticConfig {
    fn default() -> Self {
        Self {
            complexity_enabled: true,
            complexity_warning: 5,
            complexity_critical: 10,
            length_enabled: true,
            length_warning: 20,
            length_critical: 50,
            params_enabled: true,
            params_warning: 4,
            params_critical: 7,
            fan_in_enabled: true,
            fan_in_warning: 20,
            unused_procedures: true,
        }
    }
}

/// JSON schema for `.al-call-hierarchy.json`
#[derive(Debug, Deserialize, Default)]
struct ConfigFile {
    #[serde(default)]
    diagnostics: DiagnosticsSection,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct DiagnosticsSection {
    complexity: Option<ThresholdPair>,
    parameters: Option<ThresholdPair>,
    line_count: Option<ThresholdPair>,
    fan_in: Option<ThresholdSingle>,
    unused_procedures: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ThresholdPair {
    enabled: Option<bool>,
    warning: Option<u32>,
    critical: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct ThresholdSingle {
    enabled: Option<bool>,
    warning: Option<u32>,
}

impl DiagnosticConfig {
    /// Load config from `.al-call-hierarchy.json` in the given directory.
    /// Falls back to defaults for any missing values.
    pub fn load(workspace_root: &Path) -> Self {
        let config_path = workspace_root.join(".al-call-hierarchy.json");
        if !config_path.exists() {
            return Self::default();
        }

        info!("Loading config from {}", config_path.display());

        let contents = match std::fs::read_to_string(&config_path) {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to read {}: {}", config_path.display(), e);
                return Self::default();
            }
        };

        let file: ConfigFile = match serde_json::from_str(&contents) {
            Ok(f) => f,
            Err(e) => {
                warn!("Failed to parse {}: {}", config_path.display(), e);
                return Self::default();
            }
        };

        let defaults = Self::default();
        let d = file.diagnostics;

        Self {
            complexity_enabled: d.complexity.as_ref()
                .and_then(|c| c.enabled)
                .unwrap_or(defaults.complexity_enabled),
            complexity_warning: d.complexity.as_ref()
                .and_then(|c| c.warning)
                .unwrap_or(defaults.complexity_warning),
            complexity_critical: d.complexity.as_ref()
                .and_then(|c| c.critical)
                .unwrap_or(defaults.complexity_critical),
            length_enabled: d.line_count.as_ref()
                .and_then(|c| c.enabled)
                .unwrap_or(defaults.length_enabled),
            length_warning: d.line_count.as_ref()
                .and_then(|c| c.warning)
                .unwrap_or(defaults.length_warning),
            length_critical: d.line_count.as_ref()
                .and_then(|c| c.critical)
                .unwrap_or(defaults.length_critical),
            params_enabled: d.parameters.as_ref()
                .and_then(|c| c.enabled)
                .unwrap_or(defaults.params_enabled),
            params_warning: d.parameters.as_ref()
                .and_then(|c| c.warning)
                .unwrap_or(defaults.params_warning),
            params_critical: d.parameters.as_ref()
                .and_then(|c| c.critical)
                .unwrap_or(defaults.params_critical),
            fan_in_enabled: d.fan_in.as_ref()
                .and_then(|c| c.enabled)
                .unwrap_or(defaults.fan_in_enabled),
            fan_in_warning: d.fan_in.as_ref()
                .and_then(|c| c.warning)
                .map(|v| v as usize)
                .unwrap_or(defaults.fan_in_warning),
            unused_procedures: d.unused_procedures
                .unwrap_or(defaults.unused_procedures),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = DiagnosticConfig::default();
        assert_eq!(config.complexity_warning, 5);
        assert_eq!(config.complexity_critical, 10);
        assert_eq!(config.params_warning, 4);
        assert_eq!(config.params_critical, 7);
        assert_eq!(config.length_critical, 50);
        assert_eq!(config.fan_in_warning, 20);
        assert!(config.unused_procedures);
    }

    #[test]
    fn test_load_missing_file() {
        let dir = TempDir::new().unwrap();
        let config = DiagnosticConfig::load(dir.path());
        assert_eq!(config.complexity_warning, 5); // default
    }

    #[test]
    fn test_load_partial_config() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join(".al-call-hierarchy.json"),
            r#"{ "diagnostics": { "complexity": { "warning": 8 } } }"#,
        ).unwrap();
        let config = DiagnosticConfig::load(dir.path());
        assert_eq!(config.complexity_warning, 8);
        assert_eq!(config.complexity_critical, 10); // default preserved
        assert_eq!(config.params_warning, 4); // default preserved
    }

    #[test]
    fn test_load_full_config() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join(".al-call-hierarchy.json"),
            r#"{
                "diagnostics": {
                    "complexity": { "warning": 8, "critical": 15 },
                    "parameters": { "warning": 5, "critical": 10 },
                    "lineCount": { "warning": 30, "critical": 80 },
                    "fanIn": { "warning": 30 },
                    "unusedProcedures": false
                }
            }"#,
        ).unwrap();
        let config = DiagnosticConfig::load(dir.path());
        assert_eq!(config.complexity_warning, 8);
        assert_eq!(config.complexity_critical, 15);
        assert_eq!(config.params_warning, 5);
        assert_eq!(config.params_critical, 10);
        assert_eq!(config.length_warning, 30);
        assert_eq!(config.length_critical, 80);
        assert_eq!(config.fan_in_warning, 30);
        assert!(!config.unused_procedures);
    }

    #[test]
    fn test_load_disabled_categories() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join(".al-call-hierarchy.json"),
            r#"{
                "diagnostics": {
                    "complexity": { "enabled": false },
                    "parameters": { "enabled": false },
                    "lineCount": { "enabled": false },
                    "fanIn": { "enabled": false },
                    "unusedProcedures": false
                }
            }"#,
        ).unwrap();
        let config = DiagnosticConfig::load(dir.path());
        assert!(!config.complexity_enabled);
        assert!(!config.params_enabled);
        assert!(!config.length_enabled);
        assert!(!config.fan_in_enabled);
        assert!(!config.unused_procedures);
        // Thresholds still have defaults even when disabled
        assert_eq!(config.complexity_warning, 5);
        assert_eq!(config.complexity_critical, 10);
    }

    #[test]
    fn test_load_invalid_json() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join(".al-call-hierarchy.json"),
            "not json",
        ).unwrap();
        let config = DiagnosticConfig::load(dir.path());
        assert_eq!(config.complexity_warning, 5); // falls back to default
    }
}
