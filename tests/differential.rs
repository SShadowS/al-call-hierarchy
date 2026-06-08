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
use al_call_hierarchy::engine::l3::call_graph_projection::L3CallGraphProjection;
use al_call_hierarchy::engine::l3::coverage::AnalysisCoverage;
use al_call_hierarchy::engine::l3::event_graph::L3EventGraphProjection;
use al_call_hierarchy::engine::l3::l3_workspace::{
    assemble_and_resolve_workspace_default, L3RecordTypeProjection,
};
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
//   - R1b/R1c/R1d: `controlContext` + `order` + `scopeFrames` +
//     `capabilityFactsDirect`/`capabilityStatus`/`capabilityReasons`/
//     `capabilityDiagnostics` are now part of the projection — emitted by the Rust
//     side, present in the goldens, and compared structurally. Capability facts
//     are compared POSITIONALLY (extraction order — al-sem pushes them in
//     fixed family-dispatch order, never sorts); reasons are dedup+lexicographic-
//     sorted and diagnostics are (sourceRef,message)-sorted on BOTH sides before
//     comparison. They are NO LONGER forbidden. MUST FAIL if EITHER side carries a
//     STILL-forbidden L3 field anywhere (`resourceId` / `tableId` /
//     `calleeParameterIsVar` / `bindingResolution` / `sourceTableId`) — a recursive
//     key scan on both parsed values, belt-and-suspenders even though the serde
//     types omit them (e.g. a `tableId` leaking through a nested `table-field`
//     ValueSource is a hard fail).
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
    // R1c: order + scopeFrames are now EMITTED + compared (not forbidden) — removed.
    // R1d: capabilityFactsDirect / capabilityStatus / capabilityReasons /
    //   capabilityDiagnostics are now EMITTED + compared (not forbidden). The
    //   STILL-forbidden set is only the L3-resolved fields below (mirrors the
    //   refreshed manifest.json `forbiddenKeys`). The scan still HARD-FAILS if a
    //   `tableId` leaks through a nested `table-field` ValueSource.
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

// =============================================================================
// R2a L3 record-types differential pass + COVERAGE MATRIX (anti-degenerate)
// =============================================================================
//
// For each `tests/r2a-goldens/<name>.l3rt.golden.json`, run the disk-backed L3
// assemble+resolve (`assemble_and_resolve_workspace_default`) on
// `tests/r0-corpus/<name>` (the SAME source fixtures the R0/R1a passes use),
// project to the golden-shaped record-type projection, and compare structurally:
// resolved record-var/op StableTableId (OMITTED when unresolved) + per-Table
// merged fields.
//
// ## Capture point (R2a)
//
// POST-RESOLVE / PRE-SUMMARY: only the record-type surface. Every emitted field
// is built key-by-key (serde projection types), so later-gate / L4 fields cannot
// leak through a spread. A belt-and-suspenders recursive scan HARD-FAILS if any
// of `callGraph` / `eventGraph` / `coverage` / `typedEdges` / `resourceId` /
// `bindingResolution` / `argumentBindings` / `summary` / `capabilityFactsDirect`
// appears on EITHER side (matches the manifest `forbiddenKeys`).
//
// ## Comparison rules
//
//   - Both sides validated to parse as the `L3RecordTypeProjection` serde type
//     (shape guard — structurally omits everything but the record-type surface),
//     then compared as raw `serde_json::Value` POSITIONALLY (the projection is
//     already canonically sorted: tables by stableTableId, routines by
//     stableRoutineId, fields by (fieldNumber,name), vars by (name,tableId), ops
//     by operationId — so NO further sort/transform is applied here).
//
// ## COVERAGE MATRIX (anti-degenerate, [REV2])
//
// Across the corpus, the pass computes + ENFORCES nonzero counts of:
//   1. resolved record-var tableIds
//   2. resolved record-op tableIds
//   3. implicit-Rec resolutions (recordVariableName lc ∈ {"rec","xrec"} AND tableId set)
//   4. merged extension fields (declaringObjectId contains ":TableExtension:")
// computed from the RUST output (so it proves the Rust resolution actually FIRES,
// not "unresolved == unresolved"). If ANY of the four is zero the test FAILS —
// a degenerate (all-unresolved) port would otherwise pass a pure equality diff.
// The matrix counts are printed; an oracle cross-check asserts they equal the
// counts computed from the GOLDENS (al-sem ground truth).
//
// ## Allowlist gating + scope gate
//
// Reuses `KNOWN_DIVERGENCES.json` with `test == L3_TEST_NAME`; target empty.
// `R2A_L3_SET` selects the asserted fixtures:
//   - "full" (committed default since R2a Task 4 / the EXIT GATE): every
//     `tests/r2a-goldens/*.l3rt.golden.json` (the 153-fixture corpus). The
//     committed `cargo test --test differential` asserts FULL-corpus L3
//     record-type parity + the coverage matrix by default.
//   - "small": ws-d2 + ws-r2a-record-types — the proven-green dev subset, kept
//     for fast localized iteration.

/// Keys that must NEVER appear on either side of the L3 record-type comparison —
/// later-gate / L4 surfaces. Mirrors the manifest `forbiddenKeys` + the
/// projection's `r2a-l3-projection.ts` FORBIDDEN_KEYS.
const L3_FORBIDDEN_KEYS: &[&str] = &[
    "callGraph",
    "eventGraph",
    "coverage",
    "typedEdges",
    "resourceId",
    "bindingResolution",
    "argumentBindings",
    "summary",
    "capabilityFactsDirect",
];

const L3_TEST_NAME: &str = "differential_l3_record_types_match_goldens";

/// The four anti-degenerate coverage counts.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct CoverageMatrix {
    resolved_record_var_table_ids: usize,
    resolved_record_op_table_ids: usize,
    implicit_rec_resolutions: usize,
    merged_extension_fields: usize,
}

impl CoverageMatrix {
    fn add(&mut self, other: &CoverageMatrix) {
        self.resolved_record_var_table_ids += other.resolved_record_var_table_ids;
        self.resolved_record_op_table_ids += other.resolved_record_op_table_ids;
        self.implicit_rec_resolutions += other.implicit_rec_resolutions;
        self.merged_extension_fields += other.merged_extension_fields;
    }
}

/// Compute the coverage matrix for ONE projection `Value` (golden OR rust). The
/// shapes are identical, so the same walker serves both.
fn coverage_of(proj: &serde_json::Value) -> CoverageMatrix {
    let mut m = CoverageMatrix::default();
    if let Some(tables) = proj.get("tables").and_then(|t| t.as_array()) {
        for t in tables {
            if let Some(fields) = t.get("fields").and_then(|f| f.as_array()) {
                for f in fields {
                    if f.get("declaringObjectId")
                        .and_then(|d| d.as_str())
                        .map(|d| d.contains(":TableExtension:"))
                        .unwrap_or(false)
                    {
                        m.merged_extension_fields += 1;
                    }
                }
            }
        }
    }
    if let Some(routines) = proj.get("routines").and_then(|r| r.as_array()) {
        for r in routines {
            if let Some(vars) = r.get("recordVariables").and_then(|v| v.as_array()) {
                for v in vars {
                    if v.get("tableId").is_some() {
                        m.resolved_record_var_table_ids += 1;
                    }
                }
            }
            if let Some(ops) = r.get("recordOperations").and_then(|o| o.as_array()) {
                for o in ops {
                    let has_table = o.get("tableId").is_some();
                    if has_table {
                        m.resolved_record_op_table_ids += 1;
                    }
                    let name = o
                        .get("recordVariableName")
                        .and_then(|n| n.as_str())
                        .unwrap_or("");
                    if (name == "rec" || name == "xrec") && has_table {
                        m.implicit_rec_resolutions += 1;
                    }
                }
            }
        }
    }
    m
}

/// Discover every `tests/r2a-goldens/*.l3rt.golden.json`, sorted by fixture name.
fn discover_l3_goldens() -> Vec<(String, PathBuf)> {
    let dir = repo_root().join("tests").join("r2a-goldens");
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("failed to read L3 goldens dir {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".l3rt.golden.json") {
            continue; // skips manifest.json, l3rt-vectors.json
        }
        let fixture = name.trim_end_matches(".l3rt.golden.json").to_string();
        out.push((fixture, entry.path()));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The L3 record-types differential pass + coverage matrix. Gated by
/// `R2A_L3_SET` (committed default `full`: the whole 153-fixture corpus — the
/// R2a exit gate; `small` = ws-d2 + ws-r2a-record-types for dev iteration).
#[test]
fn differential_l3_record_types_match_goldens() {
    let all_goldens = discover_l3_goldens();
    assert!(
        !all_goldens.is_empty(),
        "no L3 goldens discovered under tests/r2a-goldens — corpus missing?"
    );

    // Scope gate. Committed default since R2a Task 4 (the EXIT GATE) is the FULL
    // 153-fixture corpus; `small` is the proven-green dev subset.
    let set = std::env::var("R2A_L3_SET").unwrap_or_else(|_| "full".to_string());
    let small_set = ["ws-d2", "ws-r2a-record-types"];
    let goldens: Vec<(String, PathBuf)> = match set.as_str() {
        "full" | "" => all_goldens,
        "small" => all_goldens
            .into_iter()
            .filter(|(f, _)| small_set.contains(&f.as_str()))
            .collect(),
        other => panic!("R2A_L3_SET={other:?} not recognized (expected `small` or `full`)"),
    };
    assert!(
        !goldens.is_empty(),
        "R2A_L3_SET={set:?} selected zero fixtures (small set = {small_set:?})"
    );

    // Only L3-scoped allowlist entries apply.
    let allowlist: Vec<AllowEntry> = load_allowlist()
        .into_iter()
        .filter(|e| e.test == L3_TEST_NAME)
        .collect();

    let mut all_divergences: Vec<Divergence> = Vec::new();
    let mut forbidden_hits: Vec<String> = Vec::new();
    let mut rust_cov = CoverageMatrix::default();
    let mut golden_cov = CoverageMatrix::default();

    for (fixture, golden_path) in &goldens {
        let fixture_dir = corpus_dir().join(fixture);
        assert!(
            fixture_dir.is_dir(),
            "L3 golden {} has no matching in-repo fixture at {} (offline corpus incomplete)",
            golden_path.display(),
            fixture_dir.display()
        );

        // Golden side: parse as JSON (for the diff) AND validate it parses as the
        // allowlisted L3RecordTypeProjection serde type (shape guard).
        let golden_text = std::fs::read_to_string(golden_path)
            .unwrap_or_else(|e| panic!("read L3 golden {}: {e}", golden_path.display()));
        let golden_json: serde_json::Value =
            serde_json::from_str(&golden_text).unwrap_or_else(|e| {
                panic!("L3 golden {} is not valid JSON: {e}", golden_path.display())
            });
        let _: L3RecordTypeProjection =
            serde_json::from_value(golden_json.clone()).unwrap_or_else(|e| {
                panic!(
                    "L3 golden {} does not parse as L3RecordTypeProjection: {e}",
                    golden_path.display()
                )
            });

        // Rust side: disk-backed assemble+resolve → project → JSON. Fail-closed
        // (empty) layouts yield an empty projection (never throws).
        let projection = match assemble_and_resolve_workspace_default(&fixture_dir) {
            Some(resolved) => resolved.project(),
            None => L3RecordTypeProjection {
                tables: vec![],
                routines: vec![],
            },
        };
        let rust_json = serde_json::to_value(&projection)
            .unwrap_or_else(|e| panic!("serialize Rust L3 projection for {fixture}: {e}"));

        // Forbidden later-gate / L4 field scan on BOTH sides.
        scan_l3_forbidden(
            &golden_json,
            &format!("{fixture}:golden"),
            &mut forbidden_hits,
        );
        scan_l3_forbidden(&rust_json, &format!("{fixture}:rust"), &mut forbidden_hits);

        // Coverage matrices (Rust drives the anti-degenerate gate; golden is the
        // oracle cross-check).
        rust_cov.add(&coverage_of(&rust_json));
        golden_cov.add(&coverage_of(&golden_json));

        // Positional structural diff (both sides already canonically sorted).
        diff_l2_value(fixture, "", &golden_json, &rust_json, &mut all_divergences);
    }

    all_divergences
        .sort_by(|a, b| (a.fixture.as_str(), &a.path).cmp(&(b.fixture.as_str(), &b.path)));

    // --- Forbidden-field guard (hard fail, never allowlistable) -------------
    assert!(
        forbidden_hits.is_empty(),
        "FORBIDDEN later-gate/L4 field(s) leaked into the L3 comparison \
         (golden or rust):\n  {}",
        forbidden_hits.join("\n  ")
    );

    // --- COVERAGE MATRIX gate (anti-degenerate, [REV2]) ---------------------
    eprintln!(
        "R2a L3 coverage matrix (set={set:?}, {} fixture(s)): \
         resolvedRecordVarTableIds={} resolvedRecordOpTableIds={} \
         implicitRecResolutions={} mergedExtensionFields={}",
        goldens.len(),
        rust_cov.resolved_record_var_table_ids,
        rust_cov.resolved_record_op_table_ids,
        rust_cov.implicit_rec_resolutions,
        rust_cov.merged_extension_fields,
    );
    let mut zero_axes: Vec<&str> = Vec::new();
    if rust_cov.resolved_record_var_table_ids == 0 {
        zero_axes.push("resolvedRecordVarTableIds");
    }
    if rust_cov.resolved_record_op_table_ids == 0 {
        zero_axes.push("resolvedRecordOpTableIds");
    }
    if rust_cov.implicit_rec_resolutions == 0 {
        zero_axes.push("implicitRecResolutions");
    }
    if rust_cov.merged_extension_fields == 0 {
        zero_axes.push("mergedExtensionFields");
    }
    assert!(
        zero_axes.is_empty(),
        "DEGENERATE coverage matrix (set={set:?}): axis/axes {zero_axes:?} are ZERO — \
         the L3 port is not actually RESOLVING (unresolved==unresolved would pass a \
         pure equality diff). The matrix must prove resolution fires.",
    );
    // Oracle cross-check: Rust coverage must equal the GOLDEN coverage (al-sem
    // ground truth). A mismatch means resolution diverged even if the structural
    // diff somehow missed it.
    assert_eq!(
        rust_cov, golden_cov,
        "L3 coverage matrix MISMATCH vs golden oracle (set={set:?})\n  rust   = {rust_cov:?}\n  golden = {golden_cov:?}",
    );

    // --- Allowlist gating (same semantics as R0/L2) -------------------------
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
            "\n{} UNDOCUMENTED L3 divergence(s) (not in KNOWN_DIVERGENCES.json, \
             test={L3_TEST_NAME}):\n",
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
            "\n{} UNUSED L3 allowlist entr(y/ies) (no matching divergence this run):\n",
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
        "R2a L3 record-types differential FAILED (set={set:?}):{failure}"
    );

    eprintln!(
        "R2a L3 differential: set={set:?}, {} fixture(s), 0 divergences, \
         allowlist fully consumed ({} L3 entr(y/ies)).",
        goldens.len(),
        allowlist.len()
    );
}

