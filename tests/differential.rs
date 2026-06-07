//! R0 differential harness — the SAFETY NET for the al-sem → Rust engine
//! migration.
//!
//! For each committed al-sem "golden" identity file under `tests/r0-goldens/`,
//! this runs the Rust `snapshot_workspace()` on the matching in-repo source
//! fixture under `tests/r0-corpus/` and asserts the **identity subset matches**
//! field-for-field. The default `cargo test` runs entirely OFFLINE: no Bun, no
//! al-sem checkout, no `AL_SEM_DIR`. Everything it needs is committed in-repo.
//!
//! A separate, `#[ignore]`d `refresh_goldens_from_al_sem` test (gated on the
//! `AL_SEM_DIR` env var) regenerates + copies the goldens/fixtures from al-sem.
//! It never runs in the normal loop.
//!
//! SCOPE: the in-repo corpus is the FULL source-only `ws-*` set al-sem dumps
//! (157 fixtures as of R0 Task 7, including the `ws-r0-canon-stress` identity
//! stress fixture). The gating logic, allowlist semantics, and live-refresh path
//! are all real; the harness iterates every `tests/r0-goldens/*.golden.json` and
//! requires each to match with `KNOWN_DIVERGENCES.json` == `[]`.
//!
//! ## Comparison rules
//!
//! - Objects are matched by `stableObjectId`, routines by `stableRoutineId`.
//! - Every field is compared for equality: objects compare `name`, `kind`,
//!   `signatureFingerprint`; routines compare those plus `normalizedSignatureHash`
//!   and `canonicalSignatureText`.
//! - The differ MAY sort both sides (it does, by id) but MUST NOT transform any
//!   value — no lowercasing/trimming/normalizing. That belongs in the engines.
//! - A missing object/routine on either side, an extra one, or any unequal field
//!   is a divergence.
//!
//! ## Divergence record + `path` locator format
//!
//! Each divergence is `{ fixture, path, golden_value, rust_value }`. The `path`
//! is a stable, machine-checkable locator:
//!   - field mismatch: `objects["<stableObjectId>"].signatureFingerprint`
//!     or `routines["<stableRoutineId>"].canonicalSignatureText`
//!   - present in golden, absent in rust: `objects["<id>"]:MISSING_IN_RUST`
//!     / `routines["<id>"]:MISSING_IN_RUST`
//!   - present in rust, absent in golden: `objects["<id>"]:EXTRA_IN_RUST`
//!     / `routines["<id>"]:EXTRA_IN_RUST`
//!
//! ## Allowlist gating (`KNOWN_DIVERGENCES.json`, repo root)
//!
//! An array of `{ fixture, path, reason, expires }`. The test FAILS if:
//!   (a) any divergence is NOT covered by an entry (undocumented divergence), OR
//!   (b) any allowlist entry is UNUSED this run (no matching divergence).
//! Matching is EXACT on the `(fixture, path)` pair — not prefix/glob (over-broad
//! = fail). At R0 exit the allowlist is empty and the full corpus matches.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use al_call_hierarchy::engine::l2::features::L2Projection;
use al_call_hierarchy::engine::l2::l2_workspace::project_workspace;
use al_call_hierarchy::engine::snapshot::{
    snapshot_workspace, IdentitySnapshot, ObjectIdentity, RoutineIdentity,
};
use serde::Deserialize;

/// One entry in `KNOWN_DIVERGENCES.json`.
///
/// `test` scopes the entry to ONE differential pass. It defaults to the R0
/// identity test (`differential_identity_subset_matches_goldens`) so existing
/// R0 entries need no `test` field. L2 entries MUST carry
/// `"test": "differential_l2_features_match_goldens"`. Matching is exact on the
/// `(test, fixture, path)` triple, so an R0 entry can never cover an L2
/// divergence (or vice versa).
#[derive(Debug, Clone, Deserialize)]
struct AllowEntry {
    #[serde(default = "default_allow_test")]
    test: String,
    fixture: String,
    path: String,
    #[serde(default)]
    #[allow(dead_code)] // documentation fields; not used in matching.
    reason: String,
    #[serde(default)]
    #[allow(dead_code)]
    expires: String,
}

/// Default allowlist scope: the R0 identity pass.
fn default_allow_test() -> String {
    "differential_identity_subset_matches_goldens".to_string()
}

/// A single, machine-checkable divergence between a golden and the Rust output.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Divergence {
    fixture: String,
    /// Stable locator, e.g. `routines["<id>"].canonicalSignatureText`.
    path: String,
    golden_value: String,
    rust_value: String,
}

/// Repo root = the crate manifest dir (the worktree root). `tests/` and
/// `KNOWN_DIVERGENCES.json` live directly under it.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn goldens_dir() -> PathBuf {
    repo_root().join("tests").join("r0-goldens")
}

