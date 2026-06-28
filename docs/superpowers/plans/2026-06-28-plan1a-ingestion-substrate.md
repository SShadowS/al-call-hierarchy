# Plan 1A — Ingestion Substrate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the app-set snapshot ingestion layer — extract per-app source (workspace / embedded ShowMyCode `.app` / verified local repo / symbol-only) with identity verification and per-app compilation context, ready to feed the deep resolver.

**Architecture:** A new `src/snapshot/` module. An `AppSetSnapshot` holds `AppUnit`s; each `AppUnit` is produced by a `SourceProvider` (chosen by priority) and gated by an `IdentityVerifier` that fails closed to symbol-only on mismatch. Reuses the existing `app_package.rs` zip reader and `dependencies.rs` discovery; does NOT touch resolution/graph (that is Plan 1B).

**Tech Stack:** Rust, `zip` crate (already a dep, used by `app_package.rs`), `serde`/`serde_json`, `sha2` (blake3 is already a dep — use `blake3` for hashing), the `al-syntax` IR crate (`al_syntax::parse`).

## Global Constraints

- Rust edition 2024; toolchain pinned `rust-toolchain.toml` = 1.96.0. (verbatim from repo)
- Format per-file with `rustfmt <file>`, never `cargo fmt`. Stage only intended paths; never `git add -A`. (CLAUDE.md)
- CI gates on `cargo clippy --release -- -D warnings` and `cargo fmt --check` and `cargo test --workspace`. Every task must leave all three green.
- Update `CHANGELOG.md` under `Added` for the new module (once, in Task 1).
- Hashing: use `blake3` (already a dependency). Hash hex = `blake3::hash(bytes).to_hex().to_string()`.
- No `unwrap()`/`expect()` on fallible I/O in library code — return `anyhow::Result`. (matches `app_package.rs`)

---

### Task 1: `AppId` and provenance types

**Files:**
- Create: `src/snapshot/mod.rs`
- Create: `src/snapshot/identity.rs`
- Modify: `src/lib.rs` (add `pub mod snapshot;`) — confirm the crate root file; it is `src/lib.rs` if present, else `src/main.rs` declares modules. Check with `grep -n "pub mod" src/lib.rs src/main.rs`.
- Modify: `CHANGELOG.md`
- Test: in-file `#[cfg(test)]` in `src/snapshot/identity.rs`

**Interfaces:**
- Produces:
  - `AppId { guid: String, name: String, publisher: String, version: String }` (Clone, Debug, PartialEq, Eq, Hash)
  - `TrustTier` enum `{ Workspace, EmbeddedSource, LocalSourceVerified, LocalSourceApproximate, SymbolOnly }` (Copy, Clone, Debug, PartialEq, Eq)
  - `Provenance { app: AppId, tier: TrustTier, content_hash: String }` (Clone, Debug)

- [ ] **Step 1: Confirm module-root file**

Run: `grep -n "pub mod" src/lib.rs 2>/dev/null | head` and `grep -n "mod app_package\|mod dependencies" src/lib.rs src/main.rs`
Expected: identifies which file declares top-level modules (the one with `mod app_package;`). Use that file for the `pub mod snapshot;` line below.

- [ ] **Step 2: Write the failing test**

In `src/snapshot/identity.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_id_equality_is_field_wise() {
        let a = AppId {
            guid: "4b915d7e-c02a-435f-85ab-649086c1e002".into(),
            name: "Continia Core".into(),
            publisher: "Continia Software".into(),
            version: "29.0.0.0".into(),
        };
        let b = a.clone();
        assert_eq!(a, b);
        assert_eq!(a.short(), "Continia Software/Continia Core@29.0.0.0");
    }

    #[test]
    fn trust_tier_orders_workspace_strongest() {
        assert!(TrustTier::Workspace.rank() > TrustTier::SymbolOnly.rank());
        assert!(TrustTier::EmbeddedSource.rank() > TrustTier::LocalSourceApproximate.rank());
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p al-call-hierarchy snapshot::identity 2>&1 | tail -20`
Expected: FAIL — `cannot find type AppId`.

- [ ] **Step 4: Write minimal implementation**