/// Recursively collect every forbidden object-key in `value` (L3 later-gate set),
/// with its JSON pointer path.
fn scan_l3_forbidden(value: &serde_json::Value, path: &str, hits: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let child = format!("{path}.{k}");
                if L3_FORBIDDEN_KEYS.contains(&k.as_str()) {
                    hits.push(child.clone());
                }
                scan_l3_forbidden(v, &child, hits);
            }
        }
        serde_json::Value::Array(arr) => {
            for (i, v) in arr.iter().enumerate() {
                scan_l3_forbidden(v, &format!("{path}[{i}]"), hits);
            }
        }
        _ => {}
    }
}

// ===========================================================================
// R2b — L3 CALL-GRAPH differential pass + the anti-degenerate coverage matrix.
//
// For each `tests/r2b-goldens/*.l3cg.golden.json`, run the Rust disk-backed
// assemble→resolve→project_call_graph and compare. The comparison GROUPS edges
// by callsiteId and compares each group as a SORTED MULTISET of CallEdges (never
// `Map<callsiteId, CallEdge>` — interface dispatch is multi-edge). dispatchMeta
// is compared at the group level; the upgraded argumentBindings per callsite are
// compared too. HARD-FAILS on any forbidden later-gate / L4 field (typedEdges /
// summary / coverage / eventGraph / callsiteResolutions / openWorld /
// capabilityFactsDirect / rootClassifications). KNOWN_DIVERGENCES-gated (empty).
// ===========================================================================

/// Forbidden later-gate / L4 keys that must NEVER appear in the L3 call-graph
/// comparison surface (golden OR rust). Mirrors al-sem `FORBIDDEN_KEYS`.
const L3CG_FORBIDDEN_KEYS: &[&str] = &[
    "typedEdges",
    "summary",
    "coverage",
    "eventGraph",
    "callsiteResolutions",
    "openWorld",
    "capabilityFactsDirect",
    "rootClassifications",
];

const L3CG_TEST_NAME: &str = "differential_l3_call_graph_match_goldens";

/// The expanded R2b coverage matrix axes (al-sem `CoverageCounts`). Driven by Rust;
/// oracle-cross-checked against the al-sem manifest's `coverageMatrix`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct CallGraphCoverage {
    resolved_direct: usize,
    resolved_member: usize,
    object_run_resolved: usize,
    interface_multi_edge: usize,
    interface_edges: usize,
    dynamic_unknown: usize,
    builtin: usize,
    implicit_trigger: usize,
    unresolved_unknown: usize,
    ambiguous: usize,
    member_not_found: usize,
    opaque: usize,
    external_target: usize,
    upgraded_resolved_bindings: usize,
    ambiguous_bindings: usize,
}

impl CallGraphCoverage {
    fn add(&mut self, o: &CallGraphCoverage) {
        self.resolved_direct += o.resolved_direct;
        self.resolved_member += o.resolved_member;
        self.object_run_resolved += o.object_run_resolved;
        self.interface_multi_edge += o.interface_multi_edge;
        self.interface_edges += o.interface_edges;
        self.dynamic_unknown += o.dynamic_unknown;
        self.builtin += o.builtin;
        self.implicit_trigger += o.implicit_trigger;
        self.unresolved_unknown += o.unresolved_unknown;
        self.ambiguous += o.ambiguous;
        self.member_not_found += o.member_not_found;
        self.opaque += o.opaque;
        self.external_target += o.external_target;
        self.upgraded_resolved_bindings += o.upgraded_resolved_bindings;
        self.ambiguous_bindings += o.ambiguous_bindings;
    }
}

/// Count the coverage axes from ONE projection `Value` (golden OR rust — same
/// shape). Faithful port of al-sem `countCoverage` (`scripts/dump-l3-call-graph.ts`).
fn call_graph_coverage_of(proj: &serde_json::Value) -> CallGraphCoverage {
    let mut c = CallGraphCoverage::default();
    let str_of = |v: &serde_json::Value, k: &str| -> String {
        v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
    };
    if let Some(groups) = proj.get("groups").and_then(|g| g.as_array()) {
        for g in groups {
            let edges = g.get("edges").and_then(|e| e.as_array());
            let interface_edges_in_group = edges
                .map(|es| {
                    es.iter()
                        .filter(|e| str_of(e, "dispatchKind") == "interface")
                        .count()
                })
                .unwrap_or(0);
            if interface_edges_in_group > 1 {
                c.interface_multi_edge += 1;
            }
            if let Some(edges) = edges {
                for e in edges {
                    let dk = str_of(e, "dispatchKind");
                    let res = str_of(e, "resolution");
                    match dk.as_str() {
                        "direct" => {
                            if res == "resolved" {
                                c.resolved_direct += 1;
                            }
                        }
                        "method" => match res.as_str() {
                            "resolved" => c.resolved_member += 1,
                            "ambiguous" => c.ambiguous += 1,
                            "member-not-found" => c.member_not_found += 1,
                            "opaque" => c.opaque += 1,
                            "external-target" => c.external_target += 1,
                            _ => {}
                        },
                        "interface" => c.interface_edges += 1,
                        "codeunit-run" | "page-run" | "report-run" => match res.as_str() {
                            "resolved" => c.object_run_resolved += 1,
                            "opaque" => c.opaque += 1,
                            _ => {}
                        },
                        "dynamic" => c.dynamic_unknown += 1,
                        "builtin" => c.builtin += 1,
                        "implicit-trigger" => c.implicit_trigger += 1,
                        "unresolved" => c.unresolved_unknown += 1,
                        _ => {}
                    }
                    // Direct-call ambiguity / member-not-found also surface on "direct".
                    if dk == "direct" && res == "ambiguous" {
                        c.ambiguous += 1;
                    }
                    if dk == "direct" && res == "member-not-found" {
                        c.member_not_found += 1;
                    }
                }
            }
        }
    }
    if let Some(bindings) = proj.get("bindings").and_then(|b| b.as_array()) {
        for site in bindings {
            if let Some(bs) = site.get("bindings").and_then(|b| b.as_array()) {
                for ab in bs {
                    match str_of(ab, "bindingResolution").as_str() {
                        "resolved" => c.upgraded_resolved_bindings += 1,
                        "ambiguous" => c.ambiguous_bindings += 1,
                        _ => {}
                    }
                }
            }
        }
    }
    c
}