fn corpus_dir() -> PathBuf {
    repo_root().join("tests").join("r0-corpus")
}

/// The R1a L2-feature goldens live alongside the R0 goldens but in their own
/// dir; the corpus (source fixtures) is shared with R0 (`tests/r0-corpus/`).
fn r1a_goldens_dir() -> PathBuf {
    repo_root().join("tests").join("r1a-goldens")
}

/// Discover every `tests/r0-goldens/*.golden.json` (skipping `manifest.json`),
/// returning `(fixture_name, golden_path)` sorted by fixture name.
fn discover_goldens() -> Vec<(String, PathBuf)> {
    let dir = goldens_dir();
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("failed to read goldens dir {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".golden.json") {
            continue; // skips manifest.json, README.md, etc.
        }
        let fixture = name.trim_end_matches(".golden.json").to_string();
        out.push((fixture, path));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Load + parse `KNOWN_DIVERGENCES.json` into the same struct shape.
fn load_allowlist() -> Vec<AllowEntry> {
    let path = repo_root().join("KNOWN_DIVERGENCES.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("failed to parse {} as a JSON array: {e}", path.display()))
}

/// Parse a golden file into the SAME `IdentitySnapshot` structs the engine
/// produces.
fn parse_golden(path: &Path) -> IdentitySnapshot {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read golden {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| {
        panic!(
            "failed to parse golden {} as IdentitySnapshot: {e}",
            path.display()
        )
    })
}

/// Compare one fixture's golden vs Rust snapshot, producing every divergence.
/// Pure structural comparison — NO value transforms.
fn diff_snapshots(
    fixture: &str,
    golden: &IdentitySnapshot,
    rust: &IdentitySnapshot,
) -> Vec<Divergence> {
    let mut out = Vec::new();

    // --- Objects, keyed by stableObjectId. ---
    let golden_objs: BTreeMap<&str, &ObjectIdentity> = golden
        .objects
        .iter()
        .map(|o| (o.stable_object_id.as_str(), o))
        .collect();
    let rust_objs: BTreeMap<&str, &ObjectIdentity> = rust
        .objects
        .iter()
        .map(|o| (o.stable_object_id.as_str(), o))
        .collect();

    for (id, g) in &golden_objs {
        match rust_objs.get(id) {
            None => out.push(Divergence {
                fixture: fixture.to_string(),
                path: format!("objects[{:?}]:MISSING_IN_RUST", id),
                golden_value: format!("{g:?}"),
                rust_value: "<absent>".to_string(),
            }),
            Some(r) => {
                push_field(&mut out, fixture, &obj_path(id, "name"), &g.name, &r.name);
                push_field(&mut out, fixture, &obj_path(id, "kind"), &g.kind, &r.kind);
                push_field(
                    &mut out,
                    fixture,
                    &obj_path(id, "signatureFingerprint"),
                    &g.signature_fingerprint,
                    &r.signature_fingerprint,
                );
            }
        }
    }
    for (id, r) in &rust_objs {
        if !golden_objs.contains_key(id) {
            out.push(Divergence {
                fixture: fixture.to_string(),
                path: format!("objects[{:?}]:EXTRA_IN_RUST", id),
                golden_value: "<absent>".to_string(),
                rust_value: format!("{r:?}"),
            });
        }
    }

    // --- Routines, keyed by stableRoutineId. ---
    let golden_routines: BTreeMap<&str, &RoutineIdentity> = golden
        .routines
        .iter()
        .map(|r| (r.stable_routine_id.as_str(), r))
        .collect();
    let rust_routines: BTreeMap<&str, &RoutineIdentity> = rust
        .routines
        .iter()
        .map(|r| (r.stable_routine_id.as_str(), r))
        .collect();

    for (id, g) in &golden_routines {
        match rust_routines.get(id) {
            None => out.push(Divergence {
                fixture: fixture.to_string(),
                path: format!("routines[{:?}]:MISSING_IN_RUST", id),
                golden_value: format!("{g:?}"),
                rust_value: "<absent>".to_string(),
            }),
            Some(r) => {
                push_field(&mut out, fixture, &rt_path(id, "name"), &g.name, &r.name);
                push_field(&mut out, fixture, &rt_path(id, "kind"), &g.kind, &r.kind);
                push_field(
                    &mut out,
                    fixture,
                    &rt_path(id, "signatureFingerprint"),
                    &g.signature_fingerprint,
                    &r.signature_fingerprint,
                );
                push_field(
                    &mut out,
                    fixture,
                    &rt_path(id, "normalizedSignatureHash"),
                    &g.normalized_signature_hash,
                    &r.normalized_signature_hash,
                );
                push_field(
                    &mut out,
                    fixture,
                    &rt_path(id, "canonicalSignatureText"),
                    &g.canonical_signature_text,
                    &r.canonical_signature_text,
                );
            }
        }
    }
    for (id, r) in &rust_routines {
        if !golden_routines.contains_key(id) {
            out.push(Divergence {
                fixture: fixture.to_string(),
                path: format!("routines[{:?}]:EXTRA_IN_RUST", id),
                golden_value: "<absent>".to_string(),
                rust_value: format!("{r:?}"),
            });
        }
    }

    // Stable order for human-readable reporting.
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn obj_path(id: &str, field: &str) -> String {
    format!("objects[{id:?}].{field}")
}

fn rt_path(id: &str, field: &str) -> String {
    format!("routines[{id:?}].{field}")
}

/// Emit a field divergence iff golden != rust. No transforms — exact compare.
fn push_field(out: &mut Vec<Divergence>, fixture: &str, path: &str, golden: &str, rust: &str) {
    if golden != rust {
        out.push(Divergence {
            fixture: fixture.to_string(),
            path: path.to_string(),
            golden_value: golden.to_string(),
            rust_value: rust.to_string(),
        });
    }
}

/// The default, offline differential test. Runs the Rust snapshot on every
/// in-repo golden's matching fixture, diffs, and gates on the allowlist.
#[test]
fn differential_identity_subset_matches_goldens() {
    let goldens = discover_goldens();
    assert!(
        !goldens.is_empty(),
        "no goldens discovered under {} — corpus missing?",
        goldens_dir().display()
    );

    // Only R0-scoped allowlist entries apply to this pass; L2 entries are
    // filtered out (and vice versa in the L2 test).
    let allowlist: Vec<AllowEntry> = load_allowlist()
        .into_iter()
        .filter(|e| e.test == "differential_identity_subset_matches_goldens")
        .collect();

    // Collect every divergence across every fixture.
    let mut all_divergences: Vec<Divergence> = Vec::new();
    for (fixture, golden_path) in &goldens {
        let fixture_dir = corpus_dir().join(fixture);
        assert!(
            fixture_dir.is_dir(),
            "golden {} has no matching in-repo fixture at {} (offline corpus incomplete)",
            golden_path.display(),
            fixture_dir.display()
        );

        let golden = parse_golden(golden_path);
        let rust = snapshot_workspace(&fixture_dir)
            .unwrap_or_else(|e| panic!("snapshot_workspace failed on {fixture}: {e:#}"));

        let mut divs = diff_snapshots(fixture, &golden, &rust);
        all_divergences.append(&mut divs);
    }

    // --- Allowlist gating ---------------------------------------------------
    // (a) every divergence must be covered by an exact (fixture, path) entry;
    // (b) every allowlist entry must match at least one divergence this run.
    let mut entry_used = vec![false; allowlist.len()];
    let mut undocumented: Vec<&Divergence> = Vec::new();

    for div in &all_divergences {
        let mut covered = false;
        for (i, entry) in allowlist.iter().enumerate() {
            if entry.fixture == div.fixture && entry.path == div.path {
                entry_used[i] = true;
                covered = true;
                // keep scanning so a divergence matched by multiple identical
                // entries marks them all used (still flagged later as redundant
                // only if truly unused — exact dupes both count as used).
            }
        }
        if !covered {
            undocumented.push(div);
        }
    }

    let unused: Vec<&AllowEntry> = allowlist
        .iter()
        .enumerate()
        .filter(|(i, _)| !entry_used[*i])
        .map(|(_, e)| e)
        .collect();

    let mut failure = String::new();

    if !undocumented.is_empty() {
        failure.push_str(&format!(
            "\n{} UNDOCUMENTED divergence(s) (not in KNOWN_DIVERGENCES.json):\n",
            undocumented.len()
        ));
        for d in &undocumented {
            failure.push_str(&format!(
                "  [{}] {}\n      golden = {}\n      rust   = {}\n",
                d.fixture, d.path, d.golden_value, d.rust_value
            ));
        }
    }

    if !unused.is_empty() {
        failure.push_str(&format!(
            "\n{} UNUSED allowlist entr(y/ies) (no matching divergence this run — \
             remove or fix; over-broad/stale entries are not allowed):\n",
            unused.len()
        ));
        for e in &unused {
            failure.push_str(&format!(
                "  [{}] {}  (reason: {:?}, expires: {:?})\n",
                e.fixture, e.path, e.reason, e.expires
            ));
        }
    }

    assert!(
        failure.is_empty(),
        "R0 differential harness FAILED:{failure}\n\
         (matched {} fixture(s); the goldens carry canonicalSignatureText so a \
         signature drift is human-readable above.)",
        goldens.len()
    );

    eprintln!(
        "R0 differential: {} fixture(s), 0 divergences, allowlist fully consumed ({} entr(y/ies)).",
        goldens.len(),
        allowlist.len()
    );
}

// =============================================================================
// R1a L2-features differential pass
// =============================================================================
//
// For each `tests/r1a-goldens/<name>.l2.golden.json`, run the Rust L2 dump
// (`project_workspace`) on `tests/r0-corpus/<name>` (the same source fixtures
// the R0 pass uses), and compare the allowlisted R1a projection as PARSED
// structures.
//
// ## Comparison rules (R1 spec §2 / R1a plan Task 4)
//
//   - Both sides are validated to parse as the `L2Projection` serde type (which
//     STRUCTURALLY OMITS every forbidden field), then compared as the raw parsed
//     `serde_json::Value` so the path locator is precise to the leaf.
//   - The differ MAY sort the top-level `objects`/`routines` lists (by
//     stableObjectId / stableRoutineId) — and ONLY those. It MUST NOT transform
//     any value. All enumerated arrays (operationSites / recordOperations /
//     callSites / loops / fieldAccesses / statementTree.children /
//     conditionLeaves / recordVariables / variables / …) are compared
//     POSITIONALLY, in order, because their order is semantically meaningful.
//   - R1b: `controlContext` is now part of the projection — emitted by the Rust
//     side, present in the goldens, and compared structurally (positional/by-id).
//     It is NO LONGER forbidden. MUST FAIL if EITHER side carries a STILL-forbidden
//     field anywhere (`order` / `scopeFrames` / capability / `resourceId` /
//     `tableId` / `calleeParameterIsVar` / `bindingResolution` / `sourceTableId`)
//     — a recursive key scan on both parsed values, belt-and-suspenders even
//     though the serde types omit them.
//
// ## Divergence record + `path` locator
//
// Each divergence is `{ fixture, path, golden, rust }`. The `path` is a JSON
// pointer-ish locator into the (top-sorted) projection, e.g.
//   routines[12].features.callSites[0].argumentInfos[1].kind
//   objects[2].sourceTableName:MISSING_IN_RUST
//   …:FORBIDDEN_FIELD
// so the locality (raw text / CFN node path / ExpressionInfo / operationId)
// falls straight out of the pointer.
//
// ## Allowlist gating
//
// Reuses `KNOWN_DIVERGENCES.json` + the exact-`(test,fixture,path)` gating, but
// only entries scoped to `test == "differential_l2_features_match_goldens"`
// apply here. Target: empty.
//
// ## Scope gate (Task 4 vs Task 5)
//
// `R1A_L2_SET` selects which fixtures this test ASSERTS on:
//   - unset / "full" (the committed default, since R1a Task 5 / exit gate):
//     every `tests/r1a-goldens/*.l2.golden.json` — the full 152-fixture corpus.
//   - "small": ws-d2 + ws-r0-canon-stress only (the proven-green Task-4 subset),
//     kept for fast localized iteration during development.
// Either way the harness, forbidden-field scan, and gating are identical — only
// the fixture set differs. The committed default asserts FULL-corpus L2 parity.

/// Keys that must NEVER appear on either side of the L2 comparison (later-gate /
/// L3-resolved). Mirrors `al2dump_smoke::FORBIDDEN_KEYS` + the projection's
/// `r1a-l2-projection.ts` FORBIDDEN_KEYS.
const L2_FORBIDDEN_KEYS: &[&str] = &[
    // R1b: controlContext is now REQUIRED (compared, not forbidden) — removed.
    "order",
    "scopeFrames",
    "capability",
    "resourceId",
    "tableId",
    "calleeParameterIsVar",
    "bindingResolution",
    "sourceTableId",
];

const L2_TEST_NAME: &str = "differential_l2_features_match_goldens";

/// Discover every `tests/r1a-goldens/*.l2.golden.json`, sorted by fixture name.
fn discover_l2_goldens() -> Vec<(String, PathBuf)> {
    let dir = r1a_goldens_dir();
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("failed to read L2 goldens dir {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".l2.golden.json") {
            continue; // skips manifest.json, l2-vectors.json
        }
        let fixture = name.trim_end_matches(".l2.golden.json").to_string();
        out.push((fixture, entry.path()));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Recursively collect every forbidden object-key in `value`, with its JSON
/// pointer path (so a leak is reported with locality, not just "present").
fn scan_forbidden_keys(value: &serde_json::Value, path: &str, hits: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let child = format!("{path}.{k}");
                if L2_FORBIDDEN_KEYS.contains(&k.as_str()) {
                    hits.push(child.clone());
                }
                scan_forbidden_keys(v, &child, hits);
            }
        }
        serde_json::Value::Array(arr) => {
            for (i, v) in arr.iter().enumerate() {
                scan_forbidden_keys(v, &format!("{path}[{i}]"), hits);
            }
        }
        _ => {}
    }
}

/// Canonicalize a projection `Value` for comparison: sort ONLY the top-level
/// `objects` (by stableObjectId) and `routines` (by stableRoutineId) arrays. No
/// other transform. Returns a fresh value (does not mutate the input).
fn canonicalize_l2_top(value: &serde_json::Value) -> serde_json::Value {
    let mut v = value.clone();
    if let Some(obj) = v.as_object_mut() {
        if let Some(serde_json::Value::Array(arr)) = obj.get_mut("objects") {
            arr.sort_by(|a, b| {
                a.get("stableObjectId")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .cmp(
                        b.get("stableObjectId")
                            .and_then(|x| x.as_str())
                            .unwrap_or(""),
                    )
            });
        }
        if let Some(serde_json::Value::Array(arr)) = obj.get_mut("routines") {
            arr.sort_by(|a, b| {
                a.get("stableRoutineId")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .cmp(
                        b.get("stableRoutineId")
                            .and_then(|x| x.as_str())
                            .unwrap_or(""),
                    )
            });
        }
    }
    v
}

/// Recursively diff two canonicalized projection values POSITIONALLY, emitting a
/// `Divergence` per leaf mismatch / shape mismatch / missing-or-extra key/elem.
fn diff_l2_value(
    fixture: &str,
    path: &str,
    golden: &serde_json::Value,
    rust: &serde_json::Value,
    out: &mut Vec<Divergence>,
) {
    use serde_json::Value;
    match (golden, rust) {
        (Value::Object(g), Value::Object(r)) => {
            // Keys present in golden — compare or flag MISSING_IN_RUST.
            for (k, gv) in g {
                let child = format!("{path}.{k}");
                match r.get(k) {
                    Some(rv) => diff_l2_value(fixture, &child, gv, rv, out),
                    None => out.push(Divergence {
                        fixture: fixture.to_string(),
                        path: format!("{child}:MISSING_IN_RUST"),
                        golden_value: compact(gv),
                        rust_value: "<absent>".to_string(),
                    }),
                }
            }
            // Keys only in rust — EXTRA_IN_RUST.
            for (k, rv) in r {
                if !g.contains_key(k) {
                    out.push(Divergence {
                        fixture: fixture.to_string(),
                        path: format!("{path}.{k}:EXTRA_IN_RUST"),
                        golden_value: "<absent>".to_string(),
                        rust_value: compact(rv),
                    });
                }
            }
        }
        (Value::Array(g), Value::Array(r)) => {
            if g.len() != r.len() {
                out.push(Divergence {
                    fixture: fixture.to_string(),
                    path: format!("{path}:LENGTH"),
                    golden_value: g.len().to_string(),
                    rust_value: r.len().to_string(),
                });
            }
            // Positional comparison up to the shorter length; surplus elems on
            // either side are reported as MISSING/EXTRA at their index.
            let n = g.len().min(r.len());
            for i in 0..n {
                diff_l2_value(fixture, &format!("{path}[{i}]"), &g[i], &r[i], out);
            }
            for (i, gv) in g.iter().enumerate().skip(n) {
                out.push(Divergence {
                    fixture: fixture.to_string(),
                    path: format!("{path}[{i}]:MISSING_IN_RUST"),
                    golden_value: compact(gv),
                    rust_value: "<absent>".to_string(),
                });
            }
            for (i, rv) in r.iter().enumerate().skip(n) {
                out.push(Divergence {
                    fixture: fixture.to_string(),
                    path: format!("{path}[{i}]:EXTRA_IN_RUST"),
                    golden_value: "<absent>".to_string(),
                    rust_value: compact(rv),
                });
            }
        }
        _ => {
            if golden != rust {
                out.push(Divergence {
                    fixture: fixture.to_string(),
                    path: path.to_string(),
                    golden_value: compact(golden),
                    rust_value: compact(rust),
                });
            }
        }
    }
}

/// Compact single-line JSON for divergence reporting.
fn compact(v: &serde_json::Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| format!("{v:?}"))
}