In `src/snapshot/identity.rs` (above the test module):
```rust
//! Stable app identity + provenance/trust tiers for the app-set snapshot.

/// Identity of an AL app, matching `app.json` / SymbolReference fields.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AppId {
    pub guid: String,
    pub name: String,
    pub publisher: String,
    pub version: String,
}

impl AppId {
    /// Human-readable short form for logs/citations.
    pub fn short(&self) -> String {
        format!("{}/{}@{}", self.publisher, self.name, self.version)
    }
}

/// How trustworthy the source backing an app is.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TrustTier {
    Workspace,
    EmbeddedSource,
    LocalSourceVerified,
    LocalSourceApproximate,
    SymbolOnly,
}

impl TrustTier {
    /// Higher = stronger evidence. Used for provider selection + honest claims.
    pub fn rank(self) -> u8 {
        match self {
            TrustTier::Workspace => 5,
            TrustTier::EmbeddedSource => 4,
            TrustTier::LocalSourceVerified => 3,
            TrustTier::LocalSourceApproximate => 2,
            TrustTier::SymbolOnly => 1,
        }
    }
}

/// Provenance attached to every snapshot node/unit.
#[derive(Clone, Debug)]
pub struct Provenance {
    pub app: AppId,
    pub tier: TrustTier,
    pub content_hash: String,
}
```
In `src/snapshot/mod.rs`:
```rust
//! App-set snapshot ingestion substrate (Spec 1 / Plan 1A).
//!
//! Turns "workspace + symbol-only dep tables" into an explicit set of
//! identity-verified, per-app source roots ready for deep resolution.

pub mod identity;

pub use identity::{AppId, Provenance, TrustTier};
```
In the module-root file (from Step 1), add: `pub mod snapshot;`
In `CHANGELOG.md` under `## [Unreleased]` add an `### Added` bullet:
```markdown
- **App-set snapshot ingestion substrate** (`src/snapshot/`) — per-app source
  acquisition with identity verification + trust tiers (Spec 1 / Plan 1A).
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p al-call-hierarchy snapshot::identity 2>&1 | tail -10`
Expected: PASS (2 tests).

- [ ] **Step 6: Format, lint, commit**

```bash
rustfmt src/snapshot/mod.rs src/snapshot/identity.rs
cargo clippy --release -- -D warnings 2>&1 | tail -5
git add src/snapshot/mod.rs src/snapshot/identity.rs src/lib.rs CHANGELOG.md
git commit -m "feat(snapshot): AppId + TrustTier + Provenance types"
```
(Use the actual module-root file from Step 1 instead of `src/lib.rs` if different.)

---

### Task 2: Embedded `.app` source extraction

**Files:**
- Create: `src/snapshot/embedded.rs`
- Modify: `src/snapshot/mod.rs` (add `pub mod embedded;`)
- Test: in-file `#[cfg(test)]` in `src/snapshot/embedded.rs` (uses a real `.app` via env var, skips if absent)

**Interfaces:**
- Consumes: the `.app` zip layout (40-byte NAVX header + PK zip) — same as `app_package.rs` (`NAVX_HEADER_SIZE`).
- Produces:
  - `struct SourceFile { virtual_path: String, text: String }`
  - `fn extract_embedded_source(app_path: &std::path::Path) -> anyhow::Result<Vec<SourceFile>>` — returns every `*.al` entry under the zip (path normalized, URL-decoded), or an empty Vec if the `.app` ships no source (symbol-only).
  - `fn app_content_hash(app_path: &std::path::Path) -> anyhow::Result<String>` — blake3 hex of the whole `.app` file bytes.

- [ ] **Step 1: Write the failing test**

In `src/snapshot/embedded.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // Set CDO_APP to a real ShowMyCode .app to exercise extraction.
    fn cdo_app() -> Option<std::path::PathBuf> {
        std::env::var_os("CDO_APP").map(std::path::PathBuf::from).filter(|p| p.exists())
    }

    #[test]
    fn extracts_al_source_from_showmycode_app() {
        let Some(app) = cdo_app() else { return; };
        let files = extract_embedded_source(&app).expect("extract");
        assert!(files.len() > 100, "ShowMyCode app should yield many .al files, got {}", files.len());
        assert!(files.iter().all(|f| f.virtual_path.ends_with(".al")));
        assert!(files.iter().any(|f| f.text.contains("codeunit") || f.text.contains("table")));
    }

    #[test]
    fn content_hash_is_stable() {
        let Some(app) = cdo_app() else { return; };
        assert_eq!(app_content_hash(&app).unwrap(), app_content_hash(&app).unwrap());
        assert_eq!(app_content_hash(Path::new(&app)).unwrap().len(), 64);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p al-call-hierarchy snapshot::embedded 2>&1 | tail -15`
Expected: FAIL — `cannot find function extract_embedded_source`.

- [ ] **Step 3: Write minimal implementation**