/// Discover every `tests/r2b-goldens/*.l3cg.golden.json`, sorted by fixture name.
fn discover_l3cg_goldens() -> Vec<(String, PathBuf)> {
    let dir = repo_root().join("tests").join("r2b-goldens");
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("failed to read L3cg goldens dir {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".l3cg.golden.json") {
            continue; // skips manifest.json, l3cg-vectors.json
        }
        let fixture = name.trim_end_matches(".l3cg.golden.json").to_string();
        out.push((fixture, entry.path()));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Canonical, ORDER-INDEPENDENT multiset comparison of two callsite-group lists.
/// Groups keyed by `callsiteId`; for each group the edges array is compared as a
/// SORTED MULTISET (sorted by the compact JSON of the edge), and the group-level
/// `dispatchMeta` is compared structurally. This explicitly NEVER collapses a
/// callsite to one edge. Emits one divergence per mismatch (with a `path` locator).
fn diff_l3cg(
    fixture: &str,
    golden: &serde_json::Value,
    rust: &serde_json::Value,
    out: &mut Vec<Divergence>,
) {
    // --- groups: keyed by callsiteId. ---
    let gmap = |v: &serde_json::Value| -> BTreeMap<String, serde_json::Value> {
        let mut m = BTreeMap::new();
        if let Some(arr) = v.get("groups").and_then(|g| g.as_array()) {
            for g in arr {
                let id = g
                    .get("callsiteId")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string();
                m.insert(id, g.clone());
            }
        }
        m
    };
    let gg = gmap(golden);
    let rg = gmap(rust);

    for (id, g) in &gg {
        match rg.get(id) {
            None => out.push(Divergence {
                fixture: fixture.to_string(),
                path: format!("groups[{id:?}]:MISSING_IN_RUST"),
                golden_value: compact(g),
                rust_value: "<absent>".to_string(),
            }),
            Some(r) => {
                // Edges as a sorted multiset (compact-JSON keyed).
                let mut ge = edge_multiset(g);
                let mut re = edge_multiset(r);
                ge.sort();
                re.sort();
                if ge != re {
                    out.push(Divergence {
                        fixture: fixture.to_string(),
                        path: format!("groups[{id:?}].edges"),
                        golden_value: format!("[{}]", ge.join(", ")),
                        rust_value: format!("[{}]", re.join(", ")),
                    });
                }
                // dispatchMeta (group level).
                let gm = g
                    .get("dispatchMeta")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let rm = r
                    .get("dispatchMeta")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                if gm != rm {
                    out.push(Divergence {
                        fixture: fixture.to_string(),
                        path: format!("groups[{id:?}].dispatchMeta"),
                        golden_value: compact(&gm),
                        rust_value: compact(&rm),
                    });
                }
            }
        }
    }
    for id in rg.keys() {
        if !gg.contains_key(id) {
            out.push(Divergence {
                fixture: fixture.to_string(),
                path: format!("groups[{id:?}]:EXTRA_IN_RUST"),
                golden_value: "<absent>".to_string(),
                rust_value: compact(&rg[id]),
            });
        }
    }

    // --- bindings: keyed by callsiteId, compared structurally. ---
    let bmap = |v: &serde_json::Value| -> BTreeMap<String, serde_json::Value> {
        let mut m = BTreeMap::new();
        if let Some(arr) = v.get("bindings").and_then(|b| b.as_array()) {
            for b in arr {
                let id = b
                    .get("callsiteId")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string();
                m.insert(
                    id,
                    b.get("bindings")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                );
            }
        }
        m
    };
    let gb = bmap(golden);
    let rb = bmap(rust);
    for (id, g) in &gb {
        match rb.get(id) {
            None => out.push(Divergence {
                fixture: fixture.to_string(),
                path: format!("bindings[{id:?}]:MISSING_IN_RUST"),
                golden_value: compact(g),
                rust_value: "<absent>".to_string(),
            }),
            Some(r) if r != g => out.push(Divergence {
                fixture: fixture.to_string(),
                path: format!("bindings[{id:?}]"),
                golden_value: compact(g),
                rust_value: compact(r),
            }),
            _ => {}
        }
    }
    for id in rb.keys() {
        if !gb.contains_key(id) {
            out.push(Divergence {
                fixture: fixture.to_string(),
                path: format!("bindings[{id:?}]:EXTRA_IN_RUST"),
                golden_value: "<absent>".to_string(),
                rust_value: compact(&rb[id]),
            });
        }
    }
}

/// The compact-JSON-per-edge multiset of a group's edges (NOT collapsed).
fn edge_multiset(group: &serde_json::Value) -> Vec<String> {
    group
        .get("edges")
        .and_then(|e| e.as_array())
        .map(|arr| arr.iter().map(compact).collect())
        .unwrap_or_default()
}

/// Recursively collect every forbidden later-gate object-key in `value`.
fn scan_l3cg_forbidden(value: &serde_json::Value, path: &str, hits: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let child = format!("{path}.{k}");
                if L3CG_FORBIDDEN_KEYS.contains(&k.as_str()) {
                    hits.push(child.clone());
                }
                scan_l3cg_forbidden(v, &child, hits);
            }
        }
        serde_json::Value::Array(arr) => {
            for (i, v) in arr.iter().enumerate() {
                scan_l3cg_forbidden(v, &format!("{path}[{i}]"), hits);
            }
        }
        _ => {}
    }
}

/// The R2b L3 call-graph differential pass + the expanded coverage matrix. Gated by
/// `R2B_L3CG_SET` (committed default `full`: the whole 155-fixture corpus — the R2b
/// EXIT GATE; `small` = ws-d2 + ws-interface-dispatch + ws-r2b-external for dev
/// iteration).
#[test]
fn differential_l3_call_graph_match_goldens() {
    let all_goldens = discover_l3cg_goldens();
    assert!(
        !all_goldens.is_empty(),
        "no L3cg goldens discovered under tests/r2b-goldens — corpus missing?"
    );

    let set = std::env::var("R2B_L3CG_SET").unwrap_or_else(|_| "full".to_string());
    let small_set = ["ws-d2", "ws-interface-dispatch", "ws-r2b-external"];
    let goldens: Vec<(String, PathBuf)> = match set.as_str() {
        "full" | "" => all_goldens,
        "small" => all_goldens
            .into_iter()
            .filter(|(f, _)| small_set.contains(&f.as_str()))
            .collect(),
        other => panic!("R2B_L3CG_SET={other:?} not recognized (expected `small` or `full`)"),
    };
    assert!(
        !goldens.is_empty(),
        "R2B_L3CG_SET={set:?} selected zero fixtures (small set = {small_set:?})"
    );

    let allowlist: Vec<AllowEntry> = load_allowlist()
        .into_iter()
        .filter(|e| e.test == L3CG_TEST_NAME)
        .collect();

    let mut all_divergences: Vec<Divergence> = Vec::new();
    let mut forbidden_hits: Vec<String> = Vec::new();
    let mut rust_cov = CallGraphCoverage::default();
    let mut golden_cov = CallGraphCoverage::default();

    for (fixture, golden_path) in &goldens {
        let fixture_dir = corpus_dir().join(fixture);
        assert!(
            fixture_dir.is_dir(),
            "L3cg golden {} has no matching in-repo fixture at {} (offline corpus incomplete)",
            golden_path.display(),
            fixture_dir.display()
        );

        // Golden side: parse as JSON (for the diff) AND validate it parses as the
        // allowlisted L3CallGraphProjection serde type (shape guard).
        let golden_text = std::fs::read_to_string(golden_path)
            .unwrap_or_else(|e| panic!("read L3cg golden {}: {e}", golden_path.display()));
        let golden_json: serde_json::Value =
            serde_json::from_str(&golden_text).unwrap_or_else(|e| {
                panic!(
                    "L3cg golden {} is not valid JSON: {e}",
                    golden_path.display()
                )
            });
        let _: L3CallGraphProjection =
            serde_json::from_value(golden_json.clone()).unwrap_or_else(|e| {
                panic!(
                    "L3cg golden {} does not parse as L3CallGraphProjection: {e}",
                    golden_path.display()
                )
            });

        // Rust side: disk-backed assemble+resolve → project_call_graph. Fail-closed
        // (empty) layouts yield an empty projection (never throws).
        let projection = match assemble_and_resolve_workspace_default(&fixture_dir) {
            Some(resolved) => resolved.project_call_graph(),
            None => L3CallGraphProjection {
                groups: vec![],
                bindings: vec![],
            },
        };
        let rust_json = serde_json::to_value(&projection)
            .unwrap_or_else(|e| panic!("serialize Rust L3cg projection for {fixture}: {e}"));

        // Forbidden later-gate / L4 field scan on BOTH sides (hard fail).
        scan_l3cg_forbidden(
            &golden_json,
            &format!("{fixture}:golden"),
            &mut forbidden_hits,
        );
        scan_l3cg_forbidden(&rust_json, &format!("{fixture}:rust"), &mut forbidden_hits);

        // Coverage (Rust drives the anti-degenerate gate; golden is the oracle).
        rust_cov.add(&call_graph_coverage_of(&rust_json));
        golden_cov.add(&call_graph_coverage_of(&golden_json));

        // Order-independent multiset group + binding compare.
        diff_l3cg(fixture, &golden_json, &rust_json, &mut all_divergences);
    }

    all_divergences
        .sort_by(|a, b| (a.fixture.as_str(), &a.path).cmp(&(b.fixture.as_str(), &b.path)));

    // --- Forbidden-field guard (hard fail, never allowlistable) -------------
    assert!(
        forbidden_hits.is_empty(),
        "FORBIDDEN later-gate/L4 field(s) leaked into the L3 call-graph comparison \
         (golden or rust):\n  {}",
        forbidden_hits.join("\n  ")
    );

    // --- COVERAGE MATRIX gate (anti-degenerate, expanded [REV2]) ------------
    eprintln!(
        "R2b L3cg coverage matrix (set={set:?}, {} fixture(s)):\n  \
         resolvedDirect={} resolvedMember={} objectRunResolved={} interfaceMultiEdge={} \
         interfaceEdges={} dynamicUnknown={} builtin={} implicitTrigger={} \
         unresolvedUnknown={} ambiguous={} memberNotFound={} opaque={} externalTarget={} \
         upgradedResolvedBindings={} ambiguousBindings={}",
        goldens.len(),
        rust_cov.resolved_direct,
        rust_cov.resolved_member,
        rust_cov.object_run_resolved,
        rust_cov.interface_multi_edge,
        rust_cov.interface_edges,
        rust_cov.dynamic_unknown,
        rust_cov.builtin,
        rust_cov.implicit_trigger,
        rust_cov.unresolved_unknown,
        rust_cov.ambiguous,
        rust_cov.member_not_found,
        rust_cov.opaque,
        rust_cov.external_target,
        rust_cov.upgraded_resolved_bindings,
        rust_cov.ambiguous_bindings,
    );

    // Fail-on-zero per axis ONLY for the full corpus (the small dev set cannot
    // populate every axis). NOTE on member-opaque (plan Task 4): in the bare
    // `assemble→resolve→project` dump path `has_unfetched_declared_dependency` is
    // always false (no `.app` deps fetched), so the member-call "opaque" branch is
    // structurally UNREACHABLE — every missing member object is `external-target`.
    // `opaque` is therefore populated solely by OBJECT-RUN misses (always opaque),
    // which the corpus DOES exercise, so the `opaque` axis is still enforced. The
    // `external-target` axis is enforced as the plan requires.
    if set == "full" {
        let axes: [(&str, usize); 15] = [
            ("resolvedDirect", rust_cov.resolved_direct),
            ("resolvedMember", rust_cov.resolved_member),
            ("objectRunResolved", rust_cov.object_run_resolved),
            ("interfaceMultiEdge", rust_cov.interface_multi_edge),
            ("interfaceEdges", rust_cov.interface_edges),
            ("dynamicUnknown", rust_cov.dynamic_unknown),
            ("builtin", rust_cov.builtin),
            ("implicitTrigger", rust_cov.implicit_trigger),
            ("unresolvedUnknown", rust_cov.unresolved_unknown),
            ("ambiguous", rust_cov.ambiguous),
            ("memberNotFound", rust_cov.member_not_found),
            ("opaque", rust_cov.opaque),
            ("externalTarget", rust_cov.external_target),
            (
                "upgradedResolvedBindings",
                rust_cov.upgraded_resolved_bindings,
            ),
            ("ambiguousBindings", rust_cov.ambiguous_bindings),
        ];
        let zero_axes: Vec<&str> = axes
            .iter()
            .filter(|(_, n)| *n == 0)
            .map(|(name, _)| *name)
            .collect();
        assert!(
            zero_axes.is_empty(),
            "DEGENERATE L3cg coverage matrix (set={set:?}): axis/axes {zero_axes:?} are ZERO — \
             the R2b port is not actually RESOLVING that case (unresolved==unresolved would pass \
             a pure equality diff). The matrix must prove resolution fires.",
        );
    }

    // Oracle cross-check: Rust coverage MUST equal the golden coverage (al-sem
    // ground truth) for the SAME fixture set. (For the full set this also equals the
    // manifest `coverageMatrix`; see the dedicated oracle test below.)
    assert_eq!(
        rust_cov, golden_cov,
        "L3cg coverage matrix MISMATCH vs golden oracle (set={set:?})\n  rust   = {rust_cov:?}\n  golden = {golden_cov:?}",
    );

    // --- Allowlist gating (same semantics as R0/L2/R2a) ---------------------
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
            "\n{} UNDOCUMENTED L3cg divergence(s) (not in KNOWN_DIVERGENCES.json, \
             test={L3CG_TEST_NAME}):\n",
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
            "\n{} UNUSED L3cg allowlist entr(y/ies) (no matching divergence this run):\n",
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
        "R2b L3 call-graph differential FAILED (set={set:?}):{failure}"
    );

    eprintln!(
        "R2b L3cg differential: set={set:?}, {} fixture(s), 0 divergences, \
         allowlist fully consumed ({} L3cg entr(y/ies)).",
        goldens.len(),
        allowlist.len()
    );
}

