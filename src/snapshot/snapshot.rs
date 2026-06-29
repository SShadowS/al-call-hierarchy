//! `AppUnit`, `AppSetSnapshot`, and `SnapshotBuilder` — the integration
//! linchpin that turns a workspace root into an identity-verified, per-app
//! source set.

use crate::app_package::ParsedAppPackage;
use crate::dependencies::load_all_apps;
use crate::snapshot::compilation::{
    CompilationContext, context_from_app_json, context_from_metadata,
};
use crate::snapshot::identity::{AppId, Provenance, TrustTier};
use crate::snapshot::provider::{
    EmbeddedAppProvider, LocalRepoProvider, SourceProvider, SourceRoot, SymbolOnlyProvider,
    WorkspaceProvider, select_source,
};
use anyhow::{Context, Result};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Whether the app set is a closed (all deps known) or open universe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum World {
    /// All reachable dependencies are represented in `AppSetSnapshot::apps`.
    Closed,
    /// The app set may be incomplete (reserved for reverse-dependents — later task).
    Open,
}

/// One app's resolved identity, source, compilation context, and ABI.
#[derive(Clone, Debug)]
pub struct AppUnit {
    pub id: AppId,
    pub provenance: Provenance,
    /// AL source files, if available (None = symbol-only boundary).
    pub source: Option<SourceRoot>,
    /// Per-app preprocessor symbols + version basis for `#if` evaluation.
    pub compilation: CompilationContext,
    /// This app's declared dependencies (each with its real GUID) — drives
    /// dependency-topology-aware resolution. Workspace deps come from app.json;
    /// dependency-app deps from their `.app` NavxManifest.
    pub declared_deps: Vec<crate::dependencies::AppDependency>,
    /// Parsed `.app` symbol table (None for the workspace itself).
    pub abi: Option<ParsedAppPackage>,
}

/// The full set of apps visible in a workspace, keyed by identity.
#[derive(Debug, Clone)]
pub struct AppSetSnapshot {
    /// All app units: index 0 is always the workspace app.
    pub apps: Vec<AppUnit>,
    /// Identity of the workspace app (mirrors `apps[0].id`).
    pub workspace_app: AppId,
    pub world: World,
}

/// Builds an `AppSetSnapshot` from a workspace root + optional local checkouts.
#[derive(Debug)]
pub struct SnapshotBuilder {
    /// Root of the AL workspace (must contain `app.json`).
    pub workspace_root: PathBuf,
    /// Local source checkouts to prefer over embedded source, keyed by `AppId`.
    pub local_providers: Vec<(AppId, PathBuf)>,
}