/// The L2-features differential pass. Gated by `R1A_L2_SET` (see module doc) —
/// committed default is the small proven-green set (ws-d2 + ws-r0-canon-stress).
#[test]
fn differential_l2_features_match_goldens() {
    let all_goldens = discover_l2_goldens();
    assert!(
        !all_goldens.is_empty(),
        "no L2 goldens discovered under {} — corpus missing?",
        r1a_goldens_dir().display()
    );

    // Scope gate: which fixtures this test ASSERTS on. Committed default is the
    // FULL 152-fixture corpus (R1a exit gate); `small` is the dev subset.
    let set = std::env::var("R1A_L2_SET").unwrap_or_else(|_| "full".to_string());
    let small_set = ["ws-d2", "ws-r0-canon-stress"];
    let goldens: Vec<(String, PathBuf)> = match set.as_str() {
        "full" | "" => all_goldens,
        "small" => all_goldens
            .into_iter()
            .filter(|(f, _)| small_set.contains(&f.as_str()))
            .collect(),
        other => panic!("R1A_L2_SET={other:?} not recognized (expected `small` or `full`)"),
    };
    assert!(
        !goldens.is_empty(),
        "R1A_L2_SET={set:?} selected zero fixtures (small set = {small_set:?})"
    );

    // Only L2-scoped allowlist entries apply here.
    let allowlist: Vec<AllowEntry> = load_allowlist()
        .into_iter()
        .filter(|e| e.test == L2_TEST_NAME)
        .collect();

    let mut all_divergences: Vec<Divergence> = Vec::new();
    let mut forbidden_hits: Vec<String> = Vec::new();

    for (fixture, golden_path) in &goldens {
        let fixture_dir = corpus_dir().join(fixture);
        assert!(
            fixture_dir.is_dir(),
            "L2 golden {} has no matching in-repo fixture at {} (offline corpus incomplete)",
            golden_path.display(),
            fixture_dir.display()
        );

        // Golden side: parse as JSON (for the diff) AND validate it parses as the
        // allowlisted L2Projection serde type (shape guard).
        let golden_text = std::fs::read_to_string(golden_path)
            .unwrap_or_else(|e| panic!("read L2 golden {}: {e}", golden_path.display()));
        let golden_json: serde_json::Value =
            serde_json::from_str(&golden_text).unwrap_or_else(|e| {
                panic!("L2 golden {} is not valid JSON: {e}", golden_path.display())
            });
        let _: L2Projection = serde_json::from_value(golden_json.clone()).unwrap_or_else(|e| {
            panic!(
                "L2 golden {} does not parse as L2Projection: {e}",
                golden_path.display()
            )
        });

        // Rust side: project + serialize back to JSON for the structural diff.
        let projection = project_workspace(&fixture_dir)
            .unwrap_or_else(|e| panic!("project_workspace failed on {fixture}: {e:#}"));
        let rust_json = serde_json::to_value(&projection)
            .unwrap_or_else(|e| panic!("serialize Rust L2 projection for {fixture}: {e}"));

        // Forbidden-field scan on BOTH sides (belt-and-suspenders).
        scan_forbidden_keys(
            &golden_json,
            &format!("{fixture}:golden"),
            &mut forbidden_hits,
        );
        scan_forbidden_keys(&rust_json, &format!("{fixture}:rust"), &mut forbidden_hits);

        // Canonicalize top-level lists only, then positional diff.
        let g = canonicalize_l2_top(&golden_json);
        let r = canonicalize_l2_top(&rust_json);
        diff_l2_value(fixture, "", &g, &r, &mut all_divergences);
    }

    all_divergences
        .sort_by(|a, b| (a.fixture.as_str(), &a.path).cmp(&(b.fixture.as_str(), &b.path)));

    // --- Forbidden-field guard (hard fail, never allowlistable) -------------
    assert!(
        forbidden_hits.is_empty(),
        "FORBIDDEN later-gate/L3 field(s) leaked into the L2 comparison \
         (golden or rust):\n  {}",
        forbidden_hits.join("\n  ")
    );

    // --- Allowlist gating (same semantics as R0) ----------------------------
    let mut entry_used = vec![false; allowlist.len()];
    let mut undocumented: Vec<&Divergence> = Vec::new();
    for div in &all_divergences {
        let mut covered = false;
        for (i, entry) in allowlist.iter().enumerate() {
            if entry.fixture == div.fixture && entry.path == div.path {
                entry_used[i] = true;
                covered = true;
            }
        }
        if !covered {
            undocumented.push(div);
        }
    }
    let unused: Vec<&AllowEntry> = allowlist
        .iter()
        .enumerate()
        .filter(|(i, _)| !entry_used[*i])
        .map(|(_, e)| e)
        .collect();

    let mut failure = String::new();
    if !undocumented.is_empty() {
        failure.push_str(&format!(
            "\n{} UNDOCUMENTED L2 divergence(s) (not in KNOWN_DIVERGENCES.json, \
             test={L2_TEST_NAME}):\n",
            undocumented.len()
        ));
        for d in &undocumented {
            failure.push_str(&format!(
                "  [{}] {}\n      golden = {}\n      rust   = {}\n",
                d.fixture, d.path, d.golden_value, d.rust_value
            ));
        }
    }
    if !unused.is_empty() {
        failure.push_str(&format!(
            "\n{} UNUSED L2 allowlist entr(y/ies) (no matching divergence this run):\n",
            unused.len()
        ));
        for e in &unused {
            failure.push_str(&format!(
                "  [{}] {}  (reason: {:?}, expires: {:?})\n",
                e.fixture, e.path, e.reason, e.expires
            ));
        }
    }

    assert!(
        failure.is_empty(),
        "R1a L2-features differential FAILED (set={set:?}):{failure}"
    );

    eprintln!(
        "R1a L2 differential: set={set:?}, {} fixture(s), 0 divergences, \
         allowlist fully consumed ({} L2 entr(y/ies)).",
        goldens.len(),
        allowlist.len()
    );
}

