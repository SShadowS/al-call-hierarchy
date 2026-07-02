//! Cached ABI ingestion: parse SymbolOnly dep .app packages into graph nodes.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::app_package::open_app_zip;
use crate::engine::deps::symbol_reference::{
    AbiEventKind as SrAbiEventKind, AbiRoutine, SymbolReferenceAbi, parse_symbol_reference,
};
use crate::engine::l3::al_attributes::{AttributeInfo, bool_arg, find_attribute};
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

/// Append `s`'s byte length (decimal) then a `:` separator then `s` itself —
/// a netstring-style LENGTH-DELIMITED encoding (Task 2 round-2 addendum:
/// "typed, length-delimited canonical tuple"). Concatenating multiple
/// variable-length fields naively (e.g. a plain `|`-join) lets one field's
/// content masquerade as an adjacent field's boundary — a Subtype raw name
/// crafted to contain a `|` (or a decimal id) could otherwise collide with a
/// differently-shaped tuple. Prefixing every field with its own length makes
/// the encoding injective per-field regardless of what bytes the field
/// itself contains.
fn write_len_prefixed(buf: &mut String, s: &str) {
    buf.push_str(&s.len().to_string());
    buf.push(':');
    buf.push_str(s);
}

/// Fold one parameter's canonical discriminator tuple — `type_text` (the
/// bare-fallback SOURCE-SHAPED text) + `subtype_id` + `subtype_raw_name` +
/// `subtype_tag` (Task 2 round-2 addendum: "outer kind + subtype id + raw
/// subtype name + a degradation tag") — into `buf` as four length-delimited
/// fields. See [`AbiParameter::subtype_id`]'s doc for why this is necessary
/// even though `type_text` alone often already carries full fidelity: on the
/// FAIL-CLOSED DECLINE shapes (Id-only Subtype; a Subtype Name containing a
/// `"`), `type_text` degrades to the bare outer keyword ALONE — two
/// genuinely different declarations can share that degraded text — so the
/// raw discriminator fields are folded in ADDITIONALLY (never as a
/// substitute for `type_text`) to keep them from silently colliding.
fn fold_param_discriminator(
    buf: &mut String,
    p: &crate::engine::deps::symbol_reference::AbiParameter,
) {
    write_len_prefixed(buf, &p.type_text.to_ascii_lowercase());
    write_len_prefixed(
        buf,
        &p.subtype_id.map(|id| id.to_string()).unwrap_or_default(),
    );
    write_len_prefixed(
        buf,
        &p.subtype_raw_name
            .as_deref()
            .unwrap_or("")
            .to_ascii_lowercase(),
    );
    write_len_prefixed(buf, p.subtype_tag);
}

