//! Root-classification substrate — Rust port of al-sem's §4.3 root-classifier.
//!
//! Ports (byte-parity oracle: `U:\Git\al-sem`):
//!   - `src/model/root-classification.ts` — [`RootKind`], [`ROOT_KIND_VALUES`],
//!     [`is_externally_reachable_kind`], [`RootClassification`].
//!   - `src/engine/root-classifier.ts` — [`classify_roots`] (AST-only pass).
//!   - `src/config/roots-config.ts` — [`load_roots_config`] (file → validated config).
//!   - `src/engine/root-classifier-overlay.ts` — [`overlay_config_roots`] (config merge).
//!
//! Determinism (R4-F spec Rev 2):
//!   - `kinds` are ordered by ROOT_KIND declaration order, NOT alphabetical —
//!     reproduced via `ROOT_KIND_VALUES.iter().filter(...)`.
//!   - No HashMap/HashSet iteration leaks into output: an accumulator
//!     [`std::collections::BTreeMap`] keyed by internal RoutineId backs the
//!     overlay; the AST pass sorts its `Vec` by RoutineId (ASCII ordinal).
//!   - The internal `RoutineId` is ASCII slash-form, so `<` ordering on the
//!     `String` matches al-sem's `a < b` on its `RoutineId` string.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::engine::l3::l3_workspace::{L3Object, L3Routine, L3Workspace};

// ---------------------------------------------------------------------------
// RootKind — the 12-value union, declaration order is ROOT_KIND_VALUES.
// ---------------------------------------------------------------------------

/// Canonical RootKind values in declaration order (al-sem `ROOT_KIND_VALUES`).
/// The single source of truth for valid kinds + the canonical sort order.
pub const ROOT_KIND_VALUES: [&str; 12] = [
    "trigger-table",
    "trigger-page",
    "page-action",
    "report-trigger",
    "event-subscriber",
    "install-codeunit",
    "upgrade-codeunit",
    "api-page",
    "web-service-exposed",
    "job-queue-entrypoint",
    "public-procedure",
    "test-procedure",
];

/// All current kinds are externally reachable (al-sem `isExternallyReachableKind`).
fn is_externally_reachable_kind(kind: &str) -> bool {
    ROOT_KIND_VALUES.contains(&kind)
}

