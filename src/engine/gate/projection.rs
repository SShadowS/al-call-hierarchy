//! `project_finding` → `FindingSummary` — port of al-sem
//! `src/projection/finding-summary.ts`.
//!
//! Resolves an internal `Finding`'s `SourceAnchor.enclosing_routine_id` to display
//! names (routine + owning object) via the resolved model, and converts the 0-based
//! internal range to the 1-based SARIF/display line+column. The gate's SARIF + filter
//! layers consume the projected `FindingSummary`, exactly as al-sem does.

use std::collections::HashMap;

use crate::engine::l3::l3_workspace::{L3Object, L3Routine};
use crate::engine::l5::finding::{Finding, SourceAnchor};

/// `FindingLocation` (finding-summary.ts). `line`/`column` are 1-based.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingLocation {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub object_id: Option<String>,
    pub object_name: Option<String>,
    pub routine_id: Option<String>,
    pub routine_name: Option<String>,
}

/// `FindingSummary` (finding-summary.ts) — the compact projection every output path
/// consumes. The SARIF formatter reads `detector`, `title`, `root_cause`, `severity`,
/// `fingerprint`, and `primary_location`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingSummary {
    pub id: String,
    pub fingerprint: String,
    pub detector: String,
    pub title: String,
    pub root_cause: String,
    pub severity: String,
    pub confidence_level: String,
    pub confidence_capped_by: Option<Vec<String>>,
    pub primary_location: FindingLocation,
    pub terminal_location: Option<FindingLocation>,
    pub affected_objects: Vec<String>,
    pub affected_tables: Vec<String>,
    /// `fixHint` (finding-summary.ts) — the first `fixOptions` entry, if any.
    pub fix_hint: Option<(String, String)>,
    pub path_count: usize,
}

/// Per-model id indexes for the projection (mirror `indexFor`).
pub struct ProjectionIndex<'a> {
    pub objects_by_id: HashMap<&'a str, &'a L3Object>,
    pub routines_by_id: HashMap<&'a str, &'a L3Routine>,
}

impl<'a> ProjectionIndex<'a> {
    pub fn build(objects: &'a [L3Object], routines: &'a [L3Routine]) -> Self {
        ProjectionIndex {
            objects_by_id: objects.iter().map(|o| (o.id.as_str(), o)).collect(),
            routines_by_id: routines.iter().map(|r| (r.id.as_str(), r)).collect(),
        }
    }
}

/// `toLocation(anchor)` — resolve an internal anchor to a 1-based display location
/// with the owning routine + object display names.
fn to_location(anchor: &SourceAnchor, idx: &ProjectionIndex) -> FindingLocation {
    let routine = idx.routines_by_id.get(anchor.enclosing_routine_id.as_str());
    // Object-level finding convention (d64): `enclosing_routine_id` IS the
    // object's own internal id when there is no routine to anchor on (e.g. a
    // declarative page with no routines at all). Fall back to a direct object
    // lookup so `object_id`/`object_name` still resolve in the production
    // SARIF/JSON/HTML/terminal output; `routine_id`/`routine_name` correctly
    // stay `None` below (there is no routine). Behavior-preserving for every
    // routine-anchored finding: the `or_else` only runs when the routine
    // branch already missed.
    let object = routine
        .and_then(|r| idx.objects_by_id.get(r.object_id.as_str()))
        .or_else(|| idx.objects_by_id.get(anchor.enclosing_routine_id.as_str()));
    FindingLocation {
        file: anchor.source_unit_id.clone(),
        line: anchor.start_line + 1,
        column: anchor.start_column + 1,
        object_id: object.map(|o| o.id.clone()),
        object_name: object.map(|o| o.name.clone()),
        routine_id: routine.map(|r| r.id.clone()),
        routine_name: routine.map(|r| r.name.clone()),
    }
}

/// `projectFinding(finding, model)` — project a `Finding` into a `FindingSummary`.
/// When `actionable_anchor` is set, the primary becomes that anchor and the original
/// `primary_location` becomes the terminal (mirrors finding-summary.ts).
pub fn project_finding(finding: &Finding, idx: &ProjectionIndex) -> FindingSummary {
    let (primary, terminal) = match &finding.actionable_anchor {
        Some(actionable) => (
            to_location(actionable, idx),
            Some(to_location(&finding.primary_location, idx)),
        ),
        None => (to_location(&finding.primary_location, idx), None),
    };

    FindingSummary {
        id: finding.id.clone(),
        // al-sem: `finding.fingerprint ?? finding.id`.
        fingerprint: finding
            .fingerprint
            .clone()
            .unwrap_or_else(|| finding.id.clone()),
        detector: finding.detector.clone(),
        title: finding.title.clone(),
        root_cause: finding.root_cause.clone(),
        severity: finding.severity.clone(),
        confidence_level: finding.confidence.level.clone(),
        confidence_capped_by: finding.confidence.capped_by.clone(),
        primary_location: primary,
        terminal_location: terminal,
        affected_objects: finding.affected_objects.clone(),
        affected_tables: finding.affected_tables.clone(),
        // al-sem: `finding.fixOptions[0]` (the first fix option, if any).
        fix_hint: finding
            .fix_options
            .first()
            .map(|f| (f.description.clone(), f.safety.clone())),
        // al-sem: `1 + (finding.additionalPaths?.length ?? 0)`.
        path_count: 1 + finding
            .additional_paths
            .as_ref()
            .map(|p| p.len())
            .unwrap_or(0),
    }
}