/// Oracle cross-check: the FULL-corpus Rust coverage matrix must equal the
/// al-sem manifest's published `coverageMatrix` (the ground-truth totals). This is
/// independent of the per-fixture golden compare — it guards the matrix counters
/// themselves against drift.
#[test]
fn l3cg_coverage_matrix_matches_manifest_oracle() {
    let goldens = discover_l3cg_goldens();
    assert!(!goldens.is_empty(), "no L3cg goldens — corpus missing?");

    let mut rust_cov = CallGraphCoverage::default();
    for (fixture, _) in &goldens {
        let fixture_dir = corpus_dir().join(fixture);
        let projection = match assemble_and_resolve_workspace_default(&fixture_dir) {
            Some(resolved) => resolved.project_call_graph(),
            None => L3CallGraphProjection {
                groups: vec![],
                bindings: vec![],
            },
        };
        let rust_json = serde_json::to_value(&projection).expect("serialize");
        rust_cov.add(&call_graph_coverage_of(&rust_json));
    }

    let manifest_path = repo_root()
        .join("tests")
        .join("r2b-goldens")
        .join("manifest.json");
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).expect("read r2b manifest"))
            .expect("parse r2b manifest");
    let m = manifest
        .get("coverageMatrix")
        .expect("manifest has coverageMatrix");
    let mget =
        |k: &str| -> usize { m.get(k).and_then(|v| v.as_u64()).unwrap_or(u64::MAX) as usize };
    let manifest_cov = CallGraphCoverage {
        resolved_direct: mget("resolvedDirect"),
        resolved_member: mget("resolvedMember"),
        object_run_resolved: mget("objectRunResolved"),
        interface_multi_edge: mget("interfaceMultiEdge"),
        interface_edges: mget("interfaceEdges"),
        dynamic_unknown: mget("dynamicUnknown"),
        builtin: mget("builtin"),
        implicit_trigger: mget("implicitTrigger"),
        unresolved_unknown: mget("unresolvedUnknown"),
        ambiguous: mget("ambiguous"),
        member_not_found: mget("memberNotFound"),
        opaque: mget("opaque"),
        external_target: mget("externalTarget"),
        upgraded_resolved_bindings: mget("upgradedResolvedBindings"),
        ambiguous_bindings: mget("ambiguousBindings"),
    };
    assert_eq!(
        rust_cov, manifest_cov,
        "R2b coverage matrix MISMATCH vs al-sem manifest oracle\n  rust     = {rust_cov:?}\n  manifest = {manifest_cov:?}",
    );
    eprintln!(
        "R2b coverage matrix oracle: Rust full-corpus totals == al-sem manifest coverageMatrix."
    );
}

// ===========================================================================
// R2c — L3 EVENT-GRAPH differential pass + the anti-degenerate coverage matrix.
//
// For each `tests/r2c-goldens/*.l3eg.golden.json`, run the Rust disk-backed
// assemble→resolve→build_event_graph→project_event_graph and compare. EventSymbols
// are keyed by their stable `id`; EventEdges by `(eventId, subscriberRoutineId)` —
// both already deterministically sorted by the projection, so the compare is
// positional/structural after keying. HARD-FAILS on any forbidden later-gate / L4
// field (callGraph / typedEdges / summary / coverage / publish / capability*).
// KNOWN_DIVERGENCES-gated (empty).
//
// ## The 31-event-fixtures-vs-corpus inclusion rule
//
// al-sem's dump EXCLUDES event-less fixtures: it emitted goldens ONLY for the 31
// fixtures whose RESOLVED event graph is non-empty (>=1 publisher or subscriber),
// listing the other 132 under `manifest.exclusions` ("no event graph"). The Rust
// emitter produces an EMPTY `{events:[], edges:[]}` for every event-less fixture,
// so the inclusion rule is reproduced as: compare the 31 fixtures WITH a golden
// structurally, AND additionally enforce that EVERY corpus fixture WITHOUT a golden
// projects to an empty event graph (a non-empty event graph for a non-golden
// fixture would be an inclusion divergence — the Rust port inventing events al-sem
// did not). This guards both directions of the 31-vs-163 mismatch.
//
// ## COVERAGE MATRIX (anti-degenerate, plan Task 3 / Rev 2 §6)
//
// Across the 31 goldens the pass computes + ENFORCES nonzero counts of the 8 al-sem
// `CoverageCounts` axes (`scripts/dump-l3-event-graph.ts`):
//   integrationPublishers / businessPublishers / unknownKindSymbols (synthesized
//   maybe+unknown symbols) / isolatedPublishers / symbolsWithElementName /
//   resolvedEdges / maybeEdges / unknownEdges.
// Driven by the RUST projection (proves the port actually CLASSIFIES, not
// "empty==empty"); fail-on-zero per axis; an oracle cross-check asserts the totals
// equal BOTH the per-golden recomputation AND the al-sem manifest `coverageMatrix`.

/// Forbidden later-gate / L4 keys that must NEVER appear in the L3 event-graph
/// comparison surface (golden OR rust). The event graph is R2c's surface; the call
/// graph (R2b) is a SEPARATE pass, and summaries/coverage/publish/typedEdges are
/// later gates. Mirrors the manifest `forbiddenKeys`.
const L3EG_FORBIDDEN_KEYS: &[&str] = &[
    // call-graph surface (R2b — a separate pass)
    "callsiteId",
    "dispatchKind",
    "dispatchMeta",
    "argumentBindings",
    "groups",
    "bindings",
    "callsiteResolutions",
    // later-gate / L4
    "typedEdges",
    "summary",
    "coverage",
    "publish",
    "capabilityFactsDirect",
    "rootClassifications",
];

const L3EG_TEST_NAME: &str = "differential_l3_event_graph_match_goldens";

/// The 8 al-sem event-graph coverage axes (`CoverageCounts`). Driven by Rust;
/// oracle-cross-checked against the al-sem manifest's `coverageMatrix`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct EventGraphCoverage {
    integration_publishers: usize,
    business_publishers: usize,
    /// Synthesized maybe/unknown symbols (eventKind neither integration nor business).
    unknown_kind_symbols: usize,
    isolated_publishers: usize,
    symbols_with_element_name: usize,
    resolved_edges: usize,
    maybe_edges: usize,
    unknown_edges: usize,
}

impl EventGraphCoverage {
    fn add(&mut self, o: &EventGraphCoverage) {
        self.integration_publishers += o.integration_publishers;
        self.business_publishers += o.business_publishers;
        self.unknown_kind_symbols += o.unknown_kind_symbols;
        self.isolated_publishers += o.isolated_publishers;
        self.symbols_with_element_name += o.symbols_with_element_name;
        self.resolved_edges += o.resolved_edges;
        self.maybe_edges += o.maybe_edges;
        self.unknown_edges += o.unknown_edges;
    }
}

/// Count the 8 coverage axes from ONE projection `Value` (golden OR rust — same
/// shape). Faithful port of al-sem `countCoverage` (`scripts/dump-l3-event-graph.ts`).
fn event_graph_coverage_of(proj: &serde_json::Value) -> EventGraphCoverage {
    let mut c = EventGraphCoverage::default();
    let str_of = |v: &serde_json::Value, k: &str| -> String {
        v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
    };
    if let Some(events) = proj.get("events").and_then(|e| e.as_array()) {
        for s in events {
            match str_of(s, "eventKind").as_str() {
                "integration" => c.integration_publishers += 1,
                "business" => c.business_publishers += 1,
                _ => c.unknown_kind_symbols += 1,
            }
            if s.get("isolated").and_then(|v| v.as_bool()) == Some(true) {
                c.isolated_publishers += 1;
            }
            if s.get("elementName").is_some() {
                c.symbols_with_element_name += 1;
            }
        }
    }
    if let Some(edges) = proj.get("edges").and_then(|e| e.as_array()) {
        for e in edges {
            match str_of(e, "resolution").as_str() {
                "resolved" => c.resolved_edges += 1,
                "maybe" => c.maybe_edges += 1,
                _ => c.unknown_edges += 1,
            }
        }
    }
    c
}

/// Discover every `tests/r2c-goldens/*.l3eg.golden.json`, sorted by fixture name.
fn discover_l3eg_goldens() -> Vec<(String, PathBuf)> {
    let dir = repo_root().join("tests").join("r2c-goldens");
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("failed to read L3eg goldens dir {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".l3eg.golden.json") {
            continue; // skips manifest.json
        }
        let fixture = name.trim_end_matches(".l3eg.golden.json").to_string();
        out.push((fixture, entry.path()));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Rust-side L3 event-graph projection for a fixture dir (fail-closed → empty).
fn rust_event_graph_projection(fixture_dir: &Path) -> L3EventGraphProjection {
    match assemble_and_resolve_workspace_default(fixture_dir) {
        Some(resolved) => resolved.project_event_graph(),
        None => L3EventGraphProjection {
            events: vec![],
            edges: vec![],
        },
    }
}

/// Structural diff of two L3 event-graph projection `Value`s. EventSymbols keyed by
/// `id`, EventEdges keyed by `(eventId, subscriberRoutineId)`. Each side's arrays
/// are already deterministically sorted by the projection, so keying detects
/// MISSING / EXTRA cleanly and the per-key structural compare catches field drift.
fn diff_l3eg(
    fixture: &str,
    golden: &serde_json::Value,
    rust: &serde_json::Value,
    out: &mut Vec<Divergence>,
) {
    // --- events: keyed by stable id. ---
    let emap = |v: &serde_json::Value| -> BTreeMap<String, serde_json::Value> {
        let mut m = BTreeMap::new();
        if let Some(arr) = v.get("events").and_then(|e| e.as_array()) {
            for s in arr {
                let id = s
                    .get("id")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string();
                m.insert(id, s.clone());
            }
        }
        m
    };
    let ge = emap(golden);
    let re = emap(rust);
    for (id, g) in &ge {
        match re.get(id) {
            None => out.push(Divergence {
                fixture: fixture.to_string(),
                path: format!("events[{id:?}]:MISSING_IN_RUST"),
                golden_value: compact(g),
                rust_value: "<absent>".to_string(),
            }),
            Some(r) => diff_l2_value(fixture, &format!("events[{id:?}]"), g, r, out),
        }
    }
    for id in re.keys() {
        if !ge.contains_key(id) {
            out.push(Divergence {
                fixture: fixture.to_string(),
                path: format!("events[{id:?}]:EXTRA_IN_RUST"),
                golden_value: "<absent>".to_string(),
                rust_value: compact(&re[id]),
            });
        }
    }

    // --- edges: keyed by (eventId, subscriberRoutineId). ---
    let dmap = |v: &serde_json::Value| -> BTreeMap<String, serde_json::Value> {
        let mut m = BTreeMap::new();
        if let Some(arr) = v.get("edges").and_then(|e| e.as_array()) {
            for e in arr {
                let ev = e
                    .get("eventId")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string();
                let sub = e
                    .get("subscriberRoutineId")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string();
                m.insert(format!("{ev}\u{1f}{sub}"), e.clone());
            }
        }
        m
    };
    let gd = dmap(golden);
    let rd = dmap(rust);
    for (key, g) in &gd {
        match rd.get(key) {
            None => out.push(Divergence {
                fixture: fixture.to_string(),
                path: format!("edges[{key:?}]:MISSING_IN_RUST"),
                golden_value: compact(g),
                rust_value: "<absent>".to_string(),
            }),
            Some(r) => diff_l2_value(fixture, &format!("edges[{key:?}]"), g, r, out),
        }
    }
    for key in rd.keys() {
        if !gd.contains_key(key) {
            out.push(Divergence {
                fixture: fixture.to_string(),
                path: format!("edges[{key:?}]:EXTRA_IN_RUST"),
                golden_value: "<absent>".to_string(),
                rust_value: compact(&rd[key]),
            });
        }
    }
}

/// Recursively collect every forbidden later-gate object-key in `value` (L3eg set).
fn scan_l3eg_forbidden(value: &serde_json::Value, path: &str, hits: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let child = format!("{path}.{k}");
                if L3EG_FORBIDDEN_KEYS.contains(&k.as_str()) {
                    hits.push(child.clone());
                }
                scan_l3eg_forbidden(v, &child, hits);
            }
        }
        serde_json::Value::Array(arr) => {
            for (i, v) in arr.iter().enumerate() {
                scan_l3eg_forbidden(v, &format!("{path}[{i}]"), hits);
            }
        }
        _ => {}
    }
}