/// Canonicalize a kind set to the documented invariant: deduped + sorted in
/// ROOT_KIND declaration order. Mirrors `ROOT_KIND_ORDER.filter(k => set.has(k))`.
fn canonical_kinds(set: &BTreeSet<String>) -> Vec<String> {
    ROOT_KIND_VALUES
        .iter()
        .filter(|k| set.contains(**k))
        .map(|k| k.to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// RootClassification — the per-routine output entry.
// ---------------------------------------------------------------------------

/// Per-routine classification with full provenance (al-sem `RootClassification`).
/// `sourceAnchor` is intentionally NOT carried here — the R4-F stable projection
/// omits it, and no Rust consumer (d50/d51 lookup by routine id) needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootClassification {
    /// Internal RoutineId (`${modelInstanceId}/${hash}`).
    pub routine_id: String,
    pub kinds: Vec<String>,
    pub externally_reachable: bool,
    /// "ast" | "config" | "ast+config".
    pub source: String,
    /// "static" | "user-asserted".
    pub confidence: String,
    pub config_entry_id: Option<String>,
    /// "resolved" | "ambiguous" | "unresolved".
    pub resolution_status: Option<String>,
}

// ---------------------------------------------------------------------------
// AST classifier — classify_roots (root-classifier.ts).
// ---------------------------------------------------------------------------

/// Compute the set of RootKinds a routine qualifies for, purely from its
/// structural shape + the host object's declared metadata. Mirrors `kindsFor`.
fn kinds_for(routine: &L3Routine, object: &L3Object) -> Vec<String> {
    let mut set: BTreeSet<String> = BTreeSet::new();

    // Trigger kinds — gated on routine.kind === "trigger".
    if routine.kind == "trigger" {
        match object.object_type.as_str() {
            "Table" | "TableExtension" => {
                set.insert("trigger-table".to_string());
            }
            "Page" | "PageExtension" => {
                set.insert("trigger-page".to_string());
            }
            "Report" => {
                set.insert("report-trigger".to_string());
            }
            _ => {}
        }
    }

    // Event-subscriber — direct from routine.kind.
    if routine.kind == "event-subscriber" {
        set.insert("event-subscriber".to_string());
    }

    // Codeunit Subtype-based kinds (case-insensitive).
    if object.object_type == "Codeunit" {
        let subtype = object.object_subtype.as_deref().map(|s| s.to_lowercase());
        if subtype.as_deref() == Some("install") {
            set.insert("install-codeunit".to_string());
        }
        if subtype.as_deref() == Some("upgrade") {
            set.insert("upgrade-codeunit".to_string());
        }
    }

    // Page with PageType=API (case-insensitive) — every routine is HTTP-exposed.
    if (object.object_type == "Page" || object.object_type == "PageExtension")
        && object
            .page_type
            .as_deref()
            .map(|p| p.to_lowercase())
            .as_deref()
            == Some("api")
    {
        set.insert("api-page".to_string());
    }

    // Test procedures — via [Test] attribute on the routine itself.
    if routine
        .attributes_parsed
        .iter()
        .any(|a| a.name.to_lowercase() == "test")
    {
        set.insert("test-procedure".to_string());
    }

    // Public procedures — non-trigger, non-event-subscriber procedures with
    // default access (None accessModifier). Catch-all: only when nothing more
    // specific applied (al-sem checks `kinds.length === 0` BEFORE this push).
    if routine.kind == "procedure" && routine.access_modifier.is_none() && set.is_empty() {
        set.insert("public-procedure".to_string());
    }

    canonical_kinds(&set)
}

/// AST-only root classifier (al-sem `classifyRoots`). Produces a
/// `RootClassification` for every routine that qualifies as >=1 RootKind, sorted
/// by internal RoutineId ascending. Routines whose object is missing are skipped.
pub fn classify_roots(workspace: &L3Workspace) -> Vec<RootClassification> {
    let objects_by_id: BTreeMap<&str, &L3Object> = workspace
        .objects
        .iter()
        .map(|o| (o.id.as_str(), o))
        .collect();

    let mut result: Vec<RootClassification> = Vec::new();
    for routine in &workspace.routines {
        let Some(object) = objects_by_id.get(routine.object_id.as_str()) else {
            continue;
        };
        let kinds = kinds_for(routine, object);
        if kinds.is_empty() {
            continue;
        }
        let externally_reachable = kinds.iter().any(|k| is_externally_reachable_kind(k));
        result.push(RootClassification {
            routine_id: routine.id.clone(),
            kinds,
            externally_reachable,
            source: "ast".to_string(),
            confidence: "static".to_string(),
            config_entry_id: None,
            resolution_status: None,
        });
    }

    // Canonical sort for determinism — RoutineId is an ASCII slash-form string.
    result.sort_by(|a, b| a.routine_id.cmp(&b.routine_id));
    result
}

// ---------------------------------------------------------------------------
// roots.config.json loader — load_roots_config (roots-config.ts).
// ---------------------------------------------------------------------------

/// A validated roots.config target (al-sem `RootsConfigTarget`).
#[derive(Debug, Clone)]
enum RootsConfigTarget {
    RoutineId(String),
    ObjectRoutine {
        object_id: String,
        routine_name: String,
    },
}

/// A validated roots.config entry (al-sem `RootsConfigEntry`) — only the fields
/// the overlay reads. Diagnostics are NOT part of the projection.
#[derive(Debug, Clone)]
struct RootsConfigEntry {
    id: String,
    target: RootsConfigTarget,
    /// Canonicalized (deduped + ROOT_KIND-ordered) kind list.
    kinds: Vec<String>,
    externally_reachable: Option<bool>,
}

/// A loaded + validated roots.config (al-sem `RootsConfig`). `None` ⇒ missing /
/// malformed at the top level (the overlay then passes AST roots through).
#[derive(Debug, Clone, Default)]
struct RootsConfig {
    roots: Vec<RootsConfigEntry>,
}

/// Parse a target object into one of two accepted shapes. `routineId` takes
/// precedence over `objectId + routineName`. Mirrors `parseTarget`.
fn parse_target(t: &serde_json::Value) -> Option<RootsConfigTarget> {
    let obj = t.as_object()?;
    if let Some(rid) = obj.get("routineId").and_then(|v| v.as_str()) {
        return Some(RootsConfigTarget::RoutineId(rid.to_string()));
    }
    let object_id = obj.get("objectId").and_then(|v| v.as_str());
    let routine_name = obj.get("routineName").and_then(|v| v.as_str());
    if let (Some(o), Some(n)) = (object_id, routine_name) {
        return Some(RootsConfigTarget::ObjectRoutine {
            object_id: o.to_string(),
            routine_name: n.to_string(),
        });
    }
    None
}

/// Validate a parsed JSON value into a `RootsConfig`. Faithfully ports the
/// ACCEPTANCE logic of `validateRootsConfig` (which entries survive + their
/// canonicalized kinds). Diagnostics text is intentionally not reproduced.
fn validate_roots_config(parsed: &serde_json::Value) -> Option<RootsConfig> {
    let obj = parsed.as_object()?;
    // version === 1. al-sem gates on JS `obj.version !== 1` (numeric), which accepts
    // `1`, `1.0`, and `1e0` alike — so compare numerically via `as_f64`, not `as_i64`
    // (the latter would reject the float spelling `1.0` and discard the whole config).
    if obj.get("version").and_then(|v| v.as_f64()) != Some(1.0) {
        return None;
    }
    let roots = obj.get("roots").and_then(|v| v.as_array())?;

    let valid_kinds: BTreeSet<&str> = ROOT_KIND_VALUES.iter().copied().collect();
    let mut entries: Vec<RootsConfigEntry> = Vec::new();
    let mut seen_ids: BTreeSet<String> = BTreeSet::new();

    for entry_v in roots {
        let Some(entry) = entry_v.as_object() else {
            continue;
        };
        let Some(id) = entry.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        if seen_ids.contains(id) {
            continue;
        }
        let Some(target) = entry.get("target").and_then(parse_target) else {
            continue;
        };
        let Some(kinds_arr) = entry.get("kinds").and_then(|v| v.as_array()) else {
            continue;
        };
        // Canonicalize: dedup (silent) + sort in ROOT_KIND_VALUES order.
        let mut kind_set: BTreeSet<String> = BTreeSet::new();
        for k in kinds_arr {
            if let Some(ks) = k.as_str() {
                if valid_kinds.contains(ks) {
                    kind_set.insert(ks.to_string());
                }
            }
        }
        let kinds = canonical_kinds(&kind_set);
        if kinds.is_empty() {
            continue;
        }

        // Optional externallyReachable: present-and-boolean → carry; else omit.
        let externally_reachable = match entry.get("externallyReachable") {
            Some(v) => v.as_bool(), // wrong-typed → None (dropped), matching al-sem.
            None => None,
        };

        entries.push(RootsConfigEntry {
            id: id.to_string(),
            target,
            kinds,
            externally_reachable,
        });
        seen_ids.insert(id.to_string());
    }

    Some(RootsConfig { roots: entries })
}

/// Load + validate `<workspaceRoot>/roots.config.json` (al-sem `loadRootsConfig`).
/// Missing file ⇒ `None` (clean empty, the common case). Parse / validation
/// failure ⇒ `None`. Never throws / panics.
fn load_roots_config(workspace_root: &Path) -> Option<RootsConfig> {
    let path = workspace_root.join("roots.config.json");
    let bytes = std::fs::read(&path).ok()?;
    // Strip a leading UTF-8 BOM (EF BB BF) before JSON parse — mirrors the
    // `charCodeAt(0) === 0xFEFF` slice in al-sem (here on the byte form).
    let slice: &[u8] = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        &bytes[..]
    };
    let text = std::str::from_utf8(slice).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(text).ok()?;
    validate_roots_config(&parsed)
}