In `src/snapshot/embedded.rs` (above tests):
```rust
//! Extract embedded ShowMyCode `.al` source from a `.app` package.

use anyhow::{Context, Result};
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::Path;

/// `.app` files start with a 40-byte NAVX header, then a standard zip.
const NAVX_HEADER_SIZE: u64 = 40;

/// One embedded source file recovered from a `.app`.
#[derive(Clone, Debug)]
pub struct SourceFile {
    pub virtual_path: String,
    pub text: String,
}

/// blake3 hex of the whole `.app` file (artifact identity).
pub fn app_content_hash(app_path: &Path) -> Result<String> {
    let bytes = std::fs::read(app_path)
        .with_context(|| format!("read .app: {}", app_path.display()))?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

/// Extract every `*.al` entry from the `.app`'s embedded zip. Empty Vec if the
/// app ships no source (symbol-only / runtime app).
pub fn extract_embedded_source(app_path: &Path) -> Result<Vec<SourceFile>> {
    let bytes = std::fs::read(app_path)
        .with_context(|| format!("read .app: {}", app_path.display()))?;
    if (bytes.len() as u64) < NAVX_HEADER_SIZE {
        return Ok(Vec::new());
    }
    let mut cursor = Cursor::new(bytes);
    cursor.seek(SeekFrom::Start(NAVX_HEADER_SIZE))?;
    // The zip starts at the first PK signature at/after the header.
    let mut archive = match zip::ZipArchive::new(cursor) {
        Ok(a) => a,
        Err(_) => return Ok(Vec::new()),
    };
    let mut out = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().to_string();
        if !name.to_ascii_lowercase().ends_with(".al") {
            continue;
        }
        let mut buf = String::new();
        // AL source is UTF-8 (BOM possible); read lossily to never panic.
        let mut raw = Vec::new();
        entry.read_to_end(&mut raw)?;
        buf.push_str(&String::from_utf8_lossy(strip_bom(&raw)));
        out.push(SourceFile {
            virtual_path: url_decode(&name),
            text: buf,
        });
    }
    Ok(out)
}

fn strip_bom(b: &[u8]) -> &[u8] {
    if b.starts_with(&[0xEF, 0xBB, 0xBF]) { &b[3..] } else { b }
}

/// Minimal `%XX` percent-decode (zip entry names are URL-encoded by the compiler).
fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
```
> NOTE: if `zip::ZipArchive::new` rejects the offset stream on some `.app`s, mirror the exact reader `app_package.rs::extract_app_package` uses (read `app_package.rs:251+` for the precise construction) — reuse that helper rather than duplicating. Prefer extracting `app_package.rs`'s zip-open into a shared `fn open_app_zip(path) -> Result<ZipArchive<...>>` and call it from both.

- [ ] **Step 4: Run test to verify it passes**

Run: `CDO_APP="U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud/Continia Software_Continia Document Output_29.0.0.0.app" cargo test -p al-call-hierarchy snapshot::embedded 2>&1 | tail -10`
Expected: PASS (extracts 551 `.al` files; both tests pass).

- [ ] **Step 5: Format, lint, commit**

```bash
rustfmt src/snapshot/embedded.rs src/snapshot/mod.rs
cargo clippy --release -- -D warnings 2>&1 | tail -5
git add src/snapshot/embedded.rs src/snapshot/mod.rs
git commit -m "feat(snapshot): extract embedded ShowMyCode .al source from .app"
```

---

### Task 3: `SourceProvider` trait + Workspace + EmbeddedApp providers

**Files:**
- Create: `src/snapshot/provider.rs`
- Modify: `src/snapshot/mod.rs` (add `pub mod provider;`)
- Test: in-file `#[cfg(test)]` in `src/snapshot/provider.rs`

**Interfaces:**
- Consumes: `SourceFile`, `extract_embedded_source`, `app_content_hash` (Task 2); `AppId`, `TrustTier` (Task 1).
- Produces:
  - `struct SourceRoot { files: Vec<SourceFile>, tier: TrustTier, content_hash: String }`
  - `trait SourceProvider { fn try_provide(&self, app: &AppId) -> anyhow::Result<Option<SourceRoot>>; }`
  - `struct WorkspaceProvider { root: std::path::PathBuf }`
  - `struct EmbeddedAppProvider { app_path: std::path::PathBuf }`

- [ ] **Step 1: Write the failing test**

In `src/snapshot/provider.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::identity::{AppId, TrustTier};

    fn dummy_app() -> AppId {
        AppId { guid: "g".into(), name: "Continia Document Output".into(),
                publisher: "Continia Software".into(), version: "29.0.0.0".into() }
    }

    #[test]
    fn embedded_provider_yields_source_with_tier() {
        let Some(app_path) = std::env::var_os("CDO_APP").map(std::path::PathBuf::from)
            .filter(|p| p.exists()) else { return; };
        let p = EmbeddedAppProvider { app_path };
        let root = p.try_provide(&dummy_app()).unwrap().expect("source");
        assert_eq!(root.tier, TrustTier::EmbeddedSource);
        assert!(root.files.len() > 100);
        assert_eq!(root.content_hash.len(), 64);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p al-call-hierarchy snapshot::provider 2>&1 | tail -15`