/// LIVE / REFRESH mode — NOT part of the default loop.
///
/// Gated behind `AL_SEM_DIR`. Run explicitly with:
///   AL_SEM_DIR=/u/Git/al-sem cargo test --test differential -- \
///       --ignored refresh_goldens_from_al_sem --nocapture
///
/// It (a) shells `bun run scripts/dump-goldens.ts` in `$AL_SEM_DIR` to
/// regenerate the goldens, (b) copies the source-only `ws-*` fixtures + their
/// `*.golden.json` + `manifest.json` into `tests/r0-corpus/` and
/// `tests/r0-goldens/`, (c) prints al-sem git sha + grammar sha + this engine's
/// commit for provenance, and (d) does NOT auto-commit (leaves a reviewable
/// diff). If `AL_SEM_DIR` is unset it skips (so an accidental `--ignored` run is
/// a no-op rather than a failure).
#[test]
#[ignore = "live/refresh mode: regenerates goldens from al-sem; requires AL_SEM_DIR + Bun"]
fn refresh_goldens_from_al_sem() {
    let Ok(al_sem_dir) = std::env::var("AL_SEM_DIR") else {
        eprintln!(
            "refresh_goldens_from_al_sem: AL_SEM_DIR not set — skipping (this is the \
             refresh path; set AL_SEM_DIR=/u/Git/al-sem to run it)."
        );
        return;
    };
    let al_sem = PathBuf::from(&al_sem_dir);
    assert!(
        al_sem.is_dir(),
        "AL_SEM_DIR is not a directory: {al_sem_dir}"
    );

    // (a) Regenerate goldens via Bun inside the al-sem checkout.
    eprintln!("refresh: running `bun run scripts/dump-goldens.ts` in {al_sem_dir} ...");
    let status = std::process::Command::new("bun")
        .args(["run", "scripts/dump-goldens.ts"])
        .current_dir(&al_sem)
        // dump-goldens writes the manifest JSON to stdout; discard it (files are
        // the artifact). Logs go to the inherited stderr.
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn `bun` (is Bun on PATH?): {e}"));
    assert!(
        status.success(),
        "`bun run scripts/dump-goldens.ts` failed with status {status}"
    );

    let src_goldens = al_sem.join("scripts").join("r0-goldens");
    let src_fixtures = al_sem.join("test").join("fixtures");
    let dst_goldens = goldens_dir();
    let dst_corpus = corpus_dir();
    std::fs::create_dir_all(&dst_goldens).expect("create tests/r0-goldens");
    std::fs::create_dir_all(&dst_corpus).expect("create tests/r0-corpus");

    // (b) Copy each generated golden + its source-only fixture. This copies the
    //     FULL source-only corpus al-sem produced; every copied golden is then
    //     REQUIRED to match in the default offline differential (R0 exit gate).
    let mut copied = 0usize;
    for entry in std::fs::read_dir(&src_goldens).expect("read al-sem r0-goldens") {
        let entry = entry.expect("entry");
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".golden.json") {
            continue;
        }
        let fixture = name.trim_end_matches(".golden.json").to_string();
        let fixture_src = src_fixtures.join(&fixture);
        if !fixture_src.is_dir() {
            eprintln!(
                "refresh: skip {fixture} (no source fixture at {})",
                fixture_src.display()
            );
            continue;
        }
        // golden file
        std::fs::copy(entry.path(), dst_goldens.join(&name))
            .unwrap_or_else(|e| panic!("copy golden {name}: {e}"));
        // source-only fixture (app.json + src/**)
        copy_source_fixture(&fixture_src, &dst_corpus.join(&fixture));
        copied += 1;
    }
    // manifest
    let manifest_src = src_goldens.join("manifest.json");
    if manifest_src.is_file() {
        std::fs::copy(&manifest_src, dst_goldens.join("manifest.json"))
            .expect("copy manifest.json");
    }

    // (b2) Regenerate + copy the R1a L2-feature goldens. Same al-sem checkout,
    //      a second dump script (`scripts/dump-l2-features.ts`). The L2 goldens
    //      land in `scripts/r1a-goldens/`; copy them + their manifest into
    //      `tests/r1a-goldens/`. The source fixtures are the SAME `ws-*` trees
    //      already copied to `tests/r0-corpus/` above, so no separate corpus.
    eprintln!("refresh: running `bun run scripts/dump-l2-features.ts` in {al_sem_dir} ...");
    let l2_status = std::process::Command::new("bun")
        .args(["run", "scripts/dump-l2-features.ts"])
        .current_dir(&al_sem)
        // dump-l2-features writes its manifest JSON to stdout; discard it (files
        // are the artifact). Logs go to the inherited stderr.
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn `bun` for L2 dump: {e}"));
    assert!(
        l2_status.success(),
        "`bun run scripts/dump-l2-features.ts` failed with status {l2_status}"
    );

    let src_l2_goldens = al_sem.join("scripts").join("r1a-goldens");
    let dst_l2_goldens = r1a_goldens_dir();
    std::fs::create_dir_all(&dst_l2_goldens).expect("create tests/r1a-goldens");
    let mut l2_copied = 0usize;
    for entry in std::fs::read_dir(&src_l2_goldens).expect("read al-sem r1a-goldens") {
        let entry = entry.expect("entry");
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".l2.golden.json") {
            continue; // skips manifest.json + l2-vectors.json (vectors are separate).
        }
        std::fs::copy(entry.path(), dst_l2_goldens.join(&name))
            .unwrap_or_else(|e| panic!("copy L2 golden {name}: {e}"));
        l2_copied += 1;
    }
    let l2_manifest_src = src_l2_goldens.join("manifest.json");
    if l2_manifest_src.is_file() {
        std::fs::copy(&l2_manifest_src, dst_l2_goldens.join("manifest.json"))
            .expect("copy r1a-goldens/manifest.json");
    }
    eprintln!("refresh: copied {l2_copied} L2 golden(s) into tests/r1a-goldens/.");

    // (c) Provenance.
    let al_sem_sha = git_sha(&al_sem);
    let grammar_sha = read_manifest_field(
        &dst_goldens.join("manifest.json"),
        "treeSitterAlNativeSha256",
    );
    let engine_sha = git_sha(&repo_root());
    eprintln!("refresh: copied {copied} fixture(s)/golden(s).");
    eprintln!("refresh: provenance:");
    eprintln!("  al-sem git sha     = {al_sem_sha}");
    eprintln!("  tree-sitter-al sha = {grammar_sha}");
    eprintln!("  engine commit sha  = {engine_sha}");
    eprintln!("refresh: NOT auto-committed — review the diff and commit deliberately.");
}