// ---------------------------------------------------------------------------
// Overlay — overlay_config_roots (root-classifier-overlay.ts).
// ---------------------------------------------------------------------------

/// Resolve a config target to the matching routines (al-sem `resolveTarget`).
/// `routineId` → exact id match (0 or 1). `objectId + routineName` →
/// case-insensitive name match within the object (may be multiple).
fn resolve_target<'a>(
    target: &RootsConfigTarget,
    workspace: &'a L3Workspace,
) -> Vec<&'a L3Routine> {
    match target {
        RootsConfigTarget::RoutineId(rid) => workspace
            .routines
            .iter()
            .find(|r| &r.id == rid)
            .into_iter()
            .collect(),
        RootsConfigTarget::ObjectRoutine {
            object_id,
            routine_name,
        } => {
            let lc = routine_name.to_lowercase();
            workspace
                .routines
                .iter()
                .filter(|r| &r.object_id == object_id && r.name.to_lowercase() == lc)
                .collect()
        }
    }
}

/// Merge a `RootsConfig` overlay on top of the AST classification result
/// (al-sem `overlayConfigRoots`). `config` None ⇒ AST roots pass through.
///
/// Precedence discipline mirrors al-sem exactly: `ast_by_routine` is a FROZEN
/// snapshot of the AST baseline; `by_routine` is the accumulator (a `BTreeMap`
/// here for deterministic, hash-free iteration). A second config entry on the
/// same routine still unions against the ORIGINAL AST kinds, not entry-1's
/// merged result.
fn overlay_config_roots(
    ast_roots: Vec<RootClassification>,
    config: Option<&RootsConfig>,
    workspace: &L3Workspace,
) -> Vec<RootClassification> {
    let Some(config) = config else {
        return ast_roots;
    };

    let ast_by_routine: BTreeMap<String, RootClassification> = ast_roots
        .iter()
        .map(|r| (r.routine_id.clone(), r.clone()))
        .collect();
    let mut by_routine: BTreeMap<String, RootClassification> = ast_by_routine.clone();

    for entry in &config.roots {
        let mut matches = resolve_target(&entry.target, workspace);
        // Sort matches by internal id (ASCII ordinal), first wins.
        matches.sort_by(|a, b| a.id.cmp(&b.id));

        if matches.is_empty() {
            continue;
        }
        let ambiguous = matches.len() > 1;
        let winner = matches[0];
        let existing_ast = ast_by_routine.get(&winner.id);
        let cfg_kind_set: BTreeSet<String> = entry.kinds.iter().cloned().collect();

        let resolution_status = if ambiguous {
            "ambiguous".to_string()
        } else {
            "resolved".to_string()
        };

        match existing_ast {
            None => {
                // Config-only root: no AST signal, "user-asserted" confidence.
                let kinds = canonical_kinds(&cfg_kind_set);
                if kinds.is_empty() {
                    continue;
                }
                let externally_reachable = entry
                    .externally_reachable
                    .unwrap_or_else(|| kinds.iter().any(|k| is_externally_reachable_kind(k)));
                by_routine.insert(
                    winner.id.clone(),
                    RootClassification {
                        routine_id: winner.id.clone(),
                        kinds,
                        externally_reachable,
                        source: "config".to_string(),
                        confidence: "user-asserted".to_string(),
                        config_entry_id: Some(entry.id.clone()),
                        resolution_status: Some(resolution_status),
                    },
                );
            }
            Some(existing) => {
                // AST + config corroboration: union kinds, upgrade to "static".
                // Union against the ORIGINAL (frozen) AST kind set.
                let mut unioned: BTreeSet<String> = existing.kinds.iter().cloned().collect();
                unioned.extend(entry.kinds.iter().cloned());
                let kinds = canonical_kinds(&unioned);
                let externally_reachable = entry
                    .externally_reachable
                    .unwrap_or_else(|| kinds.iter().any(|k| is_externally_reachable_kind(k)));
                by_routine.insert(
                    winner.id.clone(),
                    RootClassification {
                        routine_id: winner.id.clone(),
                        kinds,
                        externally_reachable,
                        source: "ast+config".to_string(),
                        confidence: "static".to_string(),
                        config_entry_id: Some(entry.id.clone()),
                        resolution_status: Some(resolution_status),
                    },
                );
            }
        }
    }

    let mut roots: Vec<RootClassification> = by_routine.into_values().collect();
    roots.sort_by(|a, b| a.routine_id.cmp(&b.routine_id));
    roots
}