/// The R2c L3 event-graph differential pass + the 8-axis coverage matrix. Gated by
/// `R2C_L3EG_SET` (committed default `full`: all 31 event-bearing goldens — the R2c
/// EXIT GATE; `small` = ws-d2 + ws-r2c-mixed for dev iteration). On the FULL set it
/// ALSO enforces the inclusion rule: every corpus fixture WITHOUT a golden must
/// project to an empty event graph.
#[test]
fn differential_l3_event_graph_match_goldens() {
    let all_goldens = discover_l3eg_goldens();
    assert!(
        !all_goldens.is_empty(),
        "no L3eg goldens discovered under tests/r2c-goldens — corpus missing?"
    );

    let set = std::env::var("R2C_L3EG_SET").unwrap_or_else(|_| "full".to_string());
    let small_set = ["ws-d2", "ws-r2c-mixed"];
    let goldens: Vec<(String, PathBuf)> = match set.as_str() {
        "full" | "" => all_goldens,
        "small" => all_goldens
            .into_iter()
            .filter(|(f, _)| small_set.contains(&f.as_str()))
            .collect(),
        other => panic!("R2C_L3EG_SET={other:?} not recognized (expected `small` or `full`)"),
    };
    assert!(
        !goldens.is_empty(),
        "R2C_L3EG_SET={set:?} selected zero fixtures (small set = {small_set:?})"
    );

    let allowlist: Vec<AllowEntry> = load_allowlist()
        .into_iter()
        .filter(|e| e.test == L3EG_TEST_NAME)
        .collect();

    let mut all_divergences: Vec<Divergence> = Vec::new();
    let mut forbidden_hits: Vec<String> = Vec::new();
    let mut rust_cov = EventGraphCoverage::default();
    let mut golden_cov = EventGraphCoverage::default();

    for (fixture, golden_path) in &goldens {
        let fixture_dir = corpus_dir().join(fixture);
        assert!(
            fixture_dir.is_dir(),
            "L3eg golden {} has no matching in-repo fixture at {} (offline corpus incomplete)",
            golden_path.display(),
            fixture_dir.display()
        );

        // Golden side: parse as JSON (for the diff) AND validate it parses as the
        // allowlisted L3EventGraphProjection serde type (shape guard).
        let golden_text = std::fs::read_to_string(golden_path)
            .unwrap_or_else(|e| panic!("read L3eg golden {}: {e}", golden_path.display()));
        let golden_json: serde_json::Value =
            serde_json::from_str(&golden_text).unwrap_or_else(|e| {
                panic!(
                    "L3eg golden {} is not valid JSON: {e}",
                    golden_path.display()
                )
            });
        let _: L3EventGraphProjection =
            serde_json::from_value(golden_json.clone()).unwrap_or_else(|e| {
                panic!(
                    "L3eg golden {} does not parse as L3EventGraphProjection: {e}",
                    golden_path.display()
                )
            });

        // Rust side: disk-backed assemble+resolve → project_event_graph → JSON.
        let projection = rust_event_graph_projection(&fixture_dir);
        let rust_json = serde_json::to_value(&projection)
            .unwrap_or_else(|e| panic!("serialize Rust L3eg projection for {fixture}: {e}"));

        // Forbidden later-gate / L4 field scan on BOTH sides (hard fail).
        scan_l3eg_forbidden(
            &golden_json,
            &format!("{fixture}:golden"),
            &mut forbidden_hits,
        );
        scan_l3eg_forbidden(&rust_json, &format!("{fixture}:rust"), &mut forbidden_hits);

        // Coverage (Rust drives the anti-degenerate gate; golden is the oracle).
        rust_cov.add(&event_graph_coverage_of(&rust_json));
        golden_cov.add(&event_graph_coverage_of(&golden_json));

        // Keyed structural compare (events by id, edges by (eventId, subscriber)).
        diff_l3eg(fixture, &golden_json, &rust_json, &mut all_divergences);
    }

    // --- Inclusion-rule guard (FULL set only): every corpus fixture WITHOUT a
    //     golden must project to an EMPTY event graph (al-sem excluded the 132
    //     event-less fixtures; a non-empty graph here would be an invented event). -
    if set == "full" || set.is_empty() {
        let golden_set: std::collections::HashSet<&str> =
            goldens.iter().map(|(f, _)| f.as_str()).collect();
        let entries = std::fs::read_dir(corpus_dir()).expect("read corpus dir");
        let mut corpus: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        corpus.sort();
        for fixture in &corpus {
            if golden_set.contains(fixture.as_str()) {
                continue;
            }
            let proj = rust_event_graph_projection(&corpus_dir().join(fixture));
            if !proj.events.is_empty() || !proj.edges.is_empty() {
                all_divergences.push(Divergence {
                    fixture: fixture.clone(),
                    path: "NON_GOLDEN_FIXTURE_PRODUCED_EVENTS".to_string(),
                    golden_value: "<empty event graph (excluded by al-sem)>".to_string(),
                    rust_value: format!("events={} edges={}", proj.events.len(), proj.edges.len()),
                });
            }
        }
    }

    all_divergences
        .sort_by(|a, b| (a.fixture.as_str(), &a.path).cmp(&(b.fixture.as_str(), &b.path)));

    // --- Forbidden-field guard (hard fail, never allowlistable) -------------
    assert!(
        forbidden_hits.is_empty(),
        "FORBIDDEN later-gate/L4 field(s) leaked into the L3 event-graph comparison \
         (golden or rust):\n  {}",
        forbidden_hits.join("\n  ")
    );

    // --- COVERAGE MATRIX gate (anti-degenerate, fail-on-zero) ---------------
    eprintln!(
        "R2c L3eg coverage matrix (set={set:?}, {} fixture(s)):\n  \
         integrationPublishers={} businessPublishers={} unknownKindSymbols={} \
         isolatedPublishers={} symbolsWithElementName={} resolvedEdges={} \
         maybeEdges={} unknownEdges={}",
        goldens.len(),
        rust_cov.integration_publishers,
        rust_cov.business_publishers,
        rust_cov.unknown_kind_symbols,
        rust_cov.isolated_publishers,
        rust_cov.symbols_with_element_name,
        rust_cov.resolved_edges,
        rust_cov.maybe_edges,
        rust_cov.unknown_edges,
    );
    // Fail-on-zero per axis ONLY for the full corpus (the small dev set cannot
    // populate every axis — e.g. business publishers / element names are rare).
    if set == "full" || set.is_empty() {
        let axes: [(&str, usize); 8] = [
            ("integrationPublishers", rust_cov.integration_publishers),
            ("businessPublishers", rust_cov.business_publishers),
            ("unknownKindSymbols", rust_cov.unknown_kind_symbols),
            ("isolatedPublishers", rust_cov.isolated_publishers),
            ("symbolsWithElementName", rust_cov.symbols_with_element_name),
            ("resolvedEdges", rust_cov.resolved_edges),
            ("maybeEdges", rust_cov.maybe_edges),
            ("unknownEdges", rust_cov.unknown_edges),
        ];
        let zero_axes: Vec<&str> = axes
            .iter()
            .filter(|(_, n)| *n == 0)
            .map(|(name, _)| *name)
            .collect();
        assert!(
            zero_axes.is_empty(),
            "DEGENERATE L3eg coverage matrix (set={set:?}): axis/axes {zero_axes:?} are ZERO — \
             the R2c port is not actually CLASSIFYING that case (empty==empty would pass a pure \
             equality diff). The matrix must prove the event-graph build fires.",
        );
    }

    // Oracle cross-check: Rust coverage MUST equal the golden coverage (al-sem
    // ground truth) for the SAME fixture set.
    assert_eq!(
        rust_cov, golden_cov,
        "L3eg coverage matrix MISMATCH vs golden oracle (set={set:?})\n  rust   = {rust_cov:?}\n  golden = {golden_cov:?}",
    );

    // --- Allowlist gating (same semantics as R0/L2/R2a/R2b) -----------------
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
            "\n{} UNDOCUMENTED L3eg divergence(s) (not in KNOWN_DIVERGENCES.json, \
             test={L3EG_TEST_NAME}):\n",
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
            "\n{} UNUSED L3eg allowlist entr(y/ies) (no matching divergence this run):\n",
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
        "R2c L3 event-graph differential FAILED (set={set:?}):{failure}"
    );

    eprintln!(
        "R2c L3eg differential: set={set:?}, {} fixture(s), 0 divergences, \
         allowlist fully consumed ({} L3eg entr(y/ies)).",
        goldens.len(),
        allowlist.len()
    );
}