impl SnapshotBuilder {
    /// Build the snapshot, loading the workspace and all `.alpackages` deps.
    pub fn build(&self) -> Result<AppSetSnapshot> {
        let ws = &self.workspace_root;

        // ------------------------------------------------------------------
        // Workspace unit
        // ------------------------------------------------------------------
        let app_json_path = ws.join("app.json");
        let app_json_text = std::fs::read_to_string(&app_json_path)
            .with_context(|| format!("read {}", app_json_path.display()))?;
        let app_json: serde_json::Value = serde_json::from_str(&app_json_text)
            .with_context(|| format!("parse {}", app_json_path.display()))?;

        let get_str = |k: &str| -> String {
            app_json
                .get(k)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };

        let workspace_app = AppId {
            guid: get_str("id"),
            name: get_str("name"),
            publisher: get_str("publisher"),
            version: get_str("version"),
        };

        let ws_compilation = context_from_app_json(&app_json);
        // Workspace's declared dependencies (with their GUIDs) from app.json.
        let ws_declared_deps: Vec<crate::dependencies::AppDependency> = app_json
            .get("dependencies")
            .and_then(|d| serde_json::from_value(d.clone()).ok())
            .unwrap_or_default();

        let ws_source_provider = WorkspaceProvider { root: ws.clone() };
        let ws_source = ws_source_provider
            .try_provide(&workspace_app)
            .context("workspace source provider")?;

        let ws_tier = ws_source
            .as_ref()
            .map(|s| s.tier)
            .unwrap_or(TrustTier::SymbolOnly);
        let ws_hash = ws_source
            .as_ref()
            .map(|s| s.content_hash.clone())
            .unwrap_or_default();

        let ws_unit = AppUnit {
            provenance: Provenance {
                app: workspace_app.clone(),
                tier: ws_tier,
                content_hash: ws_hash,
            },
            id: workspace_app.clone(),
            source: ws_source,
            compilation: ws_compilation,
            declared_deps: ws_declared_deps,
            abi: None,
        };

        // ------------------------------------------------------------------
        // Dependency units
        // ------------------------------------------------------------------
        let resolved_deps = load_all_apps(ws)?;

        let mut apps: Vec<AppUnit> = Vec::with_capacity(1 + resolved_deps.len());
        apps.push(ws_unit);

        for rd in resolved_deps {
            // Identity from the `.app`'s own NavxManifest (authoritative) — the
            // GUID is the only globally-unique id. Fall back to the app.json dep
            // entry for any field the manifest left blank.
            let m = &rd.package.metadata;
            let dep_id = AppId {
                guid: m.app_id.clone(),
                name: if m.name.is_empty() {
                    rd.dependency.name.clone()
                } else {
                    m.name.clone()
                },
                publisher: if m.publisher.is_empty() {
                    rd.dependency.publisher.clone()
                } else {
                    m.publisher.clone()
                },
                version: if m.version.is_empty() {
                    rd.dependency.version.clone()
                } else {
                    m.version.clone()
                },
            };

            // Build provider chain: EmbeddedAppProvider → LocalRepoProvider (if matched) → SymbolOnlyProvider.
            let mut providers: Vec<Box<dyn SourceProvider>> = vec![Box::new(EmbeddedAppProvider {
                app_path: rd.app_path.clone(),
            })];
            // Match a configured local provider by GUID when known (the unique
            // identity), else by name (case-insensitive) + version.
            if let Some((id, path)) = self.local_providers.iter().find(|(id, _)| {
                (!dep_id.guid.is_empty() && id.guid == dep_id.guid)
                    || (id.name.eq_ignore_ascii_case(&dep_id.name) && id.version == dep_id.version)
            }) {
                providers.push(Box::new(LocalRepoProvider {
                    app: id.clone(),
                    root: path.clone(),
                }));
            }
            providers.push(Box::new(SymbolOnlyProvider));

            let source = select_source(&dep_id, &providers)?;
            let tier = source
                .as_ref()
                .map(|s| s.tier)
                .unwrap_or(TrustTier::SymbolOnly);
            let content_hash = source
                .as_ref()
                .map(|s| s.content_hash.clone())
                .unwrap_or_default();

            // Real compilation basis + declared deps from the dep's manifest
            // (computed before `rd.package` is moved into `abi`).
            let dep_compilation = context_from_metadata(&rd.package.metadata);
            let dep_declared = rd.package.metadata.dependencies.clone();
            apps.push(AppUnit {
                provenance: Provenance {
                    app: dep_id.clone(),
                    tier,
                    content_hash,
                },
                id: dep_id,
                source,
                compilation: dep_compilation,
                declared_deps: dep_declared,
                abi: Some(rd.package),
            });
        }

        Ok(AppSetSnapshot {
            workspace_app,
            apps,
            world: World::Closed,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_snapshot_over_cdo_workspace() {
        let Some(ws) = std::env::var_os("CDO_WS")
            .map(std::path::PathBuf::from)
            .filter(|p| p.exists())
        else {
            return;
        };
        let snap = SnapshotBuilder {
            workspace_root: ws,
            local_providers: vec![],
        }
        .build()
        .expect("snapshot");
        // workspace + >=9 dep apps
        assert!(snap.apps.len() >= 10, "got {}", snap.apps.len());
        // 9/10 deps ship ShowMyCode source → at least 9 units have source
        let with_src = snap.apps.iter().filter(|u| u.source.is_some()).count();
        assert!(with_src >= 9, "expected >=9 source units, got {with_src}");
        // At least one dep has no embedded source (symbol-only)
        let sym_only = snap.apps.iter().filter(|u| u.source.is_none()).count();
        assert!(
            sym_only >= 1,
            "expected >=1 symbol-only units, got {sym_only}"
        );
        // Dependency apps carry their REAL unique GUID (from the .app NavxManifest
        // `App@Id`), not an empty string — the identity foundation 1B builds on.
        let deps_with_guid = snap
            .apps
            .iter()
            .skip(1) // apps[0] = workspace
            .filter(|u| u.id.guid.len() == 36 && u.id.guid.contains('-'))
            .count();
        assert!(
            deps_with_guid >= 9,
            "expected >=9 deps with a real GUID, got {deps_with_guid}"
        );
        // #1: dependency apps carry a REAL compilation basis (runtime/platform
        // from their manifest), not an empty default context.
        let deps_with_runtime = snap
            .apps
            .iter()
            .skip(1)
            .filter(|u| u.compilation.runtime.is_some() || u.compilation.application.is_some())
            .count();
        assert!(
            deps_with_runtime >= 5,
            "expected deps to carry a real compilation basis, got {deps_with_runtime}"
        );
        // #2: dependency topology is captured — at least one dep declares its own
        // dependencies (each with a GUID), enabling topology-aware resolution.
        let some_dep_declares_deps = snap.apps.iter().skip(1).any(|u| {
            !u.declared_deps.is_empty() && u.declared_deps.iter().all(|d| d.app_id.len() == 36)
        });
        assert!(
            some_dep_declares_deps,
            "expected at least one dep to declare its own GUID-bearing dependencies"
        );
        // Dependency apps are in a deterministic, filesystem-independent order
        // (sorted by AppId) so AppRef/NodeId numbering is reproducible (charter C8).
        let dep_ids: Vec<_> = snap.apps[1..]
            .iter()
            .map(|u| (&u.id.guid, &u.id.name, &u.id.publisher, &u.id.version))
            .collect();
        let mut sorted = dep_ids.clone();
        sorted.sort();
        assert_eq!(dep_ids, sorted, "dependency apps must be sorted by AppId");
    }
}
