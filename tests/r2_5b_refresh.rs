//! R2.5b UNIFIED GOLDEN REFRESH — the one-command regen path for ALL FOUR
//! cross-app L3 sub-gates (record-types / call graph / event graph / coverage).
//!
//! Mirrors the R2.5a `refresh_r2_5a_goldens_from_al_sem` pattern
//! (`tests/r2_5a_differential.rs`): `#[ignore]`d so the default `cargo test` loop
//! stays fully OFFLINE, gated on `AL_SEM_DIR`. After regenerating the al-sem
//! goldens (`bun run scripts/r2.5b-cross-app-capture.ts` + the four
//! `scripts/dump-r2.5b-*.ts` dumps), run:
//!
//! ```bash
//! AL_SEM_DIR=/u/Git/al-sem cargo test --test r2_5b_refresh -- \
//!     --ignored refresh_r2_5b_goldens_from_al_sem --nocapture
//! ```
//!
//! It re-copies, from the al-sem checkout into the engine:
//!   - ALL FOUR golden sets — `scripts/r2.5b-{rt,cg,eg,cov}-goldens/*.r2.5b-*.golden.json`
//!     + each `manifest.json` → `tests/r2-5b-{rt,cg,eg,cov}-goldens/`;
//!   - the committed dep `.app` fixtures — `test/fixtures/r2.5b-deps/<guid>.app`
//!     → each fixture's `tests/r2-5b-fixtures/<fixture>/.alpackages/<guid>.app`
//!     (keyed via the manifest's top-level `depAppGuids` + `fixtures[].fixture`),
//!     so BOTH sides read the SAME `.app` bytes.
//!
//! NOTE on the workspace `.al` files: al-sem generates the cross-app workspace
//! INLINE (TS string constants in `scripts/r2.5b-cross-app-capture.ts`, written
//! into an mkdtemp) — there is NO committed `.al` workspace dir to copy. The
//! engine's `tests/r2-5b-fixtures/<fixture>/{app.json,src/*.al}` are the
//! hand-maintained mirror of those constants; if the al-sem capture changes the
//! workspace shape, update the engine `.al` files to match (this refresh copies
//! the goldens + `.app`s; it cannot copy a workspace al-sem never commits). The
//! `.app` deps ARE committed and are copied here, so the dep-side bytes stay in
//! lockstep automatically.
//!
//! Like the R0/R2.5a refreshes: this does NOT auto-commit — it leaves a reviewable
//! diff. If `AL_SEM_DIR` is unset it skips (a stray `--ignored` run is a no-op).

use std::path::PathBuf;

use serde_json::Value;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixtures_dir() -> PathBuf {
    repo_root().join("tests").join("r2-5b-fixtures")
}

/// The four sub-gates: (al-sem source golden dir, engine dest golden dir, golden
/// filename suffix). Each al-sem dir lives under `scripts/`, the engine dir under
/// `tests/`.
const SUBGATES: &[(&str, &str, &str)] = &[
    (
        "r2.5b-rt-goldens",
        "r2-5b-rt-goldens",
        ".r2.5b-rt.golden.json",
    ),
    (
        "r2.5b-cg-goldens",
        "r2-5b-cg-goldens",
        ".r2.5b-cg.golden.json",
    ),
    (
        "r2.5b-eg-goldens",
        "r2-5b-eg-goldens",
        ".r2.5b-eg.golden.json",
    ),
    (
        "r2.5b-cov-goldens",
        "r2-5b-cov-goldens",
        ".r2.5b-cov.golden.json",
    ),
];

/// LIVE refresh: re-copy ALL FOUR R2.5b golden sets + the dep `.app` fixtures from
/// an al-sem checkout (`AL_SEM_DIR`). `#[ignore]`d — never runs in the offline loop.
#[test]
#[ignore]
fn refresh_r2_5b_goldens_from_al_sem() {
    let al_sem = match std::env::var("AL_SEM_DIR") {
        Ok(d) => PathBuf::from(d),
        Err(_) => {
            eprintln!("AL_SEM_DIR not set — skipping R2.5b refresh");
            return;
        }
    };

    let src_apps = al_sem.join("test").join("fixtures").join("r2.5b-deps");

    // The fixture → depAppGuids mapping is the SAME across all four manifests; read
    // it once from the rt manifest (after copying the goldens below).
    for (src_name, dst_name, suffix) in SUBGATES {
        let src_goldens = al_sem.join("scripts").join(src_name);
        let dst_goldens = repo_root().join("tests").join(dst_name);
        std::fs::create_dir_all(&dst_goldens).expect("mk goldens dir");
        for entry in std::fs::read_dir(&src_goldens)
            .unwrap_or_else(|e| panic!("read al-sem goldens {}: {e}", src_goldens.display()))
            .flatten()
        {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(suffix) || name == "manifest.json" {
                std::fs::copy(entry.path(), dst_goldens.join(&name))
                    .unwrap_or_else(|e| panic!("copy golden {name}: {e}"));
            }
        }
        eprintln!("R2.5b: copied {src_name} → tests/{dst_name}");
    }

    // Copy each committed dep `.app` into every fixture's `.alpackages/` (both sides
    // read the SAME bytes). The fixture → depAppGuids mapping comes from the rt
    // manifest just copied in.
    let manifest_text = std::fs::read_to_string(
        repo_root()
            .join("tests")
            .join("r2-5b-rt-goldens")
            .join("manifest.json"),
    )
    .expect("read r2.5b-rt manifest");
    let manifest: Value = serde_json::from_str(&manifest_text).expect("manifest parses");

    // Prefer per-fixture depAppGuids if present; else the manifest-level list.
    let top_level_guids: Vec<String> = manifest["depAppGuids"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|g| g.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    if let Some(fixtures) = manifest["fixtures"].as_array() {
        for f in fixtures {
            let fixture = f["fixture"].as_str().expect("fixture name");
            let alpackages = fixtures_dir().join(fixture).join(".alpackages");
            std::fs::create_dir_all(&alpackages).expect("mk .alpackages dir");
            let guids: Vec<String> = f["depAppGuids"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|g| g.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .filter(|v: &Vec<String>| !v.is_empty())
                .unwrap_or_else(|| top_level_guids.clone());
            for guid in &guids {
                let app = format!("{guid}.app");
                std::fs::copy(src_apps.join(&app), alpackages.join(&app))
                    .unwrap_or_else(|e| panic!("copy .app {app} for {fixture}: {e}"));
            }
            eprintln!(
                "R2.5b: copied {} dep .app(s) → tests/r2-5b-fixtures/{fixture}/.alpackages/",
                guids.len()
            );
        }
    }

    eprintln!(
        "R2.5b goldens (rt/cg/eg/cov) + dep .app fixtures refreshed from {}",
        al_sem.display()
    );
}
