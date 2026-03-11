//! Configuration for diagnostic thresholds.
//!
//! Config resolution order (later wins per field):
//! 1. Built-in defaults
//! 2. Global config at `~/.al-call-hierarchy/config.json`
//! 3. Workspace config at `{workspace}/.al-call-hierarchy.json`

use log::{info, warn};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// All configurable diagnostic thresholds (fully resolved, no Options)
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

/// JSON schema for config files (both global and workspace)
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

/// Returns the global config path: `~/.al-call-hierarchy/config.json`
fn global_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".al-call-hierarchy").join("config.json"))
}

/// Parse a config file, returning None if missing or invalid.
fn load_file(path: &Path) -> Option<ConfigFile> {
    if !path.exists() {
        return None;
    }

    info!("Loading config from {}", path.display());

    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to read {}: {}", path.display(), e);
            return None;
        }
    };

    match serde_json::from_str(&contents) {
        Ok(f) => Some(f),
        Err(e) => {
            warn!("Failed to parse {}: {}", path.display(), e);
            None
        }
    }
}

/// Merge two DiagnosticsSections. `overlay` values take priority over `base`.
fn merge_sections(base: DiagnosticsSection, overlay: DiagnosticsSection) -> DiagnosticsSection {
    DiagnosticsSection {
        complexity: merge_threshold_pair(base.complexity, overlay.complexity),
        parameters: merge_threshold_pair(base.parameters, overlay.parameters),
        line_count: merge_threshold_pair(base.line_count, overlay.line_count),
        fan_in: merge_threshold_single(base.fan_in, overlay.fan_in),
        unused_procedures: overlay.unused_procedures.or(base.unused_procedures),
    }
}

fn merge_threshold_pair(base: Option<ThresholdPair>, overlay: Option<ThresholdPair>) -> Option<ThresholdPair> {
    match (base, overlay) {
        (None, None) => None,
        (Some(b), None) => Some(b),
        (None, Some(o)) => Some(o),
        (Some(b), Some(o)) => Some(ThresholdPair {
            enabled: o.enabled.or(b.enabled),
            warning: o.warning.or(b.warning),
            critical: o.critical.or(b.critical),
        }),
    }
}

fn merge_threshold_single(base: Option<ThresholdSingle>, overlay: Option<ThresholdSingle>) -> Option<ThresholdSingle> {
    match (base, overlay) {
        (None, None) => None,
        (Some(b), None) => Some(b),
        (None, Some(o)) => Some(o),
        (Some(b), Some(o)) => Some(ThresholdSingle {
            enabled: o.enabled.or(b.enabled),
            warning: o.warning.or(b.warning),
        }),
    }
}

/// Apply defaults to a merged DiagnosticsSection, producing the final config.
fn apply_defaults(section: DiagnosticsSection) -> DiagnosticConfig {
    let defaults = DiagnosticConfig::default();

    DiagnosticConfig {
        complexity_enabled: section.complexity.as_ref()
            .and_then(|c| c.enabled)
            .unwrap_or(defaults.complexity_enabled),
        complexity_warning: section.complexity.as_ref()
            .and_then(|c| c.warning)
            .unwrap_or(defaults.complexity_warning),
        complexity_critical: section.complexity.as_ref()
            .and_then(|c| c.critical)
            .unwrap_or(defaults.complexity_critical),
        length_enabled: section.line_count.as_ref()
            .and_then(|c| c.enabled)
            .unwrap_or(defaults.length_enabled),
        length_warning: section.line_count.as_ref()
            .and_then(|c| c.warning)
            .unwrap_or(defaults.length_warning),
        length_critical: section.line_count.as_ref()
            .and_then(|c| c.critical)
            .unwrap_or(defaults.length_critical),
        params_enabled: section.parameters.as_ref()
            .and_then(|c| c.enabled)
            .unwrap_or(defaults.params_enabled),
        params_warning: section.parameters.as_ref()
            .and_then(|c| c.warning)
            .unwrap_or(defaults.params_warning),
        params_critical: section.parameters.as_ref()
            .and_then(|c| c.critical)
            .unwrap_or(defaults.params_critical),
        fan_in_enabled: section.fan_in.as_ref()
            .and_then(|c| c.enabled)
            .unwrap_or(defaults.fan_in_enabled),
        fan_in_warning: section.fan_in.as_ref()
            .and_then(|c| c.warning)
            .map(|v| v as usize)
            .unwrap_or(defaults.fan_in_warning),
        unused_procedures: section.unused_procedures
            .unwrap_or(defaults.unused_procedures),
    }
}

impl DiagnosticConfig {
    /// Load config by merging: defaults → global → workspace.
    pub fn load(workspace_root: &Path) -> Self {
        // Phase 1: Load both config files
        let global = global_config_path()
            .and_then(|p| load_file(&p));
        let workspace = load_file(&workspace_root.join(".al-call-hierarchy.json"));

        // Phase 2: Merge sections (global as base, workspace as overlay)
        let merged = match (global, workspace) {
            (None, None) => DiagnosticsSection::default(),
            (Some(g), None) => g.diagnostics,
            (None, Some(w)) => w.diagnostics,
            (Some(g), Some(w)) => merge_sections(g.diagnostics, w.diagnostics),
        };

        // Phase 3: Apply defaults
        apply_defaults(merged)
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

    #[test]
    fn test_merge_sections_deep() {
        let global = DiagnosticsSection {
            complexity: Some(ThresholdPair {
                enabled: None,
                warning: Some(8),
                critical: Some(15),
            }),
            parameters: Some(ThresholdPair {
                enabled: Some(false),
                warning: None,
                critical: None,
            }),
            line_count: None,
            fan_in: None,
            unused_procedures: Some(false),
        };
        let workspace = DiagnosticsSection {
            complexity: Some(ThresholdPair {
                enabled: None,
                warning: Some(5),
                critical: None,
            }),
            parameters: None,
            line_count: None,
            fan_in: None,
            unused_procedures: Some(true),
        };

        let merged = merge_sections(global, workspace);
        let config = apply_defaults(merged);

        // complexity.warning: workspace 5 overrides global 8
        assert_eq!(config.complexity_warning, 5);
        // complexity.critical: global 15 (workspace didn't set it)
        assert_eq!(config.complexity_critical, 15);
        // complexity.enabled: default true (neither set it)
        assert!(config.complexity_enabled);
        // parameters.enabled: global false (workspace didn't set it)
        assert!(!config.params_enabled);
        // parameters.warning: default 4 (neither set it)
        assert_eq!(config.params_warning, 4);
        // unusedProcedures: workspace true overrides global false
        assert!(config.unused_procedures);
        // lineCount: all defaults
        assert_eq!(config.length_warning, 20);
        assert_eq!(config.length_critical, 50);
    }

    #[test]
    fn test_global_config_path() {
        let path = global_config_path();
        // Should return Some on any system with a home directory
        assert!(path.is_some());
        let p = path.unwrap();
        assert!(p.ends_with(".al-call-hierarchy/config.json") ||
                p.ends_with(".al-call-hierarchy\\config.json"));
    }
}