// ---------------------------------------------------------------------------
// Top-level entry — mirror src/index.ts ~lines 255-271.
// ---------------------------------------------------------------------------

/// Compute `model.rootClassifications`: classify the AST roots, load any
/// `<workspace>/roots.config.json`, then overlay the config on the AST roots.
/// `workspace_root` is `None` for the inline/cross-app paths that have no disk
/// config (⇒ AST-only). Mirrors al-sem's index.ts wiring.
pub fn compute_root_classifications(
    workspace: &L3Workspace,
    workspace_root: Option<&Path>,
) -> Vec<RootClassification> {
    let ast_roots = classify_roots(workspace);
    let config = workspace_root.and_then(load_roots_config);
    overlay_config_roots(ast_roots, config.as_ref(), workspace)
}

// ---------------------------------------------------------------------------
// R4-F stable projection — the differential surface (mirrors
// scripts/r4f-root-classification-projection.ts).
//
// Field order is LOAD-BEARING (must byte-match the al-sem golden's serde order):
//   - outer: fixtureName, classificationCount, classifications
//   - inner: routineId, kinds, externallyReachable, source, confidence,
//            [configEntryId], [resolutionStatus]
// `sourceAnchor` is OMITTED. Optionals use `skip_serializing_if`. Internal
// RoutineId is projected to StableRoutineId; entries with no stable mapping are
// skipped (mirrors the projection's empty-id exclusion). Sorted by stable
// routineId ascending.
// ---------------------------------------------------------------------------