/// Copy a source-only fixture (every `app.json` + `*.al` in the tree) into `dst`,
/// skipping dependency/package dirs. Mirrors the offline-corpus contract AND the
/// al-sem WorkspaceProvider layout view: nested `a/app.json` + `b/app.json` of a
/// multi-app fixture (e.g. `ws-diff-*`) are copied so the fail-closed (multi-app)
/// branch is exercised by the offline corpus exactly as in al-sem.
fn copy_source_fixture(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap_or_else(|e| panic!("create {}: {e}", dst.display()));
    // Recurse over the whole tree, copying both `*.al` and `app.json` files
    // (root or nested), so the corpus mirrors al-sem's source tree.
    copy_al_tree(src, dst);
}

/// Recursively copy `*.al` and `app.json` files (and the dirs containing them)
/// from `src` to `dst`, skipping `node_modules` / `.alpackages` / `.git`. Copying
/// nested `app.json` files is required so the multi-app fail-closed branch is
/// reproduced offline (al-sem counts app.json under the root excluding those dirs).
fn copy_al_tree(src: &Path, dst: &Path) {
    let Ok(entries) = std::fs::read_dir(src) else {
        return;
    };
    for entry in entries {
        let entry = entry.expect("entry");
        let path = entry.path();
        let ftype = entry.file_type().expect("file_type");
        let name = entry.file_name().to_string_lossy().to_string();
        if ftype.is_dir() {
            let name_lc = name.to_lowercase();
            if name_lc == "node_modules" || name_lc == ".alpackages" || name_lc == ".git" {
                continue;
            }
            copy_al_tree(&path, &dst.join(&name));
        } else if ftype.is_file()
            && (path
                .extension()
                .map(|e| e.eq_ignore_ascii_case("al"))
                .unwrap_or(false)
                || name.eq_ignore_ascii_case("app.json"))
        {
            std::fs::create_dir_all(dst).expect("create dst dir");
            std::fs::copy(&path, dst.join(&name))
                .unwrap_or_else(|e| panic!("copy {}: {e}", path.display()));
        }
    }
}

/// `git rev-parse HEAD` in `dir`, or `<unknown>` on any failure.
fn git_sha(dir: &Path) -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "<unknown>".to_string())
}

/// Pull a top-level string field out of `manifest.json`, or `<unknown>`.
fn read_manifest_field(manifest: &Path, field: &str) -> String {
    std::fs::read_to_string(manifest)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get(field).and_then(|f| f.as_str()).map(str::to_string))
        .unwrap_or_else(|| "<unknown>".to_string())
}