/// Oracle cross-check: the FULL-corpus Rust event-graph coverage matrix must equal
/// the al-sem manifest's published `coverageMatrix` (the ground-truth totals). This
/// is independent of the per-fixture golden compare — it guards the matrix counters
/// themselves against drift.
#[test]
fn l3eg_coverage_matrix_matches_manifest_oracle() {
    let goldens = discover_l3eg_goldens();
    assert!(!goldens.is_empty(), "no L3eg goldens — corpus missing?");

    let mut rust_cov = EventGraphCoverage::default();
    for (fixture, _) in &goldens {
        let proj = rust_event_graph_projection(&corpus_dir().join(fixture));
        let rust_json = serde_json::to_value(&proj).expect("serialize");
        rust_cov.add(&event_graph_coverage_of(&rust_json));
    }

    let manifest_path = repo_root()
        .join("tests")
        .join("r2c-goldens")
        .join("manifest.json");
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).expect("read r2c manifest"))
            .expect("parse r2c manifest");
    let m = manifest
        .get("coverageMatrix")
        .expect("manifest has coverageMatrix");
    let mget =
        |k: &str| -> usize { m.get(k).and_then(|v| v.as_u64()).unwrap_or(u64::MAX) as usize };
    let manifest_cov = EventGraphCoverage {
        integration_publishers: mget("integrationPublishers"),
        business_publishers: mget("businessPublishers"),
        unknown_kind_symbols: mget("unknownKindSymbols"),
        isolated_publishers: mget("isolatedPublishers"),
        symbols_with_element_name: mget("symbolsWithElementName"),
        resolved_edges: mget("resolvedEdges"),
        maybe_edges: mget("maybeEdges"),
        unknown_edges: mget("unknownEdges"),
    };
    assert_eq!(
        rust_cov, manifest_cov,
        "R2c coverage matrix MISMATCH vs al-sem manifest oracle\n  rust     = {rust_cov:?}\n  manifest = {manifest_cov:?}",
    );
    eprintln!(
        "R2c coverage matrix oracle: Rust full-corpus totals == al-sem manifest coverageMatrix."
    );
}

// ===========================================================================
// R2d — L3 COVERAGE differential pass + the anti-degenerate coverage matrix.
//
// For each `tests/r2d-goldens/*.l3cov.golden.json`, run the Rust disk-backed
// assemble→resolve→project_coverage_disk and compare the projected
// AnalysisCoverage structurally. The projection is a single flat object; its
// multisets (unresolvedCallsites / dynamicDispatchSites) + the
// routinesParseIncomplete list are sorted by the projection, so a POSITIONAL
// array compare (after sort) detects any cardinality OR id divergence (duplicates
// are preserved on BOTH sides — never deduped). HARD-FAILS on any forbidden
// later-gate / L4 field (callGraph / eventGraph / typedEdges / summary /
// analysisGaps / …). KNOWN_DIVERGENCES-gated (empty at R2d exit).
//
// Every fixture has a golden (coverage is non-empty for every source workspace),
// so there is NO inclusion rule (unlike R2c's event graph) — all 158 compare.
//
// ## COVERAGE MATRIX (anti-degenerate, plan Task 3 / Rev 2 §6)
//
// Across the 158 goldens the pass computes + ENFORCES the al-sem manifest's
// `coverageMatrix` axes, driven by the RUST projection (proves the port actually
// CLASSIFIES, not "0==0"):
//   - sourceUnitsTotal / sourceUnitsParsed (parsed == total source-only)
//   - routinesTotal / routinesBodyAvailable
//   - routinesParseIncomplete (NONZERO — the corpus has a parse-incomplete fixture)
//   - opaqueApps (ZERO source-only — asserted ==0, NOT fail-on-zero)
//   - unresolvedCallsites (NONZERO multiset cardinality)
//   - dynamicDispatchSites (NONZERO multiset cardinality)
// An oracle cross-check asserts the Rust totals equal BOTH the per-golden
// recomputation AND the al-sem manifest `coverageMatrix`.

/// Forbidden later-gate / L4 keys that must NEVER appear in the L3 coverage
/// comparison surface (golden OR rust). Coverage is R2d's surface; the call graph
/// (R2b), event graph (R2c), and summaries/typedEdges/analysisGaps are SEPARATE
/// gates. Mirrors the manifest `forbiddenKeys`.
const L3COV_FORBIDDEN_KEYS: &[&str] = &[
    // call-graph surface (R2b — a separate pass)
    "callGraph",
    "callsiteId",
    "dispatchKind",
    "dispatchMeta",
    "argumentBindings",
    "groups",
    "bindings",
    "callsiteResolutions",
    // event-graph surface (R2c — a separate pass)
    "eventGraph",
    "events",
    "edges",
    "eventKind",
    // later-gate / L4 / R2.5
    "typedEdges",
    "summary",
    "analysisGaps",
    "capabilityFactsDirect",
    "rootClassifications",
];

const L3COV_TEST_NAME: &str = "differential_l3_coverage_match_goldens";

/// The R2d coverage matrix axes (al-sem manifest `coverageMatrix`). Driven by Rust;
/// oracle-cross-checked against the al-sem manifest's `coverageMatrix`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct CoverageMatrix2 {
    source_units_total: usize,
    source_units_parsed: usize,
    routines_total: usize,
    routines_body_available: usize,
    routines_parse_incomplete: usize,
    opaque_apps: usize,
    unresolved_callsites: usize,
    dynamic_dispatch_sites: usize,
}

impl CoverageMatrix2 {
    fn add(&mut self, o: &CoverageMatrix2) {
        self.source_units_total += o.source_units_total;
        self.source_units_parsed += o.source_units_parsed;
        self.routines_total += o.routines_total;
        self.routines_body_available += o.routines_body_available;
        self.routines_parse_incomplete += o.routines_parse_incomplete;
        self.opaque_apps += o.opaque_apps;
        self.unresolved_callsites += o.unresolved_callsites;
        self.dynamic_dispatch_sites += o.dynamic_dispatch_sites;
    }
}

/// Count the matrix axes from ONE coverage projection `Value` (golden OR rust —
/// same shape). The counts are array LENGTHS (multisets) / scalars, so a duplicate
/// in a multiset is counted once per occurrence (cardinality), matching al-sem.
fn coverage_matrix_of(proj: &serde_json::Value) -> CoverageMatrix2 {
    let num = |k: &str| -> usize { proj.get(k).and_then(|v| v.as_u64()).unwrap_or(0) as usize };
    let len = |k: &str| -> usize {
        proj.get(k)
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0)
    };
    CoverageMatrix2 {
        source_units_total: num("sourceUnitsTotal"),
        source_units_parsed: num("sourceUnitsParsed"),
        routines_total: num("routinesTotal"),
        routines_body_available: num("routinesBodyAvailable"),
        routines_parse_incomplete: len("routinesParseIncomplete"),
        opaque_apps: len("opaqueApps"),
        unresolved_callsites: len("unresolvedCallsites"),
        dynamic_dispatch_sites: len("dynamicDispatchSites"),
    }
}

/// Discover every `tests/r2d-goldens/*.l3cov.golden.json`, sorted by fixture name.
fn discover_l3cov_goldens() -> Vec<(String, PathBuf)> {
    let dir = repo_root().join("tests").join("r2d-goldens");
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("failed to read L3cov goldens dir {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".l3cov.golden.json") {
            continue; // skips manifest.json + l3cov-vectors.json
        }
        let fixture = name.trim_end_matches(".l3cov.golden.json").to_string();
        out.push((fixture, entry.path()));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Rust-side L3 coverage projection for a fixture dir (fail-closed → all-empty).
fn rust_coverage_projection(fixture_dir: &Path) -> AnalysisCoverage {
    match assemble_and_resolve_workspace_default(fixture_dir) {
        Some(resolved) => resolved.project_coverage_disk(fixture_dir),
        None => AnalysisCoverage {
            source_units_total: 0,
            source_units_parsed: 0,
            routines_total: 0,
            routines_body_available: 0,
            routines_parse_incomplete: vec![],
            opaque_apps: vec![],
            unresolved_callsites: vec![],
            dynamic_dispatch_sites: vec![],
        },
    }
}

/// Recursively collect every forbidden later-gate object-key in `value` (L3cov set).
fn scan_l3cov_forbidden(value: &serde_json::Value, path: &str, hits: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let child = format!("{path}.{k}");
                if L3COV_FORBIDDEN_KEYS.contains(&k.as_str()) {
                    hits.push(child.clone());
                }
                scan_l3cov_forbidden(v, &child, hits);
            }
        }
        serde_json::Value::Array(arr) => {
            for (i, v) in arr.iter().enumerate() {
                scan_l3cov_forbidden(v, &format!("{path}[{i}]"), hits);
            }
        }
        _ => {}
    }
}

