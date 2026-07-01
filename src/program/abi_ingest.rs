//! Cached ABI ingestion: parse SymbolOnly dep .app packages into graph nodes.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::app_package::open_app_zip;
use crate::engine::deps::symbol_reference::{
    AbiEventKind as SrAbiEventKind, AbiRoutine, SymbolReferenceAbi, parse_symbol_reference,
};
use crate::program::node::{AppRef, ObjKey, ObjectNodeId, RoutineNodeId};
use crate::program::node_extract::{Access, ObjectNode, RoutineNode};
use crate::program::resolve::edge::{AbiEventKind, AbiRoutineKind};
use crate::program::resolve::event::PublisherKind;
use crate::snapshot::{AppUnit, TrustTier};
use al_syntax::ir::ObjectKind;

// ---------------------------------------------------------------------------
// FNV-1a fingerprint (stable across runs)
// ---------------------------------------------------------------------------

fn fnv1a(data: &str) -> u64 {
    let mut h: u64 = 14695981039346656037;
    for b in data.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

pub(crate) fn param_type_fp(params: &[crate::engine::deps::symbol_reference::AbiParameter]) -> u64 {
    if params.is_empty() {
        return 0;
    }
    let joined: String = params
        .iter()
        .map(|p| p.type_text.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("|");
    fnv1a(&joined)
}

// ---------------------------------------------------------------------------
// Process-level cache for parsed SymbolReferenceAbi
// ---------------------------------------------------------------------------

/// Cache key: `(guid, name, publisher, version)`.
type AbiCacheKey = (String, String, String, String);
/// Inner map type for `AbiCache`.
type AbiCacheMap = Mutex<HashMap<AbiCacheKey, Arc<SymbolReferenceAbi>>>;

/// Process-level cache for parsed `SymbolReferenceAbi`.
///
/// Keyed by `(guid, name, publisher, version)` so each distinct app version is
/// cached independently.  Pre-seeded via [`AbiCache::seed`] for test fixtures
/// (no file I/O needed); production code fills it lazily from `.app` files via
/// [`AbiCache::get_or_load`].
#[derive(Default)]
pub struct AbiCache {
    inner: AbiCacheMap,
    /// Number of file reads performed (excludes cache hits and pre-seeded entries).
    pub parse_count: std::sync::atomic::AtomicUsize,
}

impl AbiCache {
    pub fn new() -> Self {
        AbiCache::default()
    }

    /// Pre-seed the cache with an already-parsed `SymbolReferenceAbi`.
    ///
    /// Used in tests to avoid file I/O.  A subsequent `get_or_load` for the
    /// same key returns the seeded value without incrementing `parse_count`.
    pub fn seed(
        &self,
        guid: &str,
        name: &str,
        publisher: &str,
        version: &str,
        abi: Arc<SymbolReferenceAbi>,
    ) {
        let key = (
            guid.to_string(),
            name.to_string(),
            publisher.to_string(),
            version.to_string(),
        );
        self.inner.lock().unwrap().insert(key, abi);
    }

    fn get(
        &self,
        guid: &str,
        name: &str,
        publisher: &str,
        version: &str,
    ) -> Option<Arc<SymbolReferenceAbi>> {
        let key = (
            guid.to_string(),
            name.to_string(),
            publisher.to_string(),
            version.to_string(),
        );
        self.inner.lock().unwrap().get(&key).cloned()
    }

    fn get_or_load(
        &self,
        guid: &str,
        name: &str,
        publisher: &str,
        version: &str,
        app_path: &Path,
    ) -> Arc<SymbolReferenceAbi> {
        {
            let map = self.inner.lock().unwrap();
            let key = (
                guid.to_string(),
                name.to_string(),
                publisher.to_string(),
                version.to_string(),
            );
            if let Some(arc) = map.get(&key) {
                return arc.clone();
            }
        }
        self.parse_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let abi = read_symbol_reference_from_app(app_path)
            .unwrap_or_else(|_| SymbolReferenceAbi::default());
        let arc = Arc::new(abi);
        let key = (
            guid.to_string(),
            name.to_string(),
            publisher.to_string(),
            version.to_string(),
        );
        self.inner.lock().unwrap().insert(key, arc.clone());
        arc
    }
}

// ---------------------------------------------------------------------------
// I/O helpers
// ---------------------------------------------------------------------------

pub(crate) fn read_symbol_reference_from_app(path: &Path) -> anyhow::Result<SymbolReferenceAbi> {
    let mut archive = open_app_zip(path)?;
    let mut sr_file = archive.by_name("SymbolReference.json")?;
    let mut content = Vec::new();
    sr_file.read_to_end(&mut content)?;
    let json_str = if content.starts_with(&[0xEF, 0xBB, 0xBF]) {
        std::str::from_utf8(&content[3..])?
    } else {
        std::str::from_utf8(&content)?
    };
    Ok(parse_symbol_reference(json_str))
}

// ---------------------------------------------------------------------------
// Mapping helpers
// ---------------------------------------------------------------------------

pub(crate) fn object_kind_from_abi_type(object_type: &str) -> ObjectKind {
    match object_type.to_ascii_lowercase().as_str() {
        "codeunit" => ObjectKind::Codeunit,
        "table" => ObjectKind::Table,
        "page" => ObjectKind::Page,
        "report" => ObjectKind::Report,
        "query" => ObjectKind::Query,
        "xmlport" => ObjectKind::XmlPort,
        "interface" => ObjectKind::Interface,
        "enum" | "enumtype" => ObjectKind::Enum,
        "enumextension" | "enumextensiontype" => ObjectKind::EnumExtension,
        "tableextension" => ObjectKind::TableExtension,
        "pageextension" => ObjectKind::PageExtension,
        "reportextension" => ObjectKind::ReportExtension,
        "controladdin" => ObjectKind::ControlAddIn,
        "entitlement" => ObjectKind::Entitlement,
        "permissionset" => ObjectKind::PermissionSet,
        "permissionsetextension" => ObjectKind::PermissionSetExtension,
        "profile" => ObjectKind::Profile,
        _ => ObjectKind::Codeunit,
    }
}

fn abi_routine_kind_from_str(
    routine: &AbiRoutine,
) -> (AbiRoutineKind, AbiEventKind, Option<PublisherKind>) {
    match routine.kind.as_str() {
        "event-publisher" => {
            let (abi_ek, pk) = match &routine.event_kind {
                SrAbiEventKind::Integration => {
                    (AbiEventKind::Integration, Some(PublisherKind::Integration))
                }
                SrAbiEventKind::Business => (AbiEventKind::Business, Some(PublisherKind::Business)),
                SrAbiEventKind::Unknown => (AbiEventKind::Internal, Some(PublisherKind::Internal)),
            };
            (AbiRoutineKind::EventPublisher, abi_ek, pk)
        }
        "event-subscriber" => (AbiRoutineKind::EventSubscriber, AbiEventKind::None, None),
        _ => (AbiRoutineKind::Procedure, AbiEventKind::None, None),
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Ingest one SymbolOnly dep unit into `ObjectNode` + `RoutineNode` lists.
///
/// Returns empty vecs when the ABI is not available (no `app_path` and not
/// seeded in `cache`).  Local and internal routines are silently skipped.
pub fn ingest_abi(
    unit: &AppUnit,
    app: AppRef,
    cache: &AbiCache,
) -> (Vec<ObjectNode>, Vec<RoutineNode>) {
    let id = &unit.id;
    let abi: Arc<SymbolReferenceAbi> =
        if let Some(cached) = cache.get(&id.guid, &id.name, &id.publisher, &id.version) {
            cached
        } else if let Some(path) = &unit.app_path {
            cache.get_or_load(&id.guid, &id.name, &id.publisher, &id.version, path)
        } else {
            return (vec![], vec![]);
        };

    let mut objects: Vec<ObjectNode> = Vec::new();
    let mut routines: Vec<RoutineNode> = Vec::new();

    for abi_obj in &abi.objects {
        let kind = object_kind_from_abi_type(&abi_obj.object_type);
        let key = if abi_obj.object_number != 0 {
            ObjKey::Id(abi_obj.object_number)
        } else {
            ObjKey::Name(abi_obj.name.to_ascii_lowercase())
        };
        let obj_id = ObjectNodeId { app, kind, key };

        objects.push(ObjectNode {
            id: obj_id.clone(),
            name: abi_obj.name.clone(),
            declared_id: if abi_obj.object_number != 0 {
                Some(abi_obj.object_number)
            } else {
                None
            },
            extends_target: abi_obj.extends_target_name.clone(),
            implements: abi_obj.implemented_interfaces.clone().unwrap_or_default(),
            tier: TrustTier::SymbolOnly,
        });

        for routine in &abi_obj.routines {
            if routine.is_local || routine.is_internal {
                continue;
            }
            let name_lc = routine.name.to_ascii_lowercase();
            let params_count = routine.parameters.len();
            let sig_fp = param_type_fp(&routine.parameters);
            let (routine_kind, event_kind, publisher_kind) = abi_routine_kind_from_str(routine);

            let rid = RoutineNodeId {
                object: obj_id.clone(),
                name_lc: name_lc.clone(),
                enclosing_member_lc: None,
                params_count,
                sig_fp,
            };

            routines.push(RoutineNode {
                id: rid,
                name: routine.name.clone(),
                is_trigger: false,
                access: Access::Public,
                tier: TrustTier::SymbolOnly,
                event_subscribers: vec![],
                subscriber_instance_manual: false,
                publisher_kind,
                abi_routine_kind: Some(routine_kind),
                abi_event_kind: Some(event_kind),
                // ABI routines already carry a non-zero `sig_fp` in `rid` when
                // signatures differ (see `param_type_fp` above), so a same-id
                // run here is already a true duplicate — no content key needed.
                param_sig_key: String::new(),
            });
        }
    }

    (objects, routines)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::deps::symbol_reference::{
        AbiEventKind as SrAbiEventKind, AbiObject, AbiParameter, AbiRoutine, SymbolReferenceAbi,
    };
    use crate::program::build::build_program_graph;
    use crate::snapshot::compilation::CompilationContext;
    use crate::snapshot::provider::SourceRoot;
    use crate::snapshot::snapshot::AppUnit;
    use crate::snapshot::snapshot::{AppSetSnapshot, World};
    use crate::snapshot::{AppId, Provenance};
    use al_syntax::ir::ObjectKind;
    use std::sync::Arc;

    fn dep_id(name: &str) -> AppId {
        AppId {
            guid: format!("dep-{name}"),
            name: name.into(),
            publisher: "Test".into(),
            version: "1.0.0.0".into(),
        }
    }

    fn ws_id() -> AppId {
        AppId {
            guid: "ws-guid".into(),
            name: "Workspace".into(),
            publisher: "Test".into(),
            version: "1.0.0.0".into(),
        }
    }

    fn make_dep_pub_abi() -> SymbolReferenceAbi {
        SymbolReferenceAbi {
            objects: vec![AbiObject {
                object_type: "Codeunit".into(),
                object_number: 50100,
                name: "Dep Pub".into(),
                routines: vec![
                    AbiRoutine {
                        name: "OnDepEvent".into(),
                        kind: "event-publisher".into(),
                        event_kind: SrAbiEventKind::Integration,
                        parameters: vec![
                            AbiParameter {
                                name: "p1".into(),
                                type_text: "Integer".into(),
                                is_var: false,
                                is_temporary: false,
                            },
                            AbiParameter {
                                name: "p2".into(),
                                type_text: "Text".into(),
                                is_var: false,
                                is_temporary: false,
                            },
                        ],
                        return_type_text: None,
                        is_local: false,
                        is_internal: false,
                        attributes: vec![],
                        attributes_parsed: vec![],
                    },
                    AbiRoutine {
                        name: "DoDepWork".into(),
                        kind: "procedure".into(),
                        event_kind: SrAbiEventKind::Unknown,
                        parameters: vec![AbiParameter {
                            name: "x".into(),
                            type_text: "Integer".into(),
                            is_var: false,
                            is_temporary: false,
                        }],
                        return_type_text: None,
                        is_local: false,
                        is_internal: false,
                        attributes: vec![],
                        attributes_parsed: vec![],
                    },
                    AbiRoutine {
                        name: "F".into(),
                        kind: "procedure".into(),
                        event_kind: SrAbiEventKind::Unknown,
                        parameters: vec![AbiParameter {
                            name: "a".into(),
                            type_text: "Integer".into(),
                            is_var: false,
                            is_temporary: false,
                        }],
                        return_type_text: None,
                        is_local: false,
                        is_internal: false,
                        attributes: vec![],
                        attributes_parsed: vec![],
                    },
                    AbiRoutine {
                        name: "F".into(),
                        kind: "procedure".into(),
                        event_kind: SrAbiEventKind::Unknown,
                        parameters: vec![AbiParameter {
                            name: "a".into(),
                            type_text: "Text".into(),
                            is_var: false,
                            is_temporary: false,
                        }],
                        return_type_text: None,
                        is_local: false,
                        is_internal: false,
                        attributes: vec![],
                        attributes_parsed: vec![],
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn make_ws_unit(ws: &AppId) -> AppUnit {
        AppUnit {
            id: ws.clone(),
            provenance: Provenance {
                app: ws.clone(),
                tier: TrustTier::Workspace,
                content_hash: String::new(),
            },
            source: Some(SourceRoot {
                files: vec![],
                tier: TrustTier::Workspace,
                content_hash: String::new(),
            }),
            compilation: CompilationContext::default(),
            declared_deps: vec![],
            abi: None,
            app_path: None,
        }
    }

    fn make_symbolonly_dep_unit(dep: &AppId) -> AppUnit {
        AppUnit {
            id: dep.clone(),
            provenance: Provenance {
                app: dep.clone(),
                tier: TrustTier::SymbolOnly,
                content_hash: String::new(),
            },
            source: None,
            compilation: CompilationContext::default(),
            declared_deps: vec![],
            abi: None,
            app_path: None,
        }
    }

    #[test]
    fn symbolonly_dep_nodes_appear_in_graph() {
        let ws = ws_id();
        let dep = dep_id("DepPub");
        let cache = AbiCache::new();
        cache.seed(
            &dep.guid,
            &dep.name,
            &dep.publisher,
            &dep.version,
            Arc::new(make_dep_pub_abi()),
        );

        let snap = AppSetSnapshot {
            apps: vec![make_ws_unit(&ws), make_symbolonly_dep_unit(&dep)],
            workspace_app: ws,
            world: World::Closed,
        };
        let g = build_program_graph(&snap, &cache);

        let dep_obj = g
            .objects
            .iter()
            .find(|o| o.name.eq_ignore_ascii_case("Dep Pub"))
            .expect("Dep Pub ObjectNode must exist");
        assert_eq!(dep_obj.tier, TrustTier::SymbolOnly);
        assert_eq!(dep_obj.id.kind, ObjectKind::Codeunit);

        let on_dep_event = g
            .routines
            .iter()
            .find(|r| r.id.name_lc == "ondepevent")
            .expect("OnDepEvent must exist");
        assert_eq!(on_dep_event.id.params_count, 2);
        assert_eq!(
            on_dep_event.publisher_kind,
            Some(PublisherKind::Integration)
        );

        let do_dep_work = g
            .routines
            .iter()
            .find(|r| r.id.name_lc == "dodepwork")
            .expect("DoDepWork must exist");
        assert_eq!(do_dep_work.id.params_count, 1);

        let f_overloads: Vec<_> = g.routines.iter().filter(|r| r.id.name_lc == "f").collect();
        assert_eq!(
            f_overloads.len(),
            2,
            "Both F overloads must be distinct nodes"
        );
        assert_ne!(
            f_overloads[0].id.sig_fp, f_overloads[1].id.sig_fp,
            "F(Integer) and F(Text) must have different sig_fp"
        );
        assert!(f_overloads.iter().all(|r| r.id.params_count == 1));
    }

    #[test]
    fn workspace_only_snapshot_graph_unchanged() {
        let ws = ws_id();
        let snap = AppSetSnapshot {
            apps: vec![make_ws_unit(&ws)],
            workspace_app: ws,
            world: World::Closed,
        };
        let cache = AbiCache::new();
        let g = build_program_graph(&snap, &cache);
        assert!(g.objects.iter().all(|o| o.tier != TrustTier::SymbolOnly));
        assert!(g.routines.iter().all(|r| r.tier != TrustTier::SymbolOnly));
    }

    #[test]
    fn abi_parse_cached_across_build_cycles() {
        let ws = ws_id();
        let dep = dep_id("DepPub");
        let cache = AbiCache::new();
        cache.seed(
            &dep.guid,
            &dep.name,
            &dep.publisher,
            &dep.version,
            Arc::new(make_dep_pub_abi()),
        );

        let snap = AppSetSnapshot {
            apps: vec![make_ws_unit(&ws), make_symbolonly_dep_unit(&dep)],
            workspace_app: ws,
            world: World::Closed,
        };

        let _g1 = build_program_graph(&snap, &cache);
        let _g2 = build_program_graph(&snap, &cache);

        assert_eq!(
            cache.parse_count.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "Pre-seeded cache must not trigger any file reads"
        );
    }

    #[test]
    fn local_and_internal_routines_skipped() {
        let dep = dep_id("SkipTest");
        let abi = SymbolReferenceAbi {
            objects: vec![AbiObject {
                object_type: "Codeunit".into(),
                object_number: 99,
                name: "Skip Test".into(),
                routines: vec![
                    AbiRoutine {
                        name: "Public".into(),
                        kind: "procedure".into(),
                        event_kind: SrAbiEventKind::Unknown,
                        parameters: vec![],
                        return_type_text: None,
                        is_local: false,
                        is_internal: false,
                        attributes: vec![],
                        attributes_parsed: vec![],
                    },
                    AbiRoutine {
                        name: "LocalProc".into(),
                        kind: "procedure".into(),
                        event_kind: SrAbiEventKind::Unknown,
                        parameters: vec![],
                        return_type_text: None,
                        is_local: true,
                        is_internal: false,
                        attributes: vec![],
                        attributes_parsed: vec![],
                    },
                    AbiRoutine {
                        name: "InternalProc".into(),
                        kind: "procedure".into(),
                        event_kind: SrAbiEventKind::Unknown,
                        parameters: vec![],
                        return_type_text: None,
                        is_local: false,
                        is_internal: true,
                        attributes: vec![],
                        attributes_parsed: vec![],
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };

        let ws = ws_id();
        let cache = AbiCache::new();
        cache.seed(
            &dep.guid,
            &dep.name,
            &dep.publisher,
            &dep.version,
            Arc::new(abi),
        );

        let snap = AppSetSnapshot {
            apps: vec![make_ws_unit(&ws), make_symbolonly_dep_unit(&dep)],
            workspace_app: ws,
            world: World::Closed,
        };
        let g = build_program_graph(&snap, &cache);

        let routines_in_skip: Vec<_> = g
            .routines
            .iter()
            .filter(|r| r.id.object.id_equals_number(99))
            .collect();

        assert!(
            routines_in_skip.iter().any(|r| r.id.name_lc == "public"),
            "Public must be included"
        );
        assert!(
            routines_in_skip.iter().all(|r| r.id.name_lc != "localproc"),
            "is_local must be skipped"
        );
        assert!(
            routines_in_skip
                .iter()
                .all(|r| r.id.name_lc != "internalproc"),
            "is_internal must be skipped"
        );
    }
}