Expected: FAIL — `cannot find type EmbeddedAppProvider`.

- [ ] **Step 3: Write minimal implementation**

In `src/snapshot/provider.rs` (above tests):
```rust
//! Source providers: acquire per-app source by the best available means.

use crate::snapshot::embedded::{app_content_hash, extract_embedded_source, SourceFile};
use crate::snapshot::identity::{AppId, TrustTier};
use anyhow::Result;
use std::path::PathBuf;
use walkdir::WalkDir;

/// A resolved set of source files for one app, with its trust tier + hash.
#[derive(Clone, Debug)]
pub struct SourceRoot {
    pub files: Vec<SourceFile>,
    pub tier: TrustTier,
    pub content_hash: String,
}

/// Acquires source for an app. Returns `Ok(None)` when this provider cannot
/// serve the app (caller falls through to the next provider).
pub trait SourceProvider {
    fn try_provide(&self, app: &AppId) -> Result<Option<SourceRoot>>;
}

/// The app under development — source on disk is truth.
pub struct WorkspaceProvider {
    pub root: PathBuf,
}

impl SourceProvider for WorkspaceProvider {
    fn try_provide(&self, _app: &AppId) -> Result<Option<SourceRoot>> {
        let mut files = Vec::new();
        let mut hasher = blake3::Hasher::new();
        for entry in WalkDir::new(&self.root).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|x| x.to_str()) != Some("al") {
                continue;
            }
            // Skip dependency/output dirs.
            if path.components().any(|c| {
                matches!(c.as_os_str().to_str(), Some(".alpackages") | Some(".snapshots") | Some("node_modules"))
            }) {
                continue;
            }
            let text = std::fs::read_to_string(path).unwrap_or_default();
            hasher.update(text.as_bytes());
            files.push(SourceFile {
                virtual_path: path.strip_prefix(&self.root).unwrap_or(path).to_string_lossy().replace('\\', "/"),
                text,
            });
        }
        if files.is_empty() {
            return Ok(None);
        }
        files.sort_by(|a, b| a.virtual_path.cmp(&b.virtual_path));
        Ok(Some(SourceRoot {
            files,
            tier: TrustTier::Workspace,
            content_hash: hasher.finalize().to_hex().to_string(),
        }))
    }
}

/// Embedded ShowMyCode source inside a dependency `.app`.
pub struct EmbeddedAppProvider {
    pub app_path: PathBuf,
}

impl SourceProvider for EmbeddedAppProvider {
    fn try_provide(&self, _app: &AppId) -> Result<Option<SourceRoot>> {
        let files = extract_embedded_source(&self.app_path)?;
        if files.is_empty() {
            return Ok(None); // symbol-only app
        }
        Ok(Some(SourceRoot {
            files,
            tier: TrustTier::EmbeddedSource,
            content_hash: app_content_hash(&self.app_path)?,
        }))
    }
}
```
> `walkdir` is already a transitive dep via `notify`/others but may not be direct. Confirm with `grep -n "^walkdir" Cargo.toml`; if absent, either add `walkdir = "2"` to `Cargo.toml` `[dependencies]` (and note it in the commit) OR reuse the existing workspace file-walk in `indexer.rs`/`l3_workspace.rs` (`grep -n "WalkDir\|read_dir\|walk" src/indexer.rs src/engine/l3/l3_workspace.rs`) and call that instead. Prefer reuse.

- [ ] **Step 4: Run test to verify it passes**

Run: `CDO_APP="U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud/Continia Software_Continia Document Output_29.0.0.0.app" cargo test -p al-call-hierarchy snapshot::provider 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Format, lint, commit**

```bash
rustfmt src/snapshot/provider.rs src/snapshot/mod.rs
cargo clippy --release -- -D warnings 2>&1 | tail -5
git add src/snapshot/provider.rs src/snapshot/mod.rs Cargo.toml
git commit -m "feat(snapshot): SourceProvider trait + Workspace + EmbeddedApp providers"
```

---

### Task 4: IdentityVerifier + LocalRepo/SymbolOnly providers + selection

**Files:**
- Create: `src/snapshot/verify.rs`
- Modify: `src/snapshot/provider.rs` (add `LocalRepoProvider`, `SymbolOnlyProvider`)
- Modify: `src/snapshot/mod.rs` (add `pub mod verify;`)
- Test: in-file `#[cfg(test)]` in `src/snapshot/verify.rs`

