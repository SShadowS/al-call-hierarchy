//! Dependency resolution for AL projects
//!
//! Parses app.json to discover dependencies and locates matching .app files
//! in the .alpackages folder.

use crate::app_package::{extract_app_package, ParsedAppPackage};
use anyhow::{Context, Result};
use log::{debug, info, warn};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// A dependency declared in app.json
#[derive(Debug, Clone, Deserialize)]
pub struct AppDependency {
    pub name: String,
    pub publisher: String,
    pub version: String,
}

/// Parsed app.json structure (only the fields we care about)
#[derive(Debug, Deserialize)]
struct AppJson {
    #[serde(default)]
    dependencies: Vec<AppDependency>,
}

/// A resolved dependency with its parsed package
#[derive(Debug)]
pub struct ResolvedDependency {
    pub dependency: AppDependency,
    pub app_path: PathBuf,
    pub package: ParsedAppPackage,
}

/// Parse app.json to extract dependencies
pub fn parse_app_json(path: &Path) -> Result<Vec<AppDependency>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;

    let app_json: AppJson = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse {}", path.display()))?;

    Ok(app_json.dependencies)
}

/// Find the .alpackages folder for a project
pub fn find_alpackages_folder(project_root: &Path) -> Option<PathBuf> {
    let alpackages = project_root.join(".alpackages");
    if alpackages.is_dir() {
        Some(alpackages)
    } else {
        None
    }
}

/// Parse a version string into comparable parts
/// Handles versions like "26.0.0.0" or "26.0.30643.32100"
fn parse_version(version: &str) -> Vec<u64> {
    version
        .split('.')
        .filter_map(|part| part.parse::<u64>().ok())
        .collect()
}

/// Check if actual version is compatible with required version
/// A version is compatible if it's >= the required version in major.minor
fn is_version_compatible(required: &str, actual: &str) -> bool {
    let req_parts = parse_version(required);
    let actual_parts = parse_version(actual);

    // Compare major.minor at minimum
    for i in 0..2.min(req_parts.len()) {
        let req = req_parts.get(i).copied().unwrap_or(0);
        let act = actual_parts.get(i).copied().unwrap_or(0);

        if act > req {
            return true;
        }
        if act < req {
            return false;
        }
    }

    // Major.minor are equal, so it's compatible
    true
}

/// Compare two version strings for sorting (higher version comes first)
fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let a_parts = parse_version(a);
    let b_parts = parse_version(b);

    let max_len = a_parts.len().max(b_parts.len());
    for i in 0..max_len {
        let a_part = a_parts.get(i).copied().unwrap_or(0);
        let b_part = b_parts.get(i).copied().unwrap_or(0);

        match b_part.cmp(&a_part) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }

    std::cmp::Ordering::Equal
}

/// Find a matching .app file for a dependency
/// Returns the path to the best matching .app file (highest compatible version)
pub fn find_matching_app(alpackages: &Path, dep: &AppDependency) -> Option<PathBuf> {
    let entries = match std::fs::read_dir(alpackages) {
        Ok(e) => e,
        Err(_) => return None,
    };

    // Normalize publisher and name for matching
    // File names use underscores: "Publisher_Name_Version.app"
    let expected_prefix = format!("{}_{}_", dep.publisher, dep.name);

    let mut candidates: Vec<(PathBuf, String)> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e == "app").unwrap_or(false) {
            if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                if filename.starts_with(&expected_prefix) {
                    // Extract version from filename
                    // Format: Publisher_Name_Version.app
                    let version_part = &filename[expected_prefix.len()..];
                    if let Some(version) = version_part.strip_suffix(".app") {
                        if is_version_compatible(&dep.version, version) {
                            candidates.push((path.clone(), version.to_string()));
                        }
                    }
                }
            }
        }
    }

    if candidates.is_empty() {
        return None;
    }

    // Sort by version descending (highest first)
    candidates.sort_by(|a, b| compare_versions(&a.1, &b.1));

    // Return the highest compatible version
    candidates.into_iter().next().map(|(path, _)| path)
}