/// The ABI overload dedup fingerprint (`RoutineNodeId::sig_fp`) — a
/// length-delimited fold of every parameter's canonical discriminator tuple
/// (Task 2 round-2 addendum), using the project's STABLE fingerprint
/// primitive ([`fnv1a`] — never `DefaultHasher`/a process-random hasher,
/// which would make `sig_fp` non-reproducible across runs and silently break
/// every consumer that persists or compares it within one run).
///
/// Prior to Task 2 this folded ONLY `type_text.to_ascii_lowercase()`
/// `|`-joined — degrading a parameter's type to its OUTER keyword alone
/// (never a `Subtype`), so two genuinely DIFFERENT same-arity ABI overloads
/// differing only by an object-typed parameter's Subtype (`Get(X: Codeunit
/// A)` vs `Get(X: Codeunit B)`) silently fingerprint-collided. Two
/// parameters now fingerprint identically ONLY when their ENTIRE canonical
/// tuple (text + subtype id + raw subtype name + degradation tag) matches —
/// see [`fold_param_discriminator`]'s doc for why all four fields are
/// necessary, not just `type_text`.
pub(crate) fn param_type_fp(params: &[crate::engine::deps::symbol_reference::AbiParameter]) -> u64 {
    if params.is_empty() {
        return 0;
    }
    let mut canon = String::new();
    for p in params {
        fold_param_discriminator(&mut canon, p);
    }
    fnv1a(&canon)
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

/// ABI-tier counterpart of
/// `crate::program::resolve::event::publisher_include_sender` — reads
/// `IncludeSender` (arg index 0) from an ABI routine's raw
/// `[IntegrationEvent]` / `[BusinessEvent]` / `[InternalEvent]` attribute
/// (all three carry it at position 0; see that function's doc for the
/// verified Microsoft Learn signatures). A real dependency probe (Microsoft
/// Base Application `SymbolReference.json`, 13,581 publisher-attribute
/// occurrences across `Codeunits` + every nested `Namespaces[]` level) found
/// 100% coverage — `Arguments[0].Value` was present and parsed to a literal
/// `"True"`/`"False"` on every single entry, zero unparseable — so, like the
/// source path, this is expected to be `Some` in practice. `None` remains
/// the fail-closed contract for the (unobserved) case the JSON's attribute
/// argument is absent or not a recognizable boolean.
pub(crate) fn abi_publisher_include_sender(attrs: &[AttributeInfo]) -> Option<bool> {
    for name in ["IntegrationEvent", "BusinessEvent", "InternalEvent"] {
        if let Some(attr) = find_attribute(attrs, name) {
            return bool_arg(attr, 0);
        }
    }
    None
}

/// Sentinel `RoutineNodeId.params_count` for an ABI routine whose `Parameters`
/// field was absent/unparseable in `SymbolReference.json` (`AbiRoutine::
/// parameters_known == false`) — as opposed to a genuinely 0-arg procedure,
/// which carries an explicit empty array and a KNOWN count of `0`.
///
/// Arity is TRI-STATE (Task 1, round-2 hardening): known-n / known-zero /
/// unknown. `usize::MAX` can never equal a real call site's argument count
/// (`args.len()` is always small — AL procedures cap well under a hundred
/// params), so a candidate carrying this sentinel structurally NEVER
/// arity-matches any real call. `resolve_in_object`'s `rid.params_count ==
/// arity` filter therefore excludes it by construction — an unknown-arity
/// candidate never emits an edge — with zero special-casing needed in
/// `resolver.rs`. It still counts toward the NAME-ONLY existence scan
/// (`object_has_visible_member_candidate`'s SymbolOnly branch, which is
/// arity-deferred by design), matching the tri-state contract: "exists" is
/// knowable even when "matches this arity" is not.
pub(crate) const UNKNOWN_ARITY: usize = usize::MAX;

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
            // ABI/SymbolOnly ingestion does not (yet) project SourceTable/TableNo/
            // page-control data from the dependency symbol reference — additive gap,
            // not a regression (Task 4 scope is the source `extract_nodes` path).
            source_table: None,
            table_no: None,
            source_table_temporary: false,
            page_controls: vec![],
        });

        for routine in &abi_obj.routines {
            if routine.is_local || routine.is_internal {
                continue;
            }
            let name_lc = routine.name.to_ascii_lowercase();
            // Tri-state arity (Task 1): a genuinely-parsed `Parameters` array
            // (even empty) carries its real `len()`; an absent/unparseable one
            // maps to `UNKNOWN_ARITY`, a sentinel that can never arity-match a
            // real call site — see the constant's doc for the full contract.
            let params_count = if routine.parameters_known {
                routine.parameters.len()
            } else {
                UNKNOWN_ARITY
            };
            let sig_fp = param_type_fp(&routine.parameters);
            let (routine_kind, event_kind, publisher_kind) = abi_routine_kind_from_str(routine);
            let include_sender = abi_publisher_include_sender(&routine.attributes_parsed);

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
                // `protected` ABI members are KEPT (unlike local/internal,
                // dropped above) and carried as `Access::Protected` — an
                // extension of the declaring object may call them (Task 1).
                access: if routine.is_protected {
                    Access::Protected
                } else {
                    Access::Public
                },
                tier: TrustTier::SymbolOnly,
                event_subscribers: vec![],
                subscriber_instance_manual: false,
                publisher_kind,
                include_sender,
                abi_routine_kind: Some(routine_kind),
                abi_event_kind: Some(event_kind),
                // Hardcoded empty — STILL correct post-Task-2, for a DIFFERENT
                // reason than a source routine's real `param_sig_key`. Task 2
                // made `param_type_fp` (hence `rid.sig_fp` above) carry full
                // Subtype fidelity (bare-outer-name fallback + the raw
                // discriminator fold — see `param_type_fp`/
                // `fold_param_discriminator`'s docs), so two genuinely
                // DIFFERENT ABI overloads almost always land on DIFFERENT
                // `sig_fp`s now and never even reach the same `RoutineNodeId`
                // run in `build::dedup_routines_preserving_genuine_overloads`
                // — `param_sig_key` never needs to distinguish them THERE.
                // Within a run that DOES share one `sig_fp` (i.e. every raw
                // entry's ENTIRE canonical discriminator tuple matched), ABI
                // ingestion still has no INDEPENDENT per-overload content
                // signature beyond that tuple to further distinguish them —
                // unlike source params, whose real parsed type text backs
                // `param_sig_key` below — so leaving this empty is safe:
                // every entry in such a run is content-INDISTINGUISHABLE by
                // our discriminator, whether that's because it's a literal
                // re-parse duplicate OR a genuinely-different-but-
                // fingerprint-identical pair (a residual collision the
                // round-2 addendum calls out — "any residual same-key
                // multi-entry group is collapse-marked so collisions
                // OVER-DECLINE, never select"). The safety net is downstream:
                // `dedup_routines_preserving_genuine_overloads` marks that
                // survivor `abi_overload_collapsed` whenever ≥2 raw
                // `SymbolOnly` entries shared a node id, so a later
                // type-query OR plain dispatch (`resolver::resolve_in_object`,
                // Task 2's plain-dispatch marker guard) declines rather than
                // trusts a possibly-wrong candidate.
                param_sig_key: String::new(),
                // Task 2: the reconstructed SOURCE-SHAPED return-type text
                // (see `symbol_reference::reconstruct_return_type_text`'s
                // fail-closed rules) now flows through instead of being
                // hard-discarded — resolution-neutral until Task 3 adds a
                // consumer (nothing reads `RoutineNode.return_type` for an
                // ABI-tier routine yet).
                return_type: routine.return_type_text.clone(),
                // The structured `(name, id)` cross-validation pair, carried
                // alongside the text so Task 3 can reach it via the SAME
                // `RoutineNodeId` lookup regardless of route shape (`AbiSymbol`
                // or `Routine(rid)`) — see `AbiRoutine::return_type_id`'s doc.
                return_type_id: routine.return_type_id.clone(),
                // Never marked here: ingestion emits one `RoutineNode` per RAW
                // ABI routine, with no folding yet — the actual collapse (and
                // thus the only place that can know ≥2 raw entries shared a
                // node id) happens later, once every app's routines are
                // pooled and sorted, in `build::
                // dedup_routines_preserving_genuine_overloads` (Task 3 review
                // fix). See `RoutineNode::abi_overload_collapsed`'s doc.
                abi_overload_collapsed: false,
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
                                subtype_id: None,
                                subtype_raw_name: None,
                                subtype_tag: "no_subtype",
                            },
                            AbiParameter {
                                name: "p2".into(),
                                type_text: "Text".into(),
                                is_var: false,
                                is_temporary: false,
                                subtype_id: None,
                                subtype_raw_name: None,
                                subtype_tag: "no_subtype",
                            },
                        ],
                        return_type_text: None,
                        return_type_id: None,
                        is_local: false,
                        is_internal: false,
                        is_protected: false,
                        parameters_known: true,
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
                            subtype_id: None,
                            subtype_raw_name: None,
                            subtype_tag: "no_subtype",
                        }],
                        return_type_text: None,
                        return_type_id: None,
                        is_local: false,
                        is_internal: false,
                        is_protected: false,
                        parameters_known: true,
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
                            subtype_id: None,
                            subtype_raw_name: None,
                            subtype_tag: "no_subtype",
                        }],
                        return_type_text: None,
                        return_type_id: None,
                        is_local: false,
                        is_internal: false,
                        is_protected: false,
                        parameters_known: true,
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
                            subtype_id: None,
                            subtype_raw_name: None,
                            subtype_tag: "no_subtype",
                        }],
                        return_type_text: None,
                        return_type_id: None,
                        is_local: false,
                        is_internal: false,
                        is_protected: false,
                        parameters_known: true,
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
            internals_visible_to: vec![],
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
            internals_visible_to: vec![],
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
                        return_type_id: None,
                        is_local: false,
                        is_internal: false,
                        is_protected: false,
                        parameters_known: true,
                        attributes: vec![],
                        attributes_parsed: vec![],
                    },
                    AbiRoutine {
                        name: "LocalProc".into(),
                        kind: "procedure".into(),
                        event_kind: SrAbiEventKind::Unknown,
                        parameters: vec![],
                        return_type_text: None,
                        return_type_id: None,
                        is_local: true,
                        is_internal: false,
                        is_protected: false,
                        parameters_known: true,
                        attributes: vec![],
                        attributes_parsed: vec![],
                    },
                    AbiRoutine {
                        name: "InternalProc".into(),
                        kind: "procedure".into(),
                        event_kind: SrAbiEventKind::Unknown,
                        parameters: vec![],
                        return_type_text: None,
                        return_type_id: None,
                        is_local: false,
                        is_internal: true,
                        is_protected: false,
                        parameters_known: true,
                        attributes: vec![],
                        attributes_parsed: vec![],
                    },
                    // Task 1: `protected` is NOT dropped like local/internal —
                    // it is KEPT and carried as `Access::Protected` (an
                    // extension of the declaring object may call it).
                    AbiRoutine {
                        name: "ProtectedProc".into(),
                        kind: "procedure".into(),
                        event_kind: SrAbiEventKind::Unknown,
                        parameters: vec![],
                        return_type_text: None,
                        return_type_id: None,
                        is_local: false,
                        is_internal: false,
                        is_protected: true,
                        parameters_known: true,
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
        let protected_proc = routines_in_skip
            .iter()
            .find(|r| r.id.name_lc == "protectedproc")
            .expect("ProtectedProc must be KEPT (carried as Access::Protected, not dropped)");
        assert_eq!(
            protected_proc.access,
            Access::Protected,
            "IsProtected:true must carry Access::Protected, never Access::Public"
        );
        let public_proc = routines_in_skip
            .iter()
            .find(|r| r.id.name_lc == "public")
            .expect("Public must exist");
        assert_eq!(
            public_proc.access,
            Access::Public,
            "a routine with neither IsLocal/IsInternal/IsProtected must default to Access::Public"
        );
    }

    /// Task 1 (round-2 tri-state arity hardening): an ABI routine whose
    /// `Parameters` field was absent/unparseable (`parameters_known: false`)
    /// must be ingested with the `UNKNOWN_ARITY` sentinel, never a false `0` —
    /// a real 0-arg procedure (explicit `Parameters: []`) is KNOWN-zero and
    /// must stay distinguishable in principle (both are ingested here to prove
    /// the sentinel differs from a real 0).
    #[test]
    fn unknown_arity_routine_gets_sentinel_params_count() {
        let dep = dep_id("ArityTest");
        let abi = SymbolReferenceAbi {
            objects: vec![AbiObject {
                object_type: "Codeunit".into(),
                object_number: 199,
                name: "Arity Test".into(),
                routines: vec![
                    AbiRoutine {
                        name: "KnownZero".into(),
                        kind: "procedure".into(),
                        event_kind: SrAbiEventKind::Unknown,
                        parameters: vec![],
                        return_type_text: None,
                        return_type_id: None,
                        is_local: false,
                        is_internal: false,
                        is_protected: false,
                        parameters_known: true,
                        attributes: vec![],
                        attributes_parsed: vec![],
                    },
                    AbiRoutine {
                        name: "UnknownArity".into(),
                        kind: "procedure".into(),
                        event_kind: SrAbiEventKind::Unknown,
                        parameters: vec![],
                        return_type_text: None,
                        return_type_id: None,
                        is_local: false,
                        is_internal: false,
                        is_protected: false,
                        parameters_known: false,
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

        let routines_in_arity: Vec<_> = g
            .routines
            .iter()
            .filter(|r| r.id.object.id_equals_number(199))
            .collect();

        let known_zero = routines_in_arity
            .iter()
            .find(|r| r.id.name_lc == "knownzero")
            .expect("KnownZero must exist");
        assert_eq!(
            known_zero.id.params_count, 0,
            "an explicit empty Parameters array is a KNOWN 0-arity"
        );

        let unknown = routines_in_arity
            .iter()
            .find(|r| r.id.name_lc == "unknownarity")
            .expect("UnknownArity must exist");
        assert_eq!(
            unknown.id.params_count, UNKNOWN_ARITY,
            "an absent Parameters field must map to the UNKNOWN_ARITY sentinel, never 0"
        );
        assert_ne!(
            unknown.id.params_count, known_zero.id.params_count,
            "unknown arity must never collide with a real known-zero arity"
        );
    }

    /// Task 2 invariant (b, ABI half — FLIPPED from the pre-Task-2 discard):
    /// an ABI routine's `RoutineNode.return_type` is now POPULATED from the
    /// underlying `AbiRoutine.return_type_text` (the source-shaped
    /// reconstruction — see `symbol_reference::reconstruct_return_type_text`)
    /// instead of being hard-dropped to `None`. The structured `(name, id)`
    /// cross-validation pair (`AbiRoutine::return_type_id`) is threaded onto
    /// `RoutineNode.return_type_id` alongside it, reachable via the same
    /// `RoutineNodeId` lookup regardless of which `RouteTarget` shape a
    /// consumer (Task 3) ends up resolving through.
    #[test]
    fn abi_routine_return_type_is_populated() {
        let dep = dep_id("RetTypeTest");
        let abi = SymbolReferenceAbi {
            objects: vec![AbiObject {
                object_type: "Codeunit".into(),
                object_number: 99,
                name: "Ret Type Test".into(),
                routines: vec![AbiRoutine {
                    name: "GetHelper".into(),
                    kind: "procedure".into(),
                    event_kind: SrAbiEventKind::Unknown,
                    parameters: vec![],
                    return_type_text: Some("Codeunit \"Helper\"".into()),
                    return_type_id: Some(("Helper".into(), 2354)),
                    is_local: false,
                    is_internal: false,
                    is_protected: false,
                    parameters_known: true,
                    attributes: vec![],
                    attributes_parsed: vec![],
                }],
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

        let get_helper = g
            .routines
            .iter()
            .find(|r| r.id.name_lc == "gethelper")
            .expect("GetHelper must exist");
        assert_eq!(
            get_helper.return_type.as_deref(),
            Some("Codeunit \"Helper\""),
            "an ABI routine's reconstructed return-type text must now flow through \
             to RoutineNode.return_type (Task 2 flips the prior deliberate discard)"
        );
        assert_eq!(
            get_helper.return_type_id,
            Some(("Helper".to_string(), 2354)),
            "the structured (name, id) cross-validation pair must also be carried \
             onto the graph-level RoutineNode, reachable by RoutineNodeId lookup \
             for Task 3's cross-object chain cross-validation"
        );
    }

    // -------------------------------------------------------------------
    // Task 2 round-2 addendum: `param_type_fp`'s discriminator-bearing fold
    // -------------------------------------------------------------------

    fn abi_param(
        type_text: &str,
        subtype_id: Option<i64>,
        subtype_raw_name: Option<&str>,
        subtype_tag: &'static str,
    ) -> AbiParameter {
        AbiParameter {
            name: "x".into(),
            type_text: type_text.into(),
            is_var: false,
            is_temporary: false,
            subtype_id,
            subtype_raw_name: subtype_raw_name.map(String::from),
            subtype_tag,
        }
    }

    /// (e) TRUE DUPLICATE: two parameters with an IDENTICAL canonical tuple
    /// (same text, same raw id, same raw name, same tag) must still
    /// fingerprint IDENTICALLY — this is the population
    /// `dedup_routines_preserving_genuine_overloads` correctly collapses
    /// (and marks `abi_overload_collapsed`, see `build.rs`'s existing
    /// `abi_sig_fp_collision_marks_survivor_collapsed`).
    #[test]
    fn param_type_fp_identical_discriminator_tuples_collide() {
        let p1 = vec![abi_param(
            "Codeunit \"Dep A\"",
            Some(60130),
            Some("Dep A"),
            "full",
        )];
        let p2 = vec![abi_param(
            "Codeunit \"Dep A\"",
            Some(60130),
            Some("Dep A"),
            "full",
        )];
        assert_eq!(param_type_fp(&p1), param_type_fp(&p2));
    }

    /// (b) round-1 critical sliver at the fp layer: two Id-only Subtypes
    /// sharing the IDENTICAL bare-fallback `type_text` ("Codeunit") but
    /// DIFFERENT raw ids must fingerprint DIFFERENTLY — this is what lets
    /// `DoIt(Codeunit 10)`/`DoIt(Codeunit 20)` survive as two distinct
    /// `RoutineNodeId`s instead of silently colliding.
    #[test]
    fn param_type_fp_different_id_only_subtypes_never_collide() {
        let p10 = vec![abi_param("Codeunit", Some(10), None, "id_only")];
        let p20 = vec![abi_param("Codeunit", Some(20), None, "id_only")];
        assert_ne!(param_type_fp(&p10), param_type_fp(&p20));
    }

    /// Sibling of the above at the fp layer: two quote-bearing Subtype Names
    /// sharing the IDENTICAL bare-fallback text and the SAME raw id must
    /// still fingerprint differently via the raw NAME discriminator.
    #[test]
    fn param_type_fp_different_quoted_names_never_collide() {
        let pa = vec![abi_param(
            "Codeunit",
            Some(1),
            Some("Weird\"NameA"),
            "name_quoted",
        )];
        let pb = vec![abi_param(
            "Codeunit",
            Some(1),
            Some("Weird\"NameB"),
            "name_quoted",
        )];
        assert_ne!(param_type_fp(&pa), param_type_fp(&pb));
    }

    /// Control: a genuinely scalar/no-Subtype parameter never collides with
    /// a degraded object-typed parameter that happens to share the SAME
    /// outer keyword text purely by coincidence — the degradation TAG (the
    /// canonical tuple's fourth component) keeps `"no_subtype"` distinct
    /// from `"empty_subtype"` even when id/name are both `None` in either.
    #[test]
    fn param_type_fp_no_subtype_vs_empty_subtype_tag_distinguishes() {
        let no_subtype = vec![abi_param("Codeunit", None, None, "no_subtype")];
        let empty_subtype = vec![abi_param("Codeunit", None, None, "empty_subtype")];
        assert_ne!(param_type_fp(&no_subtype), param_type_fp(&empty_subtype));
    }

    /// Full-fidelity case: two DIFFERENT named subtypes (the common,
    /// non-degraded shape) fingerprint differently — the ordinary case this
    /// primitive must never regress.
    #[test]
    fn param_type_fp_different_full_subtypes_never_collide() {
        let dep_a = vec![abi_param(
            "Codeunit \"Dep A\"",
            Some(60130),
            Some("Dep A"),
            "full",
        )];
        let dep_c = vec![abi_param(
            "Codeunit \"Dep C\"",
            Some(60140),
            Some("Dep C"),
            "full",
        )];
        assert_ne!(param_type_fp(&dep_a), param_type_fp(&dep_c));
    }
}