**Interfaces:**
- Consumes: `AppId`, `TrustTier` (Task 1); `SourceRoot`, `SourceProvider` (Task 3); `ParsedAppPackage`/`AppMetadata`/`extract_app_package` (existing `app_package.rs`).
- Produces:
  - `enum IdentityCheck { Verified, Approximate(String), Mismatch(String) }`
  - `fn verify_local_source(app: &AppId, root: &SourceRoot, expected_app_json: Option<&AppId>) -> IdentityCheck`
  - `struct LocalRepoProvider { app: AppId, root: PathBuf }`
  - `struct SymbolOnlyProvider;` (always `Ok(None)` for source — marks the app symbol-only)
  - `fn select_source(app: &AppId, providers: &[Box<dyn SourceProvider>]) -> Result<Option<SourceRoot>>` — first provider (in priority order) that yields `Some`.

- [ ] **Step 1: Write the failing test**

In `src/snapshot/verify.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::identity::{AppId, TrustTier};
    use crate::snapshot::provider::SourceRoot;

    fn app(v: &str) -> AppId {
        AppId { guid: "g".into(), name: "Core".into(), publisher: "Continia".into(), version: v.into() }
    }
    fn root() -> SourceRoot {
        SourceRoot { files: vec![], tier: TrustTier::LocalSourceApproximate, content_hash: "h".into() }
    }

    #[test]
    fn matching_version_verifies() {
        let r = verify_local_source(&app("29.0.0.0"), &root(), Some(&app("29.0.0.0")));
        assert!(matches!(r, IdentityCheck::Verified | IdentityCheck::Approximate(_)));
    }

    #[test]
    fn version_mismatch_fails_closed() {
        let r = verify_local_source(&app("29.0.0.0"), &root(), Some(&app("28.0.0.0")));
        assert!(matches!(r, IdentityCheck::Mismatch(_)));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p al-call-hierarchy snapshot::verify 2>&1 | tail -15`
Expected: FAIL — `cannot find function verify_local_source`.

- [ ] **Step 3: Write minimal implementation**

In `src/snapshot/verify.rs`:
```rust
//! Source-identity verification: source is only "sound" if it provably
//! matches the artifact under analysis; mismatch fails closed.

use crate::snapshot::identity::AppId;
use crate::snapshot::provider::SourceRoot;

/// Outcome of checking that a source root matches the app it claims to be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityCheck {
    /// Identity corroborated (e.g. git commit / source hash recorded).
    Verified,
    /// Id/version match but no strong corroboration — usable, never "sound".
    Approximate(String),
    /// Wrong app/version — must fall back to symbol-only.
    Mismatch(String),
}

/// Verify a LOCAL source root against the expected app identity (from app.json
/// or the matched `.app`). Embedded source is implicitly bound to its `.app`
/// and does not pass through here.
pub fn verify_local_source(
    app: &AppId,
    _root: &SourceRoot,
    expected: Option<&AppId>,
) -> IdentityCheck {
    let Some(exp) = expected else {
        return IdentityCheck::Approximate("no expected app.json identity to compare".into());
    };
    if exp.guid != app.guid && !exp.guid.is_empty() && !app.guid.is_empty() {
        return IdentityCheck::Mismatch(format!("guid {} != {}", app.guid, exp.guid));
    }
    if exp.version != app.version {
        return IdentityCheck::Mismatch(format!(
            "version {} != expected {}",
            app.version, exp.version
        ));
    }
    // Id+version match; no commit/source-hash corroboration yet -> approximate.
    IdentityCheck::Approximate("id+version match; no build corroboration".into())
}
```
Then in `src/snapshot/provider.rs` add (using existing `app_package`):
```rust
/// No source available — marks the app symbol-only (honest boundary).
pub struct SymbolOnlyProvider;

impl SourceProvider for SymbolOnlyProvider {
    fn try_provide(&self, _app: &AppId) -> Result<Option<SourceRoot>> {
        Ok(None)
    }
}

/// First provider (in priority order) that yields source wins.
pub fn select_source(
    app: &AppId,
    providers: &[Box<dyn SourceProvider>],
) -> Result<Option<SourceRoot>> {
    for p in providers {
        if let Some(root) = p.try_provide(app)? {
            return Ok(Some(root));
        }
    }
    Ok(None)
}
```
(Implement `LocalRepoProvider` analogously to `WorkspaceProvider` but rooted at a configured repo path and tier `LocalSourceApproximate`, downgrading/failing per `verify_local_source`. Keep it minimal — full config UX is a later task.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p al-call-hierarchy snapshot::verify 2>&1 | tail -10`
Expected: PASS (2 tests).

- [ ] **Step 5: Format, lint, commit**

```bash
rustfmt src/snapshot/verify.rs src/snapshot/provider.rs src/snapshot/mod.rs
cargo clippy --release -- -D warnings 2>&1 | tail -5
git add src/snapshot/verify.rs src/snapshot/provider.rs src/snapshot/mod.rs
git commit -m "feat(snapshot): identity verification + symbol-only/local providers + selection"
```

---

### Task 5: `CompilationContext` per app

**Files:**
- Create: `src/snapshot/compilation.rs`
- Modify: `src/snapshot/mod.rs` (add `pub mod compilation;`)
- Test: in-file `#[cfg(test)]`

**Interfaces:**
- Produces:
  - `struct CompilationContext { preproc_symbols: std::collections::BTreeSet<String>, runtime: Option<String>, platform: Option<String>, application: Option<String> }` (Clone, Debug, Default)
  - `fn context_from_app_json(app_json: &serde_json::Value) -> CompilationContext` — reads `runtime`, `platform`, `application`, and any declared preprocessor symbols (BC has no source-level symbol list in app.json by default; seed from build config / `ParsedAppPackage` manifest where available, else empty + mark branches `conditional-unverified` downstream).

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_runtime_and_platform() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"runtime":"15.0","platform":"28.0.0.0","application":"28.0.0.0"}"#).unwrap();
        let c = context_from_app_json(&v);
        assert_eq!(c.runtime.as_deref(), Some("15.0"));
        assert_eq!(c.platform.as_deref(), Some("28.0.0.0"));
        assert!(c.preproc_symbols.is_empty());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p al-call-hierarchy snapshot::compilation 2>&1 | tail -10`
Expected: FAIL — `cannot find function context_from_app_json`.

- [ ] **Step 3: Write minimal implementation**

```rust
//! Per-app compilation context: each app's own preprocessor symbols + version
//! basis, so dependency `#if` branches are evaluated with THAT app's context,
//! never the workspace's (phantom-edge prevention, charter C3).

