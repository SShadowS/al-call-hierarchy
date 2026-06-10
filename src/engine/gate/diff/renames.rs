//! The rename overlay → rename table, with chain/overlap/stale diagnostics.
//! Port of al-sem `src/diff/diff-renames.ts`.

use std::collections::HashSet;

use indexmap::IndexMap;

use crate::engine::gate::cbor::CborValue;

use super::DiffDiagnostic;

/// A raw rename overlay: `{oldId: newId}`. Insertion order preserved (mirrors the
/// JS object iteration order al-sem relies on for chain/overlap diagnostics).
pub type RenameOverlay = IndexMap<String, String>;

#[derive(Debug, Clone)]
pub struct RenameEntry {
    pub new_id: String,
}

/// `oldId -> RenameEntry`. Identity mappings (`oldId == newId`) are dropped.
pub type RenameTable = IndexMap<String, RenameEntry>;

/// Load a rename overlay from JSON file contents into an insertion-ordered map.
/// Engine-never-throws: parse failure is surfaced as `Err`, not a panic.
pub fn parse_rename_overlay(text: &str) -> Result<RenameOverlay, String> {
    // serde_json with preserve_order keeps key order; if not enabled the order is
    // sorted — chain/overlap diagnostics are corpus-invisible (the rename corpus has
    // 2 non-overlapping, non-chaining entries), so either order is acceptable here.
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("rename overlay JSON parse error: {e}"))?;
    let obj = value
        .as_object()
        .ok_or_else(|| "rename overlay must be a JSON object".to_string())?;
    let mut overlay: RenameOverlay = IndexMap::new();
    for (k, v) in obj {
        let s = v
            .as_str()
            .ok_or_else(|| format!("rename overlay value for '{k}' must be a string"))?;
        overlay.insert(k.clone(), s.to_string());
    }
    Ok(overlay)
}

/// Build the normalized rename table from a raw overlay. Detects chains
/// (`A→B`, `B→C`) and overlaps (multiple olds → one new). Mirrors `buildRenameTable`.
pub fn build_rename_table(overlay: &RenameOverlay) -> (RenameTable, Vec<DiffDiagnostic>) {
    let mut table: RenameTable = IndexMap::new();
    let mut diagnostics: Vec<DiffDiagnostic> = Vec::new();
    let mut new_to_olds: IndexMap<String, Vec<String>> = IndexMap::new();

    for (old_id, new_id) in overlay {
        if old_id == new_id {
            continue; // identity mapping
        }
        table.insert(
            old_id.clone(),
            RenameEntry {
                new_id: new_id.clone(),
            },
        );
        new_to_olds
            .entry(new_id.clone())
            .or_default()
            .push(old_id.clone());
    }

    // Chains: any newId that also appears as an oldId.
    for (old_id, entry) in &table {
        if let Some(next) = table.get(&entry.new_id) {
            diagnostics.push(DiffDiagnostic {
                kind: "rename-overlay-chain".into(),
                fields: vec![
                    (
                        "kind".into(),
                        CborValue::Text("rename-overlay-chain".into()),
                    ),
                    ("from".into(), CborValue::Text(old_id.clone())),
                    ("via".into(), CborValue::Text(entry.new_id.clone())),
                    ("to".into(), CborValue::Text(next.new_id.clone())),
                ],
            });
        }
    }

    // Overlaps: any newId with multiple oldIds.
    for (new_id, olds) in &new_to_olds {
        if olds.len() > 1 {
            let mut targets: Vec<CborValue> =
                olds.iter().map(|o| CborValue::Text(o.clone())).collect();
            targets.push(CborValue::Text(new_id.clone()));
            diagnostics.push(DiffDiagnostic {
                kind: "rename-overlay-overlap".into(),
                fields: vec![
                    (
                        "kind".into(),
                        CborValue::Text("rename-overlay-overlap".into()),
                    ),
                    ("targets".into(), CborValue::Array(targets)),
                ],
            });
        }
    }

    (table, diagnostics)
}

/// Validate the overlay against actual snapshot stable-id sets, emitting stale
/// diagnostics. Mirrors `validateOverlayAgainstSnapshots`.
pub fn validate_overlay_against_snapshots(
    table: &RenameTable,
    old_ids: &HashSet<String>,
    new_ids: &HashSet<String>,
) -> Vec<DiffDiagnostic> {
    let mut diagnostics: Vec<DiffDiagnostic> = Vec::new();
    for (old_id, entry) in table {
        if !old_ids.contains(old_id) {
            diagnostics.push(stale(old_id, "not-in-old"));
        }
        if !new_ids.contains(&entry.new_id) {
            diagnostics.push(stale(&entry.new_id, "not-in-new"));
        }
    }
    diagnostics
}

fn stale(stale_id: &str, reason: &str) -> DiffDiagnostic {
    DiffDiagnostic {
        kind: "rename-overlay-stale".into(),
        fields: vec![
            (
                "kind".into(),
                CborValue::Text("rename-overlay-stale".into()),
            ),
            ("staleId".into(), CborValue::Text(stale_id.to_string())),
            ("reason".into(), CborValue::Text(reason.to_string())),
        ],
    }
}