/// The R2d L3 coverage differential pass + the anti-degenerate coverage matrix.
/// Gated by `R2D_L3COV_SET` (committed default `full`: all 158 goldens — the R2d /
/// R2 source-only L3 EXIT GATE; `small` = ws-d2 + ws-unresolved +
/// ws-policy-api-dynamic-dispatch for dev iteration).
#[test]
fn differential_l3_coverage_match_goldens() {
    let all_goldens = discover_l3cov_goldens();
    assert!(
        !all_goldens.is_empty(),
        "no L3cov goldens discovered under tests/r2d-goldens — corpus missing?"
    );

    let set = std::env::var("R2D_L3COV_SET").unwrap_or_else(|_| "full".to_string());
    let small_set = ["ws-d2", "ws-unresolved", "ws-policy-api-dynamic-dispatch"];
    let goldens: Vec<(String, PathBuf)> = match set.as_str() {
        "full" | "" => all_goldens,
        "small" => all_goldens
            .into_iter()
            .filter(|(f, _)| small_set.contains(&f.as_str()))
            .collect(),
        other => panic!("R2D_L3COV_SET={other:?} not recognized (expected `small` or `full`)"),
    };
    assert!(
        !goldens.is_empty(),
        "R2D_L3COV_SET={set:?} selected zero fixtures (small set = {small_set:?})"
    );

    let allowlist: Vec<AllowEntry> = load_allowlist()
        .into_iter()
        .filter(|e| e.test == L3COV_TEST_NAME)
        .collect();

    let mut all_divergences: Vec<Divergence> = Vec::new();
    let mut forbidden_hits: Vec<String> = Vec::new();
    let mut rust_cov = CoverageMatrix2::default();
    let mut golden_cov = CoverageMatrix2::default();

    for (fixture, golden_path) in &goldens {
        let fixture_dir = corpus_dir().join(fixture);
        assert!(
            fixture_dir.is_dir(),
            "L3cov golden {} has no matching in-repo fixture at {} (offline corpus incomplete)",
            golden_path.display(),
            fixture_dir.display()
        );

        // Golden side: parse as JSON (for the diff) AND validate it parses as the
        // allowlisted AnalysisCoverage serde type (shape guard).
        let golden_text = std::fs::read_to_string(golden_path)
            .unwrap_or_else(|e| panic!("read L3cov golden {}: {e}", golden_path.display()));
        let golden_json: serde_json::Value =
            serde_json::from_str(&golden_text).unwrap_or_else(|e| {
                panic!(
                    "L3cov golden {} is not valid JSON: {e}",
                    golden_path.display()
                )
            });
        let _: AnalysisCoverage = serde_json::from_value(golden_json.clone()).unwrap_or_else(|e| {
            panic!(
                "L3cov golden {} does not parse as AnalysisCoverage: {e}",
                golden_path.display()
            )
        });

        // Rust side: disk-backed assemble+resolve → project_coverage_disk → JSON.
        let projection = rust_coverage_projection(&fixture_dir);
        let rust_json = serde_json::to_value(&projection)
            .unwrap_or_else(|e| panic!("serialize Rust L3cov projection for {fixture}: {e}"));

        // Forbidden later-gate / L4 field scan on BOTH sides (hard fail).
        scan_l3cov_forbidden(
            &golden_json,
            &format!("{fixture}:golden"),
            &mut forbidden_hits,
        );
        scan_l3cov_forbidden(&rust_json, &format!("{fixture}:rust"), &mut forbidden_hits);

        // Coverage (Rust drives the anti-degenerate gate; golden is the oracle).
        rust_cov.add(&coverage_matrix_of(&rust_json));
        golden_cov.add(&coverage_matrix_of(&golden_json));

        // Structural compare of the whole flat projection (multisets positional
        // after the projection's sort — preserves + checks duplicates).
        diff_l2_value(
            fixture,
            "coverage",
            &golden_json,
            &rust_json,
            &mut all_divergences,
        );
    }

    all_divergences
        .sort_by(|a, b| (a.fixture.as_str(), &a.path).cmp(&(b.fixture.as_str(), &b.path)));

    // --- Forbidden-field guard (hard fail, never allowlistable) -------------
    assert!(
        forbidden_hits.is_empty(),
        "FORBIDDEN later-gate/L4 field(s) leaked into the L3 coverage comparison \
         (golden or rust):\n  {}",
        forbidden_hits.join("\n  ")
    );

    // --- COVERAGE MATRIX gate (anti-degenerate) -----------------------------
    eprintln!(
        "R2d L3cov coverage matrix (set={set:?}, {} fixture(s)):\n  \
         sourceUnitsTotal={} sourceUnitsParsed={} routinesTotal={} routinesBodyAvailable={} \
         routinesParseIncomplete={} opaqueApps={} unresolvedCallsites={} dynamicDispatchSites={}",
        goldens.len(),
        rust_cov.source_units_total,
        rust_cov.source_units_parsed,
        rust_cov.routines_total,
        rust_cov.routines_body_available,
        rust_cov.routines_parse_incomplete,
        rust_cov.opaque_apps,
        rust_cov.unresolved_callsites,
        rust_cov.dynamic_dispatch_sites,
    );
    // Fail-on-zero per axis ONLY for the full corpus. opaqueApps is structurally
    // EMPTY source-only (asserted ==0, NOT fail-on-zero); sourceUnitsParsed must
    // equal sourceUnitsTotal (the decrement is corpus-inert — covered by a vector).
    if set == "full" || set.is_empty() {
        let nonzero_axes: [(&str, usize); 6] = [
            ("sourceUnitsTotal", rust_cov.source_units_total),
            ("routinesTotal", rust_cov.routines_total),
            ("routinesBodyAvailable", rust_cov.routines_body_available),
            (
                "routinesParseIncomplete",
                rust_cov.routines_parse_incomplete,
            ),
            ("unresolvedCallsites", rust_cov.unresolved_callsites),
            ("dynamicDispatchSites", rust_cov.dynamic_dispatch_sites),
        ];
        let zero_axes: Vec<&str> = nonzero_axes
            .iter()
            .filter(|(_, n)| *n == 0)
            .map(|(name, _)| *name)
            .collect();
        assert!(
            zero_axes.is_empty(),
            "DEGENERATE L3cov coverage matrix (set={set:?}): axis/axes {zero_axes:?} are ZERO — \
             the R2d port is not actually CLASSIFYING that case (empty==empty would pass a pure \
             equality diff). The matrix must prove coverage accounting fires.",
        );
        assert_eq!(
            rust_cov.opaque_apps, 0,
            "opaqueApps MUST be ZERO source-only (becomes non-empty only in R2.5)",
        );
        assert_eq!(
            rust_cov.source_units_parsed, rust_cov.source_units_total,
            "sourceUnitsParsed MUST equal sourceUnitsTotal source-only (decrement is corpus-inert)",
        );
    }

    // Oracle cross-check: Rust coverage MUST equal the golden coverage (al-sem
    // ground truth) for the SAME fixture set.
    assert_eq!(
        rust_cov, golden_cov,
        "L3cov coverage matrix MISMATCH vs golden oracle (set={set:?})\n  rust   = {rust_cov:?}\n  golden = {golden_cov:?}",
    );

    // --- Allowlist gating (same semantics as R0/L2/R2a/R2b/R2c) -------------
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
            "\n{} UNDOCUMENTED L3cov divergence(s) (not in KNOWN_DIVERGENCES.json, \
             test={L3COV_TEST_NAME}):\n",
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
            "\n{} UNUSED L3cov allowlist entr(y/ies) (no matching divergence this run):\n",
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
        "R2d L3 coverage differential FAILED (set={set:?}):{failure}"
    );

    eprintln!(
        "R2d L3cov differential: set={set:?}, {} fixture(s), 0 divergences, \
         allowlist fully consumed ({} L3cov entr(y/ies)).",
        goldens.len(),
        allowlist.len()
    );
}

/// Oracle cross-check: the FULL-corpus Rust coverage matrix must equal the al-sem
/// manifest's published `coverageMatrix` (the ground-truth totals). Independent of
/// the per-fixture golden compare — guards the matrix counters against drift. The
/// manifest carries extra axes (sourceUnitsDecremented / unresolvedMaxDup /
/// dynamicMaxDup) that this oracle ALSO checks.
#[test]
fn l3cov_coverage_matrix_matches_manifest_oracle() {
    let goldens = discover_l3cov_goldens();
    assert!(!goldens.is_empty(), "no L3cov goldens — corpus missing?");

    let mut rust_cov = CoverageMatrix2::default();
    let mut unresolved_max_dup = 0usize;
    let mut dynamic_max_dup = 0usize;
    for (fixture, _) in &goldens {
        let proj = rust_coverage_projection(&corpus_dir().join(fixture));
        let rust_json = serde_json::to_value(&proj).expect("serialize");
        rust_cov.add(&coverage_matrix_of(&rust_json));
        unresolved_max_dup = unresolved_max_dup.max(max_dup(&proj.unresolved_callsites));
        dynamic_max_dup = dynamic_max_dup.max(max_dup(&proj.dynamic_dispatch_sites));
    }

    let manifest_path = repo_root()
        .join("tests")
        .join("r2d-goldens")
        .join("manifest.json");
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).expect("read r2d manifest"))
            .expect("parse r2d manifest");
    let m = manifest
        .get("coverageMatrix")
        .expect("manifest has coverageMatrix");
    let mget =
        |k: &str| -> usize { m.get(k).and_then(|v| v.as_u64()).unwrap_or(u64::MAX) as usize };
    let manifest_cov = CoverageMatrix2 {
        source_units_total: mget("sourceUnitsTotal"),
        source_units_parsed: mget("sourceUnitsParsed"),
        routines_total: mget("routinesTotal"),
        routines_body_available: mget("routinesBodyAvailable"),
        routines_parse_incomplete: mget("routinesParseIncomplete"),
        opaque_apps: mget("opaqueApps"),
        unresolved_callsites: mget("unresolvedCallsites"),
        dynamic_dispatch_sites: mget("dynamicDispatchSites"),
    };
    assert_eq!(
        rust_cov, manifest_cov,
        "R2d coverage matrix MISMATCH vs al-sem manifest oracle\n  rust     = {rust_cov:?}\n  manifest = {manifest_cov:?}",
    );
    // The manifest's max-dup axes: source-only the corpus has NO real duplicate
    // (interface multi-edges are `maybe`, excluded), so both are 1 (or 0 if an axis
    // is empty — but unresolved/dynamic are nonzero). al-sem reports max-dup as the
    // max occurrences of any single id; with no real dup that is 1.
    assert_eq!(
        unresolved_max_dup,
        mget("unresolvedMaxDup"),
        "unresolvedMaxDup mismatch (rust={unresolved_max_dup})",
    );
    assert_eq!(
        dynamic_max_dup,
        mget("dynamicMaxDup"),
        "dynamicMaxDup mismatch (rust={dynamic_max_dup})",
    );
    assert_eq!(
        mget("sourceUnitsDecremented"),
        rust_cov.source_units_total - rust_cov.source_units_parsed,
        "sourceUnitsDecremented mismatch (source-only == 0)",
    );
    eprintln!(
        "R2d coverage matrix oracle: Rust full-corpus totals == al-sem manifest coverageMatrix \
         (incl. max-dup + decremented axes)."
    );
}

