//! Dependency resolution for AL projects
//!
//! Parses app.json to discover dependencies and locates matching .app files
//! in the .alpackages folder.

use crate::app_package::{ParsedAppPackage, extract_app_package};
use anyhow::{Context, Result};
use log::{debug, info, warn};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// A dependency declared in app.json
#[derive(Debug, Clone, Deserialize)]
pub struct AppDependency {
    /// The dependency's stable GUID. From app.json's `id` field, or the
    /// NavxManifest `Dependency@Id` — the only globally-unique identity.
    #[serde(rename = "id", default)]
    pub app_id: String,
    pub name: String,
    #[serde(default)]
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
// `dependency` carried for future consumers / Debug.
#[allow(dead_code)]
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

/// Append IMPLICIT Microsoft Application-/Platform-tier dependency rows to an
/// app's `declared_deps` (beyond-1B.3b Task 5.5 — THE dominant lever for the
/// real-`unknown` burndown).
///
/// Real BC apps declare Base App / System App via the top-level `application`/
/// `platform` VERSION STRING fields in app.json (or the NavxManifest
/// `App@Application`/`App@Platform` attributes for a dependency `.app`) — NEVER
/// via `dependencies[]`. Before this, `src/program`'s closure builder read only
/// the explicit `dependencies[]` array, so Base App/System App were
/// systematically absent from every app's dependency closure and every
/// cross-Microsoft-layer call resolved `OutOfClosure` → `Unknown`.
///
/// Mirrors `engine::deps::cross_app_l3::read_workspace_declared_dependencies`
/// (the existing, already-correct implicit-dep template used by the isolated
/// `engine::l4` subsystem) and, transitively, al-sem `parseWorkspaceDependencies`:
/// a non-empty `application` appends [`crate::engine::deps::cross_app_l3::MS_APPLICATION_TIER`]
/// (using the `application` string as each row's `version`); a non-empty
/// `platform` appends [`crate::engine::deps::cross_app_l3::MS_PLATFORM_TIER`]
/// likewise. An empty/absent field injects NOTHING — fixtures with a minimal
/// app.json (no `application`/`platform`) stay unaffected (low ripple).
///
/// `own_guid` guards against self-referential injection: an app never
/// implicitly depends on itself (e.g. if Base App's own manifest carried a
/// non-empty `application`, it must not gain itself as a "dependency" — the
/// `DependencyGraph::closure` DFS is cycle-safe regardless, but a self-edge is
/// still meaningless topology noise). Tier entries whose GUID matches
/// `own_guid` are skipped.
pub fn append_implicit_ms_tier_deps(
    declared: &mut Vec<AppDependency>,
    own_guid: &str,
    application: Option<&str>,
    platform: Option<&str>,
) {
    use crate::engine::deps::cross_app_l3::{MS_APPLICATION_TIER, MS_PLATFORM_TIER};

    if let Some(app_ver) = application.filter(|s| !s.is_empty()) {
        for (guid, name) in MS_APPLICATION_TIER {
            if *guid == own_guid {
                continue;
            }
            declared.push(AppDependency {
                app_id: guid.to_string(),
                name: name.to_string(),
                publisher: "Microsoft".to_string(),
                version: app_ver.to_string(),
            });
        }
    }

    if let Some(plat_ver) = platform.filter(|s| !s.is_empty()) {
        for (guid, name) in MS_PLATFORM_TIER {
            if *guid == own_guid {
                continue;
            }
            declared.push(AppDependency {
                app_id: guid.to_string(),
                name: name.to_string(),
                publisher: "Microsoft".to_string(),
                version: plat_ver.to_string(),
            });
        }
    }
}

/// Find the .alpackages folder for a project.
pub fn find_alpackages_folder(project_root: &Path) -> Option<PathBuf> {
    let alpackages = project_root.join(".alpackages");
    if alpackages.is_dir() {
        Some(alpackages)
    } else {
        None
    }
}

/// Discover every `.alpackages` folder reachable from `project_root` by
/// walking up the directory tree. The first entry is always the project's
/// own `.alpackages` (or absent if not present). Subsequent entries are
/// ancestor folders' `.alpackages` directories.
///
/// Walking stops at the first of:
///   - filesystem root,
///   - eight ancestors deep (safety),
///   - a directory containing `.git` (project boundary).
///
/// This mirrors the Go wrapper's `DiscoverPackageCachePaths` so the
/// dependency index that the wrapper relies on (for hover enrichment +
/// dependencyDocumentSymbol RPC) matches what AL LSP itself sees via
/// the augmented `packageCachePaths`. Without this, monorepos that keep
/// shared .app files in an ancestor folder would expose those files to
/// AL LSP but not to al-call-hierarchy, leading to inconsistent symbol
/// data between the two indexes.
pub fn find_all_alpackages_folders(project_root: &Path) -> Vec<PathBuf> {
    let mut folders = Vec::new();

    if let Some(own) = find_alpackages_folder(project_root) {
        folders.push(own);
    }

    let mut current = project_root.parent();
    let mut depth = 0;
    while let Some(dir) = current {
        if depth >= 8 {
            break;
        }

        let alpkg = dir.join(".alpackages");
        if alpkg.is_dir() && !folders.iter().any(|p| paths_equal(p, &alpkg)) {
            folders.push(alpkg);
        }

        if dir.join(".git").exists() {
            break;
        }

        current = dir.parent();
        depth += 1;
    }

    folders
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a2), Ok(b2)) => a2 == b2,
        _ => a == b,
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
        if path.extension().map(|e| e == "app").unwrap_or(false)
            && let Some(filename) = path.file_name().and_then(|n| n.to_str())
            && filename.starts_with(&expected_prefix)
        {
            // Extract version from filename
            // Format: Publisher_Name_Version.app
            let version_part = &filename[expected_prefix.len()..];
            if let Some(version) = version_part.strip_suffix(".app")
                && is_version_compatible(&dep.version, version)
            {
                candidates.push((path.clone(), version.to_string()));
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

/// Load every `.app` file present in a project's `.alpackages` folder.
///
/// Unlike `resolve_all`, this doesn't filter by `app.json` declarations —
/// every package found is parsed. Lets us index transitive dependencies
/// (e.g. Base Application) that are sitting in `.alpackages` but aren't
/// listed in the project's direct dependency tree. Mirrors AL LSP behavior.
pub fn load_all_apps(project_root: &Path) -> Result<Vec<ResolvedDependency>> {
    let folders = find_all_alpackages_folders(project_root);
    if folders.is_empty() {
        debug!(
            "load_all_apps: no .alpackages folder at {} or any ancestor",
            project_root.display()
        );
        return Ok(Vec::new());
    }

    let mut out: Vec<ResolvedDependency> = Vec::new();
    let mut seen_paths: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    for alpackages in folders {
        let entries = match std::fs::read_dir(&alpackages) {
            Ok(e) => e,
            Err(e) => {
                warn!(
                    "load_all_apps: read_dir({}) failed: {}",
                    alpackages.display(),
                    e
                );
                continue;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("app") {
                continue;
            }
            // Dedup by canonical path so the same .app file in two scanned
            // folders (rare but possible via symlinks) doesn't get loaded twice.
            let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if !seen_paths.insert(canonical) {
                continue;
            }

            match extract_app_package(&path) {
                Ok(package) => {
                    debug!(
                        "load_all_apps: loaded {} v{} ({} objects) from {}",
                        package.metadata.name,
                        package.metadata.version,
                        package.objects.len(),
                        alpackages.display()
                    );
                    out.push(ResolvedDependency {
                        dependency: AppDependency {
                            app_id: package.metadata.app_id.clone(),
                            name: package.metadata.name.clone(),
                            publisher: package.metadata.publisher.clone(),
                            version: package.metadata.version.clone(),
                        },
                        app_path: path,
                        package,
                    });
                }
                Err(e) => {
                    warn!("load_all_apps: failed to parse {}: {}", path.display(), e);
                }
            }
        }
    }

    // Deterministic, filesystem-independent order so downstream AppRef/NodeId
    // numbering is reproducible across machines (charter C8). Version is compared
    // lexicographically purely as a stable tiebreak, not as semver.
    out.sort_by(|a, b| {
        (
            &a.dependency.app_id,
            &a.dependency.name,
            &a.dependency.publisher,
            &a.dependency.version,
        )
            .cmp(&(
                &b.dependency.app_id,
                &b.dependency.name,
                &b.dependency.publisher,
                &b.dependency.version,
            ))
    });

    Ok(out)
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
                        #[cfg(feature = "telemetry")]
                        crate::telemetry::record_indexer_issue(
                            crate::telemetry::IndexerIssueKind::AppParseFailed,
                            0,
                            None,
                        );
                    }
                }
            }
            None => {
                warn!(
                    "Could not find matching .app for {} {} (publisher: {})",
                    dep.name, dep.version, dep.publisher
                );
                #[cfg(feature = "telemetry")]
                {
                    let dep_id = format!("{}:{}", dep.publisher, dep.name);
                    crate::telemetry::record_indexer_issue(
                        crate::telemetry::IndexerIssueKind::MissingDependency,
                        0,
                        Some(&dep_id),
                    );
                }
            }
        }
    }

    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // beyond-1B.3b Task 5.5: append_implicit_ms_tier_deps
    // -----------------------------------------------------------------------

    #[test]
    fn implicit_deps_appended_when_application_and_platform_non_empty() {
        let mut declared: Vec<AppDependency> = Vec::new();
        append_implicit_ms_tier_deps(
            &mut declared,
            "aaaa0000-0000-0000-0000-000000000000",
            Some("24.0.0.0"),
            Some("24.0.0.0"),
        );
        // 3 MS_APPLICATION_TIER rows + 2 MS_PLATFORM_TIER rows.
        assert_eq!(declared.len(), 5, "got {declared:?}");
        assert!(
            declared
                .iter()
                .any(|d| d.app_id == "437dbf0e-84ff-417a-965d-ed2bb9650972"
                    && d.name == "Base Application"
                    && d.version == "24.0.0.0"),
            "Base App must be injected from `application`; got {declared:?}"
        );
        assert!(
            declared
                .iter()
                .any(|d| d.app_id == "63ca2fa4-4f03-4f2b-a480-172fef340d3f"
                    && d.name == "System Application"
                    && d.version == "24.0.0.0"),
            "System App must be injected from `platform`; got {declared:?}"
        );
    }

    #[test]
    fn implicit_deps_none_when_application_and_platform_absent() {
        let mut declared: Vec<AppDependency> = Vec::new();
        append_implicit_ms_tier_deps(
            &mut declared,
            "aaaa0000-0000-0000-0000-000000000000",
            None,
            None,
        );
        assert!(
            declared.is_empty(),
            "no application/platform must inject nothing; got {declared:?}"
        );
    }

    #[test]
    fn implicit_deps_none_when_application_and_platform_empty_string() {
        let mut declared: Vec<AppDependency> = Vec::new();
        append_implicit_ms_tier_deps(
            &mut declared,
            "aaaa0000-0000-0000-0000-000000000000",
            Some(""),
            Some(""),
        );
        assert!(
            declared.is_empty(),
            "empty-string application/platform must inject nothing (not just absent); got {declared:?}"
        );
    }

    #[test]
    fn implicit_deps_application_only_does_not_inject_platform_tier() {
        let mut declared: Vec<AppDependency> = Vec::new();
        append_implicit_ms_tier_deps(
            &mut declared,
            "aaaa0000-0000-0000-0000-000000000000",
            Some("24.0.0.0"),
            None,
        );
        assert_eq!(
            declared.len(),
            3,
            "only MS_APPLICATION_TIER; got {declared:?}"
        );
        assert!(
            declared
                .iter()
                .all(|d| d.app_id != "63ca2fa4-4f03-4f2b-a480-172fef340d3f"),
            "System App must NOT be injected when `platform` is absent; got {declared:?}"
        );
    }

    #[test]
    fn implicit_deps_self_guard_skips_own_guid() {
        // Base App's own GUID happens to equal an MS_APPLICATION_TIER entry —
        // it must never gain itself as an implicit dependency.
        let mut declared: Vec<AppDependency> = Vec::new();
        append_implicit_ms_tier_deps(
            &mut declared,
            "437dbf0e-84ff-417a-965d-ed2bb9650972", // Base App's own guid
            Some("24.0.0.0"),
            None,
        );
        assert_eq!(
            declared.len(),
            2,
            "Base App entry self-skipped; got {declared:?}"
        );
        assert!(
            declared
                .iter()
                .all(|d| d.app_id != "437dbf0e-84ff-417a-965d-ed2bb9650972"),
            "must not inject Base App as its own dependency; got {declared:?}"
        );
    }

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
            app_id: String::new(),
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
        assert!(
            path.to_string_lossy()
                .contains("Continia Software_Continia Core_")
        );
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
            app_id: String::new(),
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

    // ---------------------------------------------------------------
    // find_all_alpackages_folders — parallel coverage to the Go side's
    // DiscoverPackageCachePaths tests (al-language-server-go/wrapper/
    // project_packagecache_test.go). Keeping these in lockstep prevents
    // the Go and Rust ancestor-walk semantics from drifting apart.
    // ---------------------------------------------------------------

    #[test]
    fn find_all_alpackages_own_only() {
        let dir = tempfile::TempDir::new().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir_all(project.join(".alpackages")).unwrap();

        let folders = find_all_alpackages_folders(&project);
        assert_eq!(folders.len(), 1, "expected single entry, got {:?}", folders);
        assert!(folders[0].ends_with(".alpackages"));
    }

    #[test]
    fn find_all_alpackages_finds_ancestor() {
        let dir = tempfile::TempDir::new().unwrap();
        let parent = dir.path().join("parent");
        let project = parent.join("Cloud");
        std::fs::create_dir_all(parent.join(".alpackages")).unwrap();
        std::fs::create_dir_all(project.join(".alpackages")).unwrap();

        let folders = find_all_alpackages_folders(&project);
        assert!(
            folders.len() >= 2,
            "expected own + ancestor, got {:?}",
            folders
        );
        assert!(folders[0].ends_with(project.join(".alpackages").as_path().file_name().unwrap()));
        assert!(
            folders
                .iter()
                .any(|f| f.starts_with(&parent) && !f.starts_with(&project)),
            "ancestor folder missing from {:?}",
            folders
        );
    }

    #[test]
    fn find_all_alpackages_stops_at_git_boundary() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = dir.path().join("repo");
        let project = repo.join("Cloud");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(project.join(".alpackages")).unwrap();
        // .alpackages ABOVE the .git boundary should NOT be picked up.
        std::fs::create_dir_all(dir.path().join(".alpackages")).unwrap();

        let folders = find_all_alpackages_folders(&project);
        let above = dir.path().join(".alpackages");
        for f in &folders {
            assert!(
                std::fs::canonicalize(f).unwrap() != std::fs::canonicalize(&above).unwrap(),
                "walked past .git boundary: included {:?}",
                f
            );
        }
    }

    #[test]
    fn find_all_alpackages_no_dups() {
        let dir = tempfile::TempDir::new().unwrap();
        let parent = dir.path().join("parent");
        let project = parent.join("child");
        std::fs::create_dir_all(parent.join(".alpackages")).unwrap();
        std::fs::create_dir_all(project.join(".alpackages")).unwrap();

        let folders = find_all_alpackages_folders(&project);
        let mut canonical: Vec<_> = folders
            .iter()
            .map(|f| std::fs::canonicalize(f).unwrap())
            .collect();
        canonical.sort();
        let len_before = canonical.len();
        canonical.dedup();
        assert_eq!(
            canonical.len(),
            len_before,
            "duplicate path in {:?}",
            folders
        );
    }

    #[test]
    fn find_all_alpackages_depth_cap() {
        // Build a long ancestor chain with .alpackages in every level.
        // The walk should stop at the 8-deep cap regardless.
        let dir = tempfile::TempDir::new().unwrap();
        let mut current = dir.path().to_path_buf();
        // 12 levels of ancestor .alpackages
        for i in 0..12 {
            current = current.join(format!("level{}", i));
            std::fs::create_dir_all(current.join(".alpackages")).unwrap();
        }
        // Place project at the deepest level.
        let project = current.clone();

        let folders = find_all_alpackages_folders(&project);
        // Own + at most 8 ancestors = 9 total.
        assert!(
            folders.len() <= 9,
            "expected ≤ 9 (own + 8 ancestors), got {}",
            folders.len()
        );
    }
}