/// One stable RootClassification — all ids in stable form.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StableRootClassification {
    #[serde(rename = "routineId")]
    pub routine_id: String,
    pub kinds: Vec<String>,
    #[serde(rename = "externallyReachable")]
    pub externally_reachable: bool,
    pub source: String,
    pub confidence: String,
    #[serde(rename = "configEntryId", skip_serializing_if = "Option::is_none")]
    pub config_entry_id: Option<String>,
    #[serde(rename = "resolutionStatus", skip_serializing_if = "Option::is_none")]
    pub resolution_status: Option<String>,
}

/// The full R4-F root-classification projection for one fixture run.
#[derive(Debug, Clone, serde::Serialize)]
pub struct R4FRootClassProjection {
    #[serde(rename = "fixtureName")]
    pub fixture_name: String,
    #[serde(rename = "classificationCount")]
    pub classification_count: usize,
    pub classifications: Vec<StableRootClassification>,
}

/// Project a resolved workspace's `root_classifications` to the stable R4-F form.
/// Mirrors `projectRootClassifications`: map each internal RoutineId to its
/// StableRoutineId via the routine stable-map; drop entries with no stable id;
/// sort by stable routineId ascending.
pub fn project_r4f_root_classifications(
    resolved: &crate::engine::l3::l3_workspace::L3Resolved,
    fixture_name: &str,
) -> R4FRootClassProjection {
    let map = crate::engine::l4::summary::build_routine_stable_map(&resolved.workspace.routines);

    let mut stable: Vec<StableRootClassification> = Vec::new();
    for rc in &resolved.root_classifications {
        let Some(stable_id) = map.get(&rc.routine_id) else {
            continue;
        };
        if stable_id.is_empty() {
            continue;
        }
        stable.push(StableRootClassification {
            routine_id: stable_id.clone(),
            kinds: rc.kinds.clone(),
            externally_reachable: rc.externally_reachable,
            source: rc.source.clone(),
            confidence: rc.confidence.clone(),
            config_entry_id: rc.config_entry_id.clone(),
            resolution_status: rc.resolution_status.clone(),
        });
    }

    stable.sort_by(|a, b| a.routine_id.cmp(&b.routine_id));

    R4FRootClassProjection {
        fixture_name: fixture_name.to_string(),
        classification_count: stable.len(),
        classifications: stable,
    }
}