/// Max occurrences of any single id in a multiset (the manifest `*MaxDup` axis).
fn max_dup(ids: &[String]) -> usize {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for id in ids {
        *counts.entry(id.as_str()).or_insert(0) += 1;
    }
    counts.values().copied().max().unwrap_or(0)
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

    // (b3) Regenerate + copy the R2a L3 record-type goldens. A third dump script
    //      (`scripts/dump-l3-record-types.ts`) writes `scripts/r2a-goldens/
    //      *.l3rt.golden.json` + `manifest.json`; copy them into
    //      `tests/r2a-goldens/`. Source fixtures are the SAME `ws-*` trees already
    //      copied to `tests/r0-corpus/` above (no separate corpus). NOTE the new
    //      `ws-r2a-record-types` fixture only appears here if al-sem's r2a goldens
    //      include it (its golden's source fixture is copied via the R0 loop above
    //      when an R0 golden exists; if not, copy it explicitly below).
    eprintln!("refresh: running `bun run scripts/dump-l3-record-types.ts` in {al_sem_dir} ...");
    let l3_status = std::process::Command::new("bun")
        .args(["run", "scripts/dump-l3-record-types.ts"])
        .current_dir(&al_sem)
        // dump-l3-record-types writes its manifest JSON to stdout; discard it
        // (files are the artifact). Logs go to the inherited stderr.
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn `bun` for L3 dump: {e}"));
    assert!(
        l3_status.success(),
        "`bun run scripts/dump-l3-record-types.ts` failed with status {l3_status}"
    );

    let src_l3_goldens = al_sem.join("scripts").join("r2a-goldens");
    let dst_l3_goldens = repo_root().join("tests").join("r2a-goldens");
    std::fs::create_dir_all(&dst_l3_goldens).expect("create tests/r2a-goldens");
    let mut l3_copied = 0usize;
    for entry in std::fs::read_dir(&src_l3_goldens).expect("read al-sem r2a-goldens") {
        let entry = entry.expect("entry");
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".l3rt.golden.json") {
            continue; // skips manifest.json + l3rt-vectors.json (vectors are separate).
        }
        // Ensure the source fixture is present in the offline corpus (the R0 loop
        // above only copies fixtures with an R0 golden; a record-types-only fixture
        // like ws-r2a-record-types would otherwise be missing).
        let fixture = name.trim_end_matches(".l3rt.golden.json").to_string();
        let fixture_dst = dst_corpus.join(&fixture);
        if !fixture_dst.is_dir() {
            let fixture_src = src_fixtures.join(&fixture);
            if fixture_src.is_dir() {
                copy_source_fixture(&fixture_src, &fixture_dst);
                eprintln!(
                    "refresh: copied missing source fixture {fixture} into tests/r0-corpus/."
                );
            }
        }
        std::fs::copy(entry.path(), dst_l3_goldens.join(&name))
            .unwrap_or_else(|e| panic!("copy L3 golden {name}: {e}"));
        l3_copied += 1;
    }
    let l3_manifest_src = src_l3_goldens.join("manifest.json");
    if l3_manifest_src.is_file() {
        std::fs::copy(&l3_manifest_src, dst_l3_goldens.join("manifest.json"))
            .expect("copy r2a-goldens/manifest.json");
    }
    eprintln!("refresh: copied {l3_copied} L3 golden(s) into tests/r2a-goldens/.");

    // (b4) Regenerate + copy the R2b L3 CALL-GRAPH goldens. A fourth dump script
    //      (`scripts/dump-l3-call-graph.ts`) writes `scripts/r2b-goldens/
    //      *.l3cg.golden.json` + `manifest.json`; copy them into
    //      `tests/r2b-goldens/`. Source fixtures are the SAME `ws-*` trees already
    //      copied to `tests/r0-corpus/` above; any call-graph-only fixture
    //      (ws-r2b-opaque / ws-r2b-external / ws-interface-dispatch) is copied
    //      explicitly here when missing.
    eprintln!("refresh: running `bun run scripts/dump-l3-call-graph.ts` in {al_sem_dir} ...");
    let l3cg_status = std::process::Command::new("bun")
        .args(["run", "scripts/dump-l3-call-graph.ts"])
        .current_dir(&al_sem)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn `bun` for L3 call-graph dump: {e}"));
    assert!(
        l3cg_status.success(),
        "`bun run scripts/dump-l3-call-graph.ts` failed with status {l3cg_status}"
    );

    let src_l3cg_goldens = al_sem.join("scripts").join("r2b-goldens");
    let dst_l3cg_goldens = repo_root().join("tests").join("r2b-goldens");
    std::fs::create_dir_all(&dst_l3cg_goldens).expect("create tests/r2b-goldens");
    let mut l3cg_copied = 0usize;
    for entry in std::fs::read_dir(&src_l3cg_goldens).expect("read al-sem r2b-goldens") {
        let entry = entry.expect("entry");
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".l3cg.golden.json") {
            continue; // skips manifest.json + l3cg-vectors.json (vectors are separate).
        }
        // Ensure the source fixture is present in the offline corpus.
        let fixture = name.trim_end_matches(".l3cg.golden.json").to_string();
        let fixture_dst = dst_corpus.join(&fixture);
        if !fixture_dst.is_dir() {
            let fixture_src = src_fixtures.join(&fixture);
            if fixture_src.is_dir() {
                copy_source_fixture(&fixture_src, &fixture_dst);
                eprintln!(
                    "refresh: copied missing source fixture {fixture} into tests/r0-corpus/."
                );
            }
        }
        std::fs::copy(entry.path(), dst_l3cg_goldens.join(&name))
            .unwrap_or_else(|e| panic!("copy L3cg golden {name}: {e}"));
        l3cg_copied += 1;
    }
    let l3cg_manifest_src = src_l3cg_goldens.join("manifest.json");
    if l3cg_manifest_src.is_file() {
        std::fs::copy(&l3cg_manifest_src, dst_l3cg_goldens.join("manifest.json"))
            .expect("copy r2b-goldens/manifest.json");
    }
    eprintln!("refresh: copied {l3cg_copied} L3 call-graph golden(s) into tests/r2b-goldens/.");

    // (b5) Regenerate + copy the R2c L3 EVENT-GRAPH goldens. A fifth dump script
    //      (`scripts/dump-l3-event-graph.ts`) writes `scripts/r2c-goldens/
    //      *.l3eg.golden.json` + `manifest.json`; copy them into
    //      `tests/r2c-goldens/`. Only the 31 event-BEARING fixtures get a golden
    //      (al-sem excludes event-less fixtures); their source `ws-*` trees are the
    //      SAME ones copied to `tests/r0-corpus/` above, with the event-only
    //      fixtures (ws-r2c-maybe-elem / ws-r2c-mixed / ws-r2c-two-sub-maybe) copied
    //      explicitly here when missing.
    eprintln!("refresh: running `bun run scripts/dump-l3-event-graph.ts` in {al_sem_dir} ...");
    let l3eg_status = std::process::Command::new("bun")
        .args(["run", "scripts/dump-l3-event-graph.ts"])
        .current_dir(&al_sem)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn `bun` for L3 event-graph dump: {e}"));
    assert!(
        l3eg_status.success(),
        "`bun run scripts/dump-l3-event-graph.ts` failed with status {l3eg_status}"
    );

    let src_l3eg_goldens = al_sem.join("scripts").join("r2c-goldens");
    let dst_l3eg_goldens = repo_root().join("tests").join("r2c-goldens");
    std::fs::create_dir_all(&dst_l3eg_goldens).expect("create tests/r2c-goldens");
    let mut l3eg_copied = 0usize;
    for entry in std::fs::read_dir(&src_l3eg_goldens).expect("read al-sem r2c-goldens") {
        let entry = entry.expect("entry");
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".l3eg.golden.json") {
            continue; // skips manifest.json + l3eg-vectors.json (vectors are separate).
        }
        // Ensure the source fixture is present in the offline corpus.
        let fixture = name.trim_end_matches(".l3eg.golden.json").to_string();
        let fixture_dst = dst_corpus.join(&fixture);
        if !fixture_dst.is_dir() {
            let fixture_src = src_fixtures.join(&fixture);
            if fixture_src.is_dir() {
                copy_source_fixture(&fixture_src, &fixture_dst);
                eprintln!(
                    "refresh: copied missing source fixture {fixture} into tests/r0-corpus/."
                );
            }
        }
        std::fs::copy(entry.path(), dst_l3eg_goldens.join(&name))
            .unwrap_or_else(|e| panic!("copy L3eg golden {name}: {e}"));
        l3eg_copied += 1;
    }
    let l3eg_manifest_src = src_l3eg_goldens.join("manifest.json");
    if l3eg_manifest_src.is_file() {
        std::fs::copy(&l3eg_manifest_src, dst_l3eg_goldens.join("manifest.json"))
            .expect("copy r2c-goldens/manifest.json");
    }
    eprintln!("refresh: copied {l3eg_copied} L3 event-graph golden(s) into tests/r2c-goldens/.");

    // (b6) Regenerate + copy the R2d L3 COVERAGE goldens. A sixth dump script
    //      (`scripts/dump-l3-coverage.ts`) writes `scripts/r2d-goldens/
    //      *.l3cov.golden.json` + `manifest.json`; copy them into
    //      `tests/r2d-goldens/`. EVERY source workspace has a coverage golden (158
    //      total — no inclusion rule); their source `ws-*` trees are the SAME ones
    //      copied to `tests/r0-corpus/` above (copied explicitly here when missing).
    eprintln!("refresh: running `bun run scripts/dump-l3-coverage.ts` in {al_sem_dir} ...");
    let l3cov_status = std::process::Command::new("bun")
        .args(["run", "scripts/dump-l3-coverage.ts"])
        .current_dir(&al_sem)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn `bun` for L3 coverage dump: {e}"));
    assert!(
        l3cov_status.success(),
        "`bun run scripts/dump-l3-coverage.ts` failed with status {l3cov_status}"
    );

    let src_l3cov_goldens = al_sem.join("scripts").join("r2d-goldens");
    let dst_l3cov_goldens = repo_root().join("tests").join("r2d-goldens");
    std::fs::create_dir_all(&dst_l3cov_goldens).expect("create tests/r2d-goldens");
    let mut l3cov_copied = 0usize;
    for entry in std::fs::read_dir(&src_l3cov_goldens).expect("read al-sem r2d-goldens") {
        let entry = entry.expect("entry");
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".l3cov.golden.json") {
            continue; // skips manifest.json + l3cov-vectors.json (vectors are separate).
        }
        // Ensure the source fixture is present in the offline corpus.
        let fixture = name.trim_end_matches(".l3cov.golden.json").to_string();
        let fixture_dst = dst_corpus.join(&fixture);
        if !fixture_dst.is_dir() {
            let fixture_src = src_fixtures.join(&fixture);
            if fixture_src.is_dir() {
                copy_source_fixture(&fixture_src, &fixture_dst);
                eprintln!(
                    "refresh: copied missing source fixture {fixture} into tests/r0-corpus/."
                );
            }
        }
        std::fs::copy(entry.path(), dst_l3cov_goldens.join(&name))
            .unwrap_or_else(|e| panic!("copy L3cov golden {name}: {e}"));
        l3cov_copied += 1;
    }
    // Also copy the vectors file so the offline l3cov_vectors test stays in sync.
    let l3cov_vectors_src = src_l3cov_goldens.join("l3cov-vectors.json");
    if l3cov_vectors_src.is_file() {
        let dst_vectors = repo_root().join("tests").join("r2d-vectors");
        std::fs::create_dir_all(&dst_vectors).expect("create tests/r2d-vectors");
        std::fs::copy(&l3cov_vectors_src, dst_vectors.join("l3cov-vectors.json"))
            .expect("copy l3cov-vectors.json");
    }
    let l3cov_manifest_src = src_l3cov_goldens.join("manifest.json");
    if l3cov_manifest_src.is_file() {
        std::fs::copy(&l3cov_manifest_src, dst_l3cov_goldens.join("manifest.json"))
            .expect("copy r2d-goldens/manifest.json");
    }
    eprintln!("refresh: copied {l3cov_copied} L3 coverage golden(s) into tests/r2d-goldens/.");

    // (b7) Regenerate + copy the R3a-1 COMBINED-GRAPH + SCC goldens. A seventh dump
    //      script (`scripts/dump-r3a1-combined-graph.ts`) writes
    //      `scripts/r3a1-goldens/*.r3a1.golden.json` + `manifest.json`; copy them
    //      into `tests/r3a1-goldens/`. The 158 source-only `ws-*` fixtures are the
    //      SAME trees already copied to `tests/r0-corpus/` above (copied explicitly
    //      here when missing). al-sem EXCLUDES `.app`-bearing + empty fail-closed
    //      fixtures, so the golden set is exactly the source-only-analyzable corpus.
    eprintln!("refresh: running `bun run scripts/dump-r3a1-combined-graph.ts` in {al_sem_dir} ...");
    let r3a1_status = std::process::Command::new("bun")
        .args(["run", "scripts/dump-r3a1-combined-graph.ts"])
        .current_dir(&al_sem)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn `bun` for R3a-1 combined-graph dump: {e}"));
    assert!(
        r3a1_status.success(),
        "`bun run scripts/dump-r3a1-combined-graph.ts` failed with status {r3a1_status}"
    );

    let src_r3a1_goldens = al_sem.join("scripts").join("r3a1-goldens");
    let dst_r3a1_goldens = repo_root().join("tests").join("r3a1-goldens");
    std::fs::create_dir_all(&dst_r3a1_goldens).expect("create tests/r3a1-goldens");
    let mut r3a1_copied = 0usize;
    for entry in std::fs::read_dir(&src_r3a1_goldens).expect("read al-sem r3a1-goldens") {
        let entry = entry.expect("entry");
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".r3a1.golden.json") {
            continue; // skips manifest.json + r3a1-vectors.json (vectors are separate).
        }
        // Ensure the source fixture is present in the offline corpus.
        let fixture = name.trim_end_matches(".r3a1.golden.json").to_string();
        let fixture_dst = dst_corpus.join(&fixture);
        if !fixture_dst.is_dir() {
            let fixture_src = src_fixtures.join(&fixture);
            if fixture_src.is_dir() {
                copy_source_fixture(&fixture_src, &fixture_dst);
                eprintln!(
                    "refresh: copied missing source fixture {fixture} into tests/r0-corpus/."
                );
            }
        }
        std::fs::copy(entry.path(), dst_r3a1_goldens.join(&name))
            .unwrap_or_else(|e| panic!("copy R3a-1 golden {name}: {e}"));
        r3a1_copied += 1;
    }
    let r3a1_manifest_src = src_r3a1_goldens.join("manifest.json");
    if r3a1_manifest_src.is_file() {
        std::fs::copy(&r3a1_manifest_src, dst_r3a1_goldens.join("manifest.json"))
            .expect("copy r3a1-goldens/manifest.json");
    }
    eprintln!(
        "refresh: copied {r3a1_copied} R3a-1 combined-graph golden(s) into tests/r3a1-goldens/."
    );

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