/// Resolve all dependencies for a project
///
/// Returns a list of resolved dependencies with their parsed packages.
/// Dependencies that cannot be resolved are logged as warnings and skipped.
pub fn resolve_all(project_root: &Path) -> Result<Vec<ResolvedDependency>> {
    let app_json_path = project_root.join("app.json");
    if !app_json_path.exists() {
        debug!("No app.json found at {}", project_root.display());
        return Ok(Vec::new());
    }

    let dependencies = parse_app_json(&app_json_path)?;
    if dependencies.is_empty() {
        debug!("No dependencies declared in app.json");
        return Ok(Vec::new());
    }

    let alpackages = match find_alpackages_folder(project_root) {
        Some(path) => path,
        None => {
            warn!("No .alpackages folder found at {}", project_root.display());
            return Ok(Vec::new());
        }
    };

    info!(
        "Resolving {} dependencies from {}",
        dependencies.len(),
        alpackages.display()
    );

    let mut resolved = Vec::new();

    for dep in dependencies {
        match find_matching_app(&alpackages, &dep) {
            Some(app_path) => {
                debug!(
                    "Found {} {} -> {}",
                    dep.name,
                    dep.version,
                    app_path.display()
                );

                match extract_app_package(&app_path) {
                    Ok(package) => {
                        info!(
                            "Loaded {} v{} ({} objects)",
                            package.metadata.name,
                            package.metadata.version,
                            package.objects.len()
                        );
                        resolved.push(ResolvedDependency {
                            dependency: dep,
                            app_path,
                            package,
                        });
                    }
                    Err(e) => {
                        warn!("Failed to parse {}: {}", app_path.display(), e);
                    }
                }
            }
            None => {
                warn!(
                    "Could not find matching .app for {} {} (publisher: {})",
                    dep.name, dep.version, dep.publisher
                );
            }
        }
    }

    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_version() {
        assert_eq!(parse_version("26.0.0.0"), vec![26, 0, 0, 0]);
        assert_eq!(parse_version("26.0.30643.32100"), vec![26, 0, 30643, 32100]);
        assert_eq!(parse_version("1.2.3"), vec![1, 2, 3]);
    }

    #[test]
    fn test_is_version_compatible() {
        // Same major.minor
        assert!(is_version_compatible("26.0.0.0", "26.0.0.0"));
        assert!(is_version_compatible("26.0.0.0", "26.0.30643.32100"));

        // Higher minor
        assert!(is_version_compatible("26.0.0.0", "26.1.0.0"));

        // Lower minor - not compatible
        assert!(!is_version_compatible("26.1.0.0", "26.0.0.0"));

        // Higher major
        assert!(is_version_compatible("26.0.0.0", "27.0.0.0"));

        // Lower major - not compatible
        assert!(!is_version_compatible("27.0.0.0", "26.0.0.0"));
    }

    #[test]
    fn test_compare_versions() {
        use std::cmp::Ordering;

        // Higher version should come first (Less means a > b)
        assert_eq!(
            compare_versions("26.0.30643.32100", "26.0.30643.31340"),
            Ordering::Less
        );
        assert_eq!(compare_versions("26.0.0.0", "25.0.0.0"), Ordering::Less);
        assert_eq!(compare_versions("26.0.0.0", "26.0.0.0"), Ordering::Equal);
        assert_eq!(compare_versions("25.0.0.0", "26.0.0.0"), Ordering::Greater);
    }

    #[test]
    fn test_resolve_real_project() {
        let test_path = Path::new("u:/Git/DO/Cloud");
        if !test_path.exists() {
            eprintln!("Skipping test: test project not found");
            return;
        }

        let result = resolve_all(test_path);
        assert!(result.is_ok(), "Failed to resolve: {:?}", result.err());

        let resolved = result.unwrap();
        println!("Resolved {} dependencies", resolved.len());

        for dep in &resolved {
            println!(
                "  {} v{} -> {} objects",
                dep.package.metadata.name,
                dep.package.metadata.version,
                dep.package.objects.len()
            );
        }

        // Should have at least the declared dependencies
        assert!(
            !resolved.is_empty(),
            "Should resolve at least one dependency"
        );
    }

    #[test]
    fn test_parse_app_json_real_project() {
        let app_json = Path::new("U:/Git/DO.Support-wi-75148/DocumentOutput/Cloud/app.json");
        if !app_json.exists() {
            eprintln!("Skipping test: DO.Support-wi-75148 not available");
            return;
        }

        let deps = parse_app_json(app_json).expect("Failed to parse app.json");
        assert!(!deps.is_empty(), "Should have dependencies");
        println!("Found {} dependencies", deps.len());
        for dep in &deps {
            println!("  {} by {} v{}", dep.name, dep.publisher, dep.version);
        }
    }

    #[test]
    fn test_find_alpackages_folder_exists() {
        let project = Path::new("U:/Git/DO.Support-wi-75148/DocumentOutput/Cloud");
        if !project.exists() {
            eprintln!("Skipping test: project not available");
            return;
        }

        let result = find_alpackages_folder(project);
        assert!(result.is_some(), "Should find .alpackages folder");
    }

    #[test]
    fn test_find_alpackages_folder_missing() {
        let result = find_alpackages_folder(Path::new("/nonexistent/path"));
        assert!(result.is_none());
    }

    #[test]
    fn test_find_matching_app_real() {
        let alpackages = Path::new("U:/Git/DO.Support-wi-75148/DocumentOutput/Cloud/.alpackages");
        if !alpackages.exists() {
            eprintln!("Skipping test: .alpackages not available");
            return;
        }

        let dep = AppDependency {
            name: "Continia Core".to_string(),
            publisher: "Continia Software".to_string(),
            version: "29.0.0.0".to_string(),
        };

        let result = find_matching_app(alpackages, &dep);
        assert!(
            result.is_some(),
            "Should find matching .app for Continia Core"
        );
        let path = result.unwrap();
        assert!(path
            .to_string_lossy()
            .contains("Continia Software_Continia Core_"));
        println!("Found: {}", path.display());
    }

    #[test]
    fn test_find_matching_app_not_found() {
        let alpackages = Path::new("U:/Git/DO.Support-wi-75148/DocumentOutput/Cloud/.alpackages");
        if !alpackages.exists() {
            eprintln!("Skipping test: .alpackages not available");
            return;
        }

        let dep = AppDependency {
            name: "NonExistent App".to_string(),
            publisher: "Nobody".to_string(),
            version: "1.0.0.0".to_string(),
        };

        let result = find_matching_app(alpackages, &dep);
        assert!(result.is_none(), "Should not find non-existent app");
    }

    #[test]
    fn test_resolve_all_do_support() {
        let project = Path::new("U:/Git/DO.Support-wi-75148/DocumentOutput/Cloud");
        if !project.exists() {
            eprintln!("Skipping test: DO.Support-wi-75148 not available");
            return;
        }

        let result = resolve_all(project);
        assert!(result.is_ok(), "Failed to resolve: {:?}", result.err());
        let resolved = result.unwrap();
        println!("Resolved {} dependencies", resolved.len());
        for dep in &resolved {
            println!(
                "  {} v{} -> {} objects",
                dep.package.metadata.name,
                dep.package.metadata.version,
                dep.package.objects.len()
            );
        }
        assert!(
            !resolved.is_empty(),
            "Should resolve at least one dependency"
        );
    }

    #[test]
    fn test_resolve_all_no_app_json() {
        // Use a temp dir with no app.json
        let dir = tempfile::TempDir::new().unwrap();
        let result = resolve_all(dir.path()).unwrap();
        assert!(result.is_empty(), "No app.json should return empty");
    }

    #[test]
    fn test_resolve_all_empty_dependencies() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("app.json"), r#"{"dependencies": []}"#).unwrap();
        let result = resolve_all(dir.path()).unwrap();
        assert!(result.is_empty(), "Empty dependencies should return empty");
    }

    #[test]
    fn test_resolve_all_no_alpackages() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("app.json"),
            r#"{"dependencies": [{"name": "Test", "publisher": "Pub", "version": "1.0.0.0"}]}"#,
        )
        .unwrap();
        let result = resolve_all(dir.path()).unwrap();
        assert!(result.is_empty(), "No .alpackages should return empty");
    }
}