use std::collections::BTreeSet;

#[derive(Clone, Debug, Default)]
pub struct CompilationContext {
    pub preproc_symbols: BTreeSet<String>,
    pub runtime: Option<String>,
    pub platform: Option<String>,
    pub application: Option<String>,
}

pub fn context_from_app_json(app_json: &serde_json::Value) -> CompilationContext {
    let get = |k: &str| app_json.get(k).and_then(|v| v.as_str()).map(str::to_string);
    CompilationContext {
        preproc_symbols: BTreeSet::new(),
        runtime: get("runtime"),
        platform: get("platform"),
        application: get("application"),
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p al-call-hierarchy snapshot::compilation 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Format, lint, commit**

```bash
rustfmt src/snapshot/compilation.rs src/snapshot/mod.rs
cargo clippy --release -- -D warnings 2>&1 | tail -5
git add src/snapshot/compilation.rs src/snapshot/mod.rs
git commit -m "feat(snapshot): per-app CompilationContext from app.json"
```

---

### Task 6: `AppUnit` + `AppSetSnapshot` + `SnapshotBuilder`

**Files:**
- Create: `src/snapshot/snapshot.rs`
- Modify: `src/snapshot/mod.rs` (re-export `AppSetSnapshot`, `AppUnit`, `SnapshotBuilder`)
- Test: in-file `#[cfg(test)]` (integration over the real CDO workspace, guarded by env var)

**Interfaces:**
- Consumes: everything above; `dependencies::{find_all_alpackages_folders, parse_app_json, find_matching_app, load_all_apps, AppDependency, ResolvedDependency}`; `app_package::extract_app_package`.
- Produces:
  - `struct AppUnit { id: AppId, provenance: Provenance, source: Option<SourceRoot>, compilation: CompilationContext, abi: Option<ParsedAppPackage> }`
  - `struct AppSetSnapshot { apps: Vec<AppUnit>, workspace_app: AppId, world: World }`
  - `enum World { Closed, Open }`
  - `struct SnapshotBuilder { workspace_root: PathBuf, local_providers: Vec<(AppId, PathBuf)> }` with `fn build(&self) -> anyhow::Result<AppSetSnapshot>`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builds_snapshot_over_cdo_workspace() {
        let Some(ws) = std::env::var_os("CDO_WS").map(std::path::PathBuf::from)
            .filter(|p| p.exists()) else { return; };
        let snap = SnapshotBuilder { workspace_root: ws, local_providers: vec![] }
            .build().expect("snapshot");
        // workspace + 10 deps
        assert!(snap.apps.len() >= 10, "got {}", snap.apps.len());
        // 9/10 deps ship source -> at least 9 units have source
        let with_src = snap.apps.iter().filter(|u| u.source.is_some()).count();
        assert!(with_src >= 9, "expected >=9 source units, got {with_src}");
        // exactly one symbol-only (Microsoft_Application)
        let sym_only = snap.apps.iter().filter(|u| u.source.is_none()).count();
        assert!(sym_only >= 1);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p al-call-hierarchy snapshot::snapshot 2>&1 | tail -15`
Expected: FAIL — `cannot find type SnapshotBuilder`.

- [ ] **Step 3: Write minimal implementation**

In `src/snapshot/snapshot.rs`: define the structs above; `build()` (a) reads workspace `app.json` → `workspace_app` `AppId` + `CompilationContext`; builds the `WorkspaceProvider` unit; (b) `dependencies::load_all_apps(workspace_root)` → for each `ResolvedDependency`, derive `AppId` (from its `app.json` dep entry + `.app` `AppMetadata`), select source via `[EmbeddedAppProvider, configured LocalRepoProvider, SymbolOnlyProvider]`, run `extract_app_package` for the ABI side, attach `CompilationContext`. Return `AppSetSnapshot { apps, workspace_app, world: World::Closed }`.
Wire all field/type names exactly as declared in the Interfaces blocks above (run the type-consistency check in self-review). Keep `World::Open` reserved (reverse-dependents are a later task).

- [ ] **Step 4: Run test to verify it passes**

Run: `CDO_WS="U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud" cargo test -p al-call-hierarchy snapshot::snapshot 2>&1 | tail -12`
Expected: PASS — snapshot has ≥10 apps, ≥9 with source, ≥1 symbol-only.

- [ ] **Step 5: Format, lint, commit**

```bash
rustfmt src/snapshot/snapshot.rs src/snapshot/mod.rs
cargo clippy --release -- -D warnings 2>&1 | tail -5
git add src/snapshot/snapshot.rs src/snapshot/mod.rs
git commit -m "feat(snapshot): AppUnit + AppSetSnapshot + SnapshotBuilder (deep ingestion)"
```

---

### Task 7: Deep parse of snapshot source into the IR

**Files:**
- Create: `src/snapshot/parse.rs`
- Modify: `src/snapshot/mod.rs`
- Test: in-file `#[cfg(test)]` (guarded by `CDO_WS`)

**Interfaces:**
- Consumes: `AppSetSnapshot`, `AppUnit`, `SourceFile`; `al_syntax::parse(&str) -> al_syntax::AlFile`.
- Produces:
  - `struct ParsedUnit { app: AppId, files: Vec<ParsedFile> }`
  - `struct ParsedFile { virtual_path: String, file: al_syntax::AlFile, provenance: Provenance }`
  - `fn parse_snapshot(snap: &AppSetSnapshot) -> Vec<ParsedUnit>` — parses every source file of every source-bearing unit (parallel via `rayon`, matching `indexer.rs`); symbol-only units contribute no `ParsedUnit` source (their ABI is used in Plan 1B).

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_all_source_units_zero_panics() {
        let Some(ws) = std::env::var_os("CDO_WS").map(std::path::PathBuf::from)
            .filter(|p| p.exists()) else { return; };
        let snap = crate::snapshot::SnapshotBuilder { workspace_root: ws, local_providers: vec![] }
            .build().unwrap();
        let parsed = parse_snapshot(&snap);
        assert!(!parsed.is_empty());
        let total_files: usize = parsed.iter().map(|u| u.files.len()).sum();
        assert!(total_files > 1000, "deep parse should cover many files, got {total_files}");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p al-call-hierarchy snapshot::parse 2>&1 | tail -12`
Expected: FAIL — `cannot find function parse_snapshot`.

- [ ] **Step 3: Write minimal implementation**

In `src/snapshot/parse.rs`: iterate `snap.apps`, for each unit with `Some(source)`, `rayon::prelude::*` `par_iter()` over `source.files`, call `al_syntax::parse(&f.text)`, collect `ParsedFile { virtual_path, file, provenance: unit.provenance.clone() }`. (Mirror the rayon usage in `src/indexer.rs` — `grep -n "par_iter\|rayon" src/indexer.rs`.)

- [ ] **Step 4: Run test to verify it passes**

Run: `CDO_WS="U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud" cargo test -p al-call-hierarchy snapshot::parse 2>&1 | tail -10`
Expected: PASS (parses >1000 files across the source units).

- [ ] **Step 5: Format, lint, commit**

```bash
rustfmt src/snapshot/parse.rs src/snapshot/mod.rs
cargo clippy --release -- -D warnings 2>&1 | tail -5
git add src/snapshot/parse.rs src/snapshot/mod.rs
git commit -m "feat(snapshot): deep-parse snapshot source into the owned IR (rayon)"
```

---

### Task 8: Content-addressed cache + robustness gate

**Files:**
- Create: `src/snapshot/cache.rs`
- Create: `tests/snapshot_robustness.rs`
- Modify: `src/snapshot/mod.rs`
- Test: `tests/snapshot_robustness.rs`

**Interfaces:**
- Consumes: `extract_embedded_source`, `app_content_hash`, `SourceRoot`.
- Produces:
  - `fn cache_dir() -> std::path::PathBuf` (under the system cache or a `.al-ch-cache/` in workspace)
  - `fn cached_source(app_path: &Path) -> Result<Vec<SourceFile>>` — returns cached extracted source keyed by `app_content_hash`, extracting + storing on miss.

- [ ] **Step 1: Write the failing test (robustness — the real value gate)**

In `tests/snapshot_robustness.rs`:
```rust
//! Spec 1 robustness: building + deep-parsing the CDO snapshot never panics
//! and recovers an Unknown-free lowering on clean source.
#[test]
fn cdo_snapshot_deep_parse_is_panic_free() {
    let Some(ws) = std::env::var_os("CDO_WS").map(std::path::PathBuf::from)
        .filter(|p| p.exists()) else { return; };
    let snap = al_call_hierarchy::snapshot::SnapshotBuilder {
        workspace_root: ws, local_providers: vec![],
    }.build().expect("snapshot builds");
    let parsed = al_call_hierarchy::snapshot::parse_snapshot(&snap);
    // No panic reaching here is the assertion; sanity on coverage:
    let files: usize = parsed.iter().map(|u| u.files.len()).sum();
    assert!(files > 1000);
}
```
> Confirm the crate name for integration tests: `grep -n "^name" Cargo.toml` (top `[package]`). Use that as the `extern` crate path (here assumed `al_call_hierarchy`).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p al-call-hierarchy --test snapshot_robustness 2>&1 | tail -12`
Expected: FAIL — `parse_snapshot`/`SnapshotBuilder` not public at crate root, or cache module missing.

- [ ] **Step 3: Write minimal implementation**

Implement `src/snapshot/cache.rs` (blake3-keyed dir of extracted source; JSON or raw files). Ensure `snapshot` re-exports `SnapshotBuilder`, `AppSetSnapshot`, `parse_snapshot`, `ParsedUnit` at `crate::snapshot::`. The cache is used by `EmbeddedAppProvider` (swap `extract_embedded_source` → `cached_source`).

- [ ] **Step 4: Run test to verify it passes**

Run: `CDO_WS="U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud" cargo test -p al-call-hierarchy --test snapshot_robustness 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Full gate + commit**

```bash
rustfmt src/snapshot/cache.rs src/snapshot/mod.rs tests/snapshot_robustness.rs
cargo clippy --release --all-features -- -D warnings 2>&1 | tail -5
cargo test --workspace 2>&1 | grep -E 'test result:|FAILED' | tail -20
git add src/snapshot/cache.rs src/snapshot/mod.rs tests/snapshot_robustness.rs
git commit -m "feat(snapshot): content-addressed source cache + robustness gate"
```

---

## Self-Review

**Spec coverage (Plan 1A vs Spec 1 §3.1–3.3, §3.7, §8 steps 1–4 & 8):**
- §3.1 SourceProvider → Tasks 3, 4 ✓ · §3.2 IdentityVerifier → Task 4 ✓ · §3.3 CompilationContext → Task 5 ✓ · §3.7 cache + determinism → Task 8 (determinism partial — full stable-ID is Plan 1B with NodeId) ✓ · embedded extraction (the de-risker) → Task 2 ✓ · snapshot model → Task 6 ✓ · deep parse → Task 7 ✓.
- Deferred to Plan 1B (correct): NodeId/topology resolution, 2-axis Edge, AbiCrossCheck, deep re-baseline metric (§3.4–3.6, §4.1–4.2, §8 steps 5–7, 9).

**Placeholder scan:** Two intentional "confirm with grep" steps (module-root file in Task 1; walkdir/crate-name in Tasks 3/8) — these are *verification* steps with the exact command + fallback, not deferred work. No `TODO`/`add appropriate X`.

**Type consistency:** `SourceFile{virtual_path,text}`, `SourceRoot{files,tier,content_hash}`, `AppId{guid,name,publisher,version}`, `TrustTier`, `Provenance{app,tier,content_hash}`, `AppUnit{id,provenance,source,compilation,abi}`, `AppSetSnapshot{apps,workspace_app,world}`, `parse_snapshot`/`ParsedUnit{app,files}`/`ParsedFile{virtual_path,file,provenance}` — consistent across Tasks 1–8.

**Known follow-ups (Plan 1B):** `LocalRepoProvider` full config UX; per-app preproc symbol *sourcing* (Task 5 reads versions, not symbols — flagged in Spec §7 as a spike); determinism of serialized graph (needs NodeId).
