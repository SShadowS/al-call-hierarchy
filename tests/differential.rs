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

use al_call_hierarchy::engine::snapshot::{
    snapshot_workspace, IdentitySnapshot, ObjectIdentity, RoutineIdentity,
};
use serde::Deserialize;

/// One entry in `KNOWN_DIVERGENCES.json`.
#[derive(Debug, Clone, Deserialize)]
struct AllowEntry {
    fixture: String,
    path: String,
    #[serde(default)]
    #[allow(dead_code)] // documentation fields; not used in matching.
    reason: String,
    #[serde(default)]
    #[allow(dead_code)]
    expires: String,
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

    let allowlist = load_allowlist();

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

/// Copy a source-only fixture (`app.json` + `src/**/*.al`) into `dst`, skipping
/// dependency/package dirs. Mirrors the offline-corpus contract.
fn copy_source_fixture(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap_or_else(|e| panic!("create {}: {e}", dst.display()));
    // app.json (verbatim).
    let app_json = src.join("app.json");
    if app_json.is_file() {
        std::fs::copy(&app_json, dst.join("app.json")).expect("copy app.json");
    }
    // Recurse only over `src/` plus any top-level *.al.
    copy_al_tree(src, dst);
}

/// Recursively copy `*.al` files (and the dirs containing them) from `src` to
/// `dst`, skipping `.alpackages` / `.git`.
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
            if name_lc == ".alpackages" || name_lc == ".git" {
                continue;
            }
            copy_al_tree(&path, &dst.join(&name));
        } else if ftype.is_file()
            && path
                .extension()
                .map(|e| e.eq_ignore_ascii_case("al"))
                .unwrap_or(false)
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
