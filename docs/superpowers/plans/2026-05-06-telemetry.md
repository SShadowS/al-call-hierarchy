# Telemetry Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add anonymous failure-diagnostics telemetry to the AL Call Hierarchy LSP server so resolution gaps, parser anomalies, and indexer issues from real installs surface to the maintainer without leaking customer code.

**Architecture:** Behind a default-on `telemetry` cargo feature, a `src/telemetry/` module exposes `record_*` functions that LSP request threads call (sync, ~5µs hot path: atomic check + domain-separated salted blake3 hash + workspace-scoped LRU dedup + non-blocking `try_send` to a bounded mpsc). A dedicated background thread runs a tokio current-thread runtime hosting an OpenTelemetry SDK with `opentelemetry-application-insights` exporter; it batches and ships to Azure Application Insights and constructs the unsampled `session.summary` directly after queue drain. Telemetry is opt-out (off via `DO_NOT_TRACK`, `--no-telemetry`, env, config), off-by-default in dev/CI builds, and produces only structural fingerprints + 128-bit per-installation salted hashes.

**Tech Stack:** Rust 1.75+, `blake3` (hashing), `lru` (dedup), `tokio` (current-thread runtime, rt feature), `opentelemetry` + `opentelemetry_sdk` + `opentelemetry-application-insights` + `tracing` + `tracing-opentelemetry` (telemetry pipeline).

**Spec:** `docs/superpowers/specs/2026-05-06-telemetry-design.md`. Section references in tasks point to the numbered sections in the spec.

---

## File Structure

| File | Purpose | Phase |
|---|---|---|
| `Cargo.toml` | Add `telemetry` feature, deps | 0, 1 |
| `build.rs` | Read `AL_CH_TELEMETRY_CONNECTION_STRING` at build time | 3 |
| `src/main.rs` | Add `--no-telemetry` CLI flag, call `telemetry::init`/`shutdown` | 0, 1 |
| `src/server.rs` | Wire `init`/`shutdown` around server loop | 0, 1 |
| `src/handlers.rs` | Add `al-call-hierarchy/telemetryStatus`; instrument `record_handler_empty` | 1, 2 |
| `src/parser.rs` | Instrument resolution misses + parser errors | 2 |
| `src/indexer.rs` | Instrument indexer issues | 2 |
| `src/config.rs` | Extend with `telemetry` block alongside existing `diagnostics` | 0 |
| `src/telemetry/mod.rs` | Public API: `init`, `shutdown`, `record_*`, `status` | 0 |
| `src/telemetry/consent.rs` | Resolution order with CI/debug detection | 0 |
| `src/telemetry/install_id.rs` | Salt generation/load at `~/.al-call-hierarchy/installation-id` | 0 |
| `src/telemetry/hash.rs` | `blake3_keyed` with domain separation, 128-bit truncation | 0 |
| `src/telemetry/session_marker.rs` | `~/.al-call-hierarchy/session.lock` create/delete | 0 |
| `src/telemetry/events.rs` | Event structs + `EventKind` + `ALL_LEAF_KINDS` const + OTLP attribute mapping | 0, 1 |
| `src/telemetry/counters.rs` | Atomic counter arrays shared producer↔background | 1 |
| `src/telemetry/dedup.rs` | LRU + workspace-scoped keys + TTL sweep | 1 |
| `src/telemetry/pipeline.rs` | mpsc + try_send + producer-side counter bumps | 1 |
| `src/telemetry/exporter.rs` | Tokio runtime + OTel SDK init + AI exporter + retry | 1 |
| `src/telemetry/summary.rs` | Post-drain summary construction | 1 |
| `src/telemetry/status.rs` | Snapshot of counters for transparency endpoint | 1 |
| `tests/fixtures/telemetry/unresolved_app_dep/` | AL workspace with unresolved .app-dep call | 2 |
| `tests/fixtures/telemetry/parser_error/` | Malformed AL file | 2 |
| `tests/fixtures/telemetry/missing_dep/` | app.json declaring missing dep | 2 |
| `tests/telemetry_integration.rs` | End-to-end pipeline + fixtures | 1, 2 |
| `tests/telemetry_privacy_lint.rs` | Source-scan: no raw String fields outside hash domain | 0 |
| `benches/telemetry_hot_path.rs` | Criterion bench for hot-path budget | 1 |
| `tools/telemetry-spike/main.rs` | Phase 0.5 spike binary | 0.5 |
| `docs/telemetry-ingestion-spike.md` | Phase 0.5 findings | 0.5 |
| `docs/telemetry.md` | Schema reference for users | 3 |
| `docs/telemetry-smoke-test.md` | Manual smoke test procedure | 3 |
| `README.md` | "Telemetry" section above "Installation" | 3 |
| `CHANGELOG.md` | `Added` entry under 0.7.0 | 3 |

---

## Phase 0 — Foundation (PR #1)

**Phase goal:** Skeleton compiles with and without `--no-default-features`. All foundational utilities (hash, install_id, consent, session_marker, config extension, event structs) implemented and unit-tested. `record_*` functions exist as no-ops. `init`/`shutdown` wired into `server.rs`. No LSP behavior change.

### Task 0.1: Add `telemetry` cargo feature flag

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add features section to Cargo.toml**

Add after the `[dependencies]` section:

```toml
[features]
default = ["telemetry"]
telemetry = []
```

(The feature is currently empty — deps gate on it later in Phase 1.)

- [ ] **Step 2: Verify both build configurations compile**

Run:
```bash
cargo build
cargo build --no-default-features
```
Expected: both succeed with no warnings.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml
git commit -m "feat(telemetry): add default-on telemetry cargo feature"
```

---

### Task 0.2: Create telemetry module skeleton

**Files:**
- Create: `src/telemetry/mod.rs`
- Modify: `src/main.rs` (add `mod telemetry;`)

- [ ] **Step 1: Create empty module**

Write `src/telemetry/mod.rs`:

```rust
//! Anonymous failure-diagnostics telemetry.
//!
//! See `docs/superpowers/specs/2026-05-06-telemetry-design.md` for the full design.
//!
//! When the `telemetry` feature is disabled, all public functions are no-ops
//! that compile to a single early return.

#![allow(dead_code)] // Stubs come online in Phase 0/1.

#[cfg(feature = "telemetry")]
mod consent;
#[cfg(feature = "telemetry")]
mod hash;
#[cfg(feature = "telemetry")]
mod install_id;
#[cfg(feature = "telemetry")]
mod session_marker;
#[cfg(feature = "telemetry")]
pub mod events;

/// Opaque handle returned from `init` and passed to `shutdown`.
/// When telemetry is disabled, this is a zero-sized type.
#[cfg(feature = "telemetry")]
pub struct TelemetryHandle {
    _private: (),
}

#[cfg(not(feature = "telemetry"))]
pub struct TelemetryHandle;

/// Initialize the telemetry subsystem. Returns a no-op handle when disabled.
pub fn init() -> TelemetryHandle {
    #[cfg(feature = "telemetry")]
    {
        TelemetryHandle { _private: () }
    }
    #[cfg(not(feature = "telemetry"))]
    {
        TelemetryHandle
    }
}

/// Shut down telemetry. Drains the queue and emits the session summary.
pub fn shutdown(_handle: TelemetryHandle) {
    // Phase 1 wires this up; Phase 0 stub is a no-op.
}
```

- [ ] **Step 2: Register module in main.rs**

Edit `src/main.rs`. After the existing `mod` lines (around line 6-17), add:

```rust
mod telemetry;
```

- [ ] **Step 3: Verify both feature configs compile**

```bash
cargo build
cargo build --no-default-features
```
Expected: both succeed.

- [ ] **Step 4: Commit**

```bash
git add src/telemetry/mod.rs src/main.rs
git commit -m "feat(telemetry): add telemetry module skeleton with no-op API"
```

---

### Task 0.3: Add foundation deps (blake3, dirs already present)

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add blake3 to dependencies**

In `Cargo.toml` `[dependencies]` section, add:

```toml
# Telemetry hashing
blake3 = "1"
```

`dirs = "6"` is already a dependency (used by config.rs). No change needed.

- [ ] **Step 2: Verify**

```bash
cargo build
```
Expected: succeeds, blake3 and its deps download.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "feat(telemetry): add blake3 dependency for salted hashing"
```

---

### Task 0.4: Implement `hash.rs` with TDD

**Files:**
- Create: `src/telemetry/hash.rs`
- Test: same file (`#[cfg(test)] mod tests`)

Spec reference: §5 "Hashing rules".

- [ ] **Step 1: Write the failing test**

Create `src/telemetry/hash.rs`:

```rust
//! Domain-separated, salted, truncated BLAKE3 hashes for AL identifiers.
//!
//! See spec §5 "Hashing rules". 128-bit (32-char hex) for queryable identifiers,
//! 64-bit (16-char hex) for `install_id` and `workspace_id`.

use blake3::Hasher;

/// Domain tags for hash inputs. Prevents cross-field collision.
pub const DOMAIN_OBJECT: &[u8] = b"object:";
pub const DOMAIN_PROCEDURE: &[u8] = b"procedure:";
pub const DOMAIN_APP_ID: &[u8] = b"app_id:";
pub const DOMAIN_FILE: &[u8] = b"file:";
pub const DOMAIN_WORKSPACE: &[u8] = b"workspace:";
pub const DOMAIN_NODE_KIND: &[u8] = b"node_kind:";

const MAX_INPUT_BYTES: usize = 4096;

/// 32-byte salt (the local installation-id).
pub type Salt = [u8; 32];

/// Hash an AL identifier with domain separation. Returns 32-char lowercase hex
/// (128 bits of digest).
pub fn hash_identifier(salt: &Salt, domain: &[u8], input: &str) -> String {
    let normalized = input.to_lowercase();
    let bytes = normalized.as_bytes();
    let truncated = &bytes[..bytes.len().min(MAX_INPUT_BYTES)];

    let mut h = Hasher::new_keyed(salt);
    h.update(domain);
    h.update(truncated);
    let digest = h.finalize();
    hex_lower_truncated(digest.as_bytes(), 16)
}

/// 16-char hex form for `install_id`/`workspace_id` (64 bits).
pub fn hash_short(salt: &Salt, domain: &[u8], input: &[u8]) -> String {
    let mut h = Hasher::new_keyed(salt);
    h.update(domain);
    let truncated = &input[..input.len().min(MAX_INPUT_BYTES)];
    h.update(truncated);
    let digest = h.finalize();
    hex_lower_truncated(digest.as_bytes(), 8)
}

/// Compute the public `install_id` from the salt itself: blake3(salt)[..8] → 16 hex.
pub fn install_id_from_salt(salt: &Salt) -> String {
    let digest = blake3::hash(salt);
    hex_lower_truncated(digest.as_bytes(), 8)
}

fn hex_lower_truncated(bytes: &[u8], n: usize) -> String {
    let mut s = String::with_capacity(n * 2);
    for &b in &bytes[..n] {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn salt() -> Salt {
        [0x42; 32]
    }

    #[test]
    fn hash_identifier_is_deterministic() {
        let s = salt();
        let a = hash_identifier(&s, DOMAIN_PROCEDURE, "PostInvoice");
        let b = hash_identifier(&s, DOMAIN_PROCEDURE, "PostInvoice");
        assert_eq!(a, b);
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_identifier_is_case_insensitive() {
        let s = salt();
        let a = hash_identifier(&s, DOMAIN_PROCEDURE, "PostInvoice");
        let b = hash_identifier(&s, DOMAIN_PROCEDURE, "postinvoice");
        let c = hash_identifier(&s, DOMAIN_PROCEDURE, "POSTINVOICE");
        assert_eq!(a, b);
        assert_eq!(a, c);
    }

    #[test]
    fn different_salts_produce_different_hashes() {
        let s1 = [0x42; 32];
        let s2 = [0x43; 32];
        let a = hash_identifier(&s1, DOMAIN_PROCEDURE, "PostInvoice");
        let b = hash_identifier(&s2, DOMAIN_PROCEDURE, "PostInvoice");
        assert_ne!(a, b);
    }

    #[test]
    fn different_domains_produce_different_hashes() {
        let s = salt();
        let as_object = hash_identifier(&s, DOMAIN_OBJECT, "Customer");
        let as_procedure = hash_identifier(&s, DOMAIN_PROCEDURE, "Customer");
        assert_ne!(as_object, as_procedure);
    }

    #[test]
    fn install_id_is_16_chars() {
        let id = install_id_from_salt(&salt());
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_short_is_16_chars() {
        let s = salt();
        let id = hash_short(&s, DOMAIN_WORKSPACE, b"/some/path");
        assert_eq!(id.len(), 16);
    }

    #[test]
    fn extreme_input_is_truncated_not_panicked() {
        let s = salt();
        let huge = "a".repeat(10 * 1024 * 1024); // 10MB
        let _ = hash_identifier(&s, DOMAIN_PROCEDURE, &huge);
        // No panic, no OOM, returns within reasonable time.
    }
}
```

- [ ] **Step 2: Register in `mod.rs`**

`src/telemetry/mod.rs` already declares `#[cfg(feature = "telemetry")] mod hash;` from Task 0.2. No change needed.

- [ ] **Step 3: Run tests**

```bash
cargo test --lib telemetry::hash
```
Expected: 7 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/telemetry/hash.rs
git commit -m "feat(telemetry): add domain-separated salted blake3 hash module"
```

---

### Task 0.5: Implement `install_id.rs` with TDD

**Files:**
- Create: `src/telemetry/install_id.rs`

Spec reference: §5 "Hashing rules" (salt storage), §9 "Error Handling" (unreadable file, unwritable directory).

- [ ] **Step 1: Write the failing tests**

Create `src/telemetry/install_id.rs`:

```rust
//! Per-installation salt management.
//!
//! Stored at `~/.al-call-hierarchy/installation-id` (32 random bytes).
//! Generated on first use; persists across runs.

use crate::telemetry::hash::Salt;
use anyhow::{Context, Result};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const SALT_BYTES: usize = 32;

/// Resolve `~/.al-call-hierarchy/installation-id`.
fn default_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".al-call-hierarchy").join("installation-id"))
}

/// Load existing salt, or generate-and-persist a fresh one.
/// Falls back to an in-memory salt if the filesystem is unwritable.
pub fn load_or_create() -> (Salt, bool /* persisted */) {
    let Some(path) = default_path() else {
        return (random_salt(), false);
    };
    load_or_create_at(&path)
}

pub fn load_or_create_at(path: &Path) -> (Salt, bool) {
    if let Ok(salt) = read_salt(path) {
        return (salt, true);
    }
    let salt = random_salt();
    if let Err(e) = persist_salt(path, &salt) {
        log::warn!(
            "telemetry: failed to persist installation-id at {}: {}. Using in-memory salt for this session.",
            path.display(),
            e
        );
        return (salt, false);
    }
    (salt, true)
}

fn read_salt(path: &Path) -> Result<Salt> {
    let mut f = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut buf = [0u8; SALT_BYTES];
    f.read_exact(&mut buf)
        .with_context(|| format!("reading 32 bytes from {}", path.display()))?;
    Ok(buf)
}

fn persist_salt(path: &Path, salt: &Salt) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating dir {}", parent.display()))?;
    }
    fs::write(path, salt).with_context(|| format!("writing {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn random_salt() -> Salt {
    let mut salt = [0u8; SALT_BYTES];
    getrandom_compat(&mut salt);
    salt
}

#[cfg(not(test))]
fn getrandom_compat(buf: &mut [u8]) {
    // Fall back to time-based weak entropy if blake3's RNG isn't desired.
    // We pull from std with a small mix; for stronger entropy use a real RNG.
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut h = blake3::Hasher::new();
    h.update(&nanos.to_le_bytes());
    h.update(&std::process::id().to_le_bytes());
    let mut reader = h.finalize_xof();
    reader.fill(buf);
}

#[cfg(test)]
fn getrandom_compat(buf: &mut [u8]) {
    // Tests need determinism; use the address of `buf` for variability.
    let seed = buf.as_ptr() as usize;
    let mut h = blake3::Hasher::new();
    h.update(&seed.to_le_bytes());
    let mut reader = h.finalize_xof();
    reader.fill(buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn first_call_creates_and_persists_salt() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("salt");
        let (salt, persisted) = load_or_create_at(&path);
        assert!(persisted);
        assert!(path.exists());
        assert_eq!(salt.len(), 32);
    }

    #[test]
    fn second_call_reuses_existing_salt() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("salt");
        let (s1, _) = load_or_create_at(&path);
        let (s2, _) = load_or_create_at(&path);
        assert_eq!(s1, s2);
    }

    #[test]
    fn corrupt_short_file_falls_back_to_new_salt() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("salt");
        fs::write(&path, b"too short").unwrap();
        let (salt, persisted) = load_or_create_at(&path);
        // We could not parse the existing file; we generate fresh and overwrite.
        assert_eq!(salt.len(), 32);
        assert!(persisted);
    }
}
```

- [ ] **Step 2: Add tempfile to dev-dependencies if not present**

Check `Cargo.toml`. `tempfile = "3"` is already in `[dev-dependencies]`. No change.

- [ ] **Step 3: Run tests**

```bash
cargo test --lib telemetry::install_id
```
Expected: 3 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/telemetry/install_id.rs
git commit -m "feat(telemetry): add install_id module for per-installation salt"
```

---

### Task 0.6: Implement `session_marker.rs` with TDD

**Files:**
- Create: `src/telemetry/session_marker.rs`

Spec reference: §6 "Crash detection (session marker)", §9 error-handling rows for `session.lock`.

- [ ] **Step 1: Write the failing tests**

Create `src/telemetry/session_marker.rs`:

```rust
//! Crash-detection marker.
//!
//! Created at startup, deleted on graceful shutdown. Presence at startup
//! signals that the previous session terminated abnormally (SIGKILL, OS crash,
//! power loss, exporter hang past shutdown budget).

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

fn default_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".al-call-hierarchy").join("session.lock"))
}

/// Result of writing the marker at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkerStatus {
    /// `true` if the marker file existed before this session started.
    pub previous_session_unclean: bool,
    /// `true` if we successfully wrote the marker for this session.
    pub created: bool,
}

/// Record this session's marker. Returns whether the previous session was unclean.
pub fn record_session_start() -> MarkerStatus {
    match default_path() {
        Some(p) => record_at(&p),
        None => MarkerStatus {
            previous_session_unclean: false,
            created: false,
        },
    }
}

pub fn record_at(path: &Path) -> MarkerStatus {
    let previously_existed = path.exists();
    let created = match write_marker(path) {
        Ok(()) => true,
        Err(e) => {
            log::warn!(
                "telemetry: failed to write session marker at {}: {}",
                path.display(),
                e
            );
            false
        }
    };
    MarkerStatus {
        previous_session_unclean: previously_existed,
        created,
    }
}

/// Delete the marker. Called on graceful shutdown after summary export.
pub fn record_clean_shutdown() {
    if let Some(p) = default_path() {
        clean_shutdown_at(&p);
    }
}

pub fn clean_shutdown_at(path: &Path) {
    if let Err(e) = fs::remove_file(path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            log::warn!(
                "telemetry: failed to remove session marker at {}: {}",
                path.display(),
                e
            );
        }
    }
}

fn write_marker(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating dir {}", parent.display()))?;
    }
    fs::write(path, b"").with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn first_session_no_previous_marker() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.lock");
        let status = record_at(&path);
        assert!(!status.previous_session_unclean);
        assert!(status.created);
        assert!(path.exists());
    }

    #[test]
    fn second_session_without_clean_shutdown_detects_unclean() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.lock");
        let _ = record_at(&path);
        // Simulated crash: no clean_shutdown_at call.
        let status = record_at(&path);
        assert!(status.previous_session_unclean);
        assert!(status.created);
    }

    #[test]
    fn clean_shutdown_removes_marker() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.lock");
        let _ = record_at(&path);
        clean_shutdown_at(&path);
        assert!(!path.exists());
    }

    #[test]
    fn clean_shutdown_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.lock");
        clean_shutdown_at(&path); // file doesn't exist; must not panic
        clean_shutdown_at(&path);
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test --lib telemetry::session_marker
```
Expected: 4 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/telemetry/session_marker.rs
git commit -m "feat(telemetry): add session marker for crash detection"
```

---

### Task 0.7: Implement `consent.rs` with TDD

**Files:**
- Create: `src/telemetry/consent.rs`

Spec reference: §7 "Resolution order for `enabled`".

- [ ] **Step 1: Write the failing tests**

Create `src/telemetry/consent.rs`:

```rust
//! Telemetry enable/disable resolution.
//!
//! See spec §7 "Resolution order for `enabled`". Three tiers:
//! hard-off (DNT, --no-telemetry, AL_CH_TELEMETRY=0),
//! hard-on (AL_CH_TELEMETRY=1, init-option, config),
//! defaults (off in debug/test/CI; on in release for interactive use).

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisabledReason {
    DoNotTrack,
    CliFlag,
    EnvOff,
    DebugBuild,
    CfgTest,
    CiEnvironment,
    ConfigOff,
    InitOptionOff,
    NoConnectionString,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Enabled,
    Disabled(DisabledReason),
}

/// Inputs to the consent decision. Centralized to keep tests pure.
#[derive(Debug, Default, Clone)]
pub struct Inputs {
    /// `--no-telemetry` CLI flag.
    pub cli_no_telemetry: bool,
    /// LSP `initializationOptions.telemetry.enabled`, if provided.
    pub init_option: Option<bool>,
    /// `config.json` `telemetry.enabled`, if provided.
    pub config: Option<bool>,
    /// All environment variables, snapshotted (testable).
    pub env: HashMap<String, String>,
    /// True for `cfg(debug_assertions)` builds.
    pub is_debug: bool,
    /// True for `cfg(test)` builds.
    pub is_test: bool,
}

const CI_ENV_VARS: &[&str] = &[
    "CI",
    "GITHUB_ACTIONS",
    "GITLAB_CI",
    "BUILDKITE",
    "CIRCLECI",
    "TRAVIS",
    "JENKINS_URL",
    "TEAMCITY_VERSION",
    "TF_BUILD",
];

pub fn decide(inputs: &Inputs) -> Decision {
    // Hard-off tier
    if inputs.env.get("DO_NOT_TRACK").map(|s| s.as_str()) == Some("1") {
        return Decision::Disabled(DisabledReason::DoNotTrack);
    }
    if inputs.cli_no_telemetry {
        return Decision::Disabled(DisabledReason::CliFlag);
    }
    if inputs.env.get("AL_CH_TELEMETRY").map(|s| s.as_str()) == Some("0") {
        return Decision::Disabled(DisabledReason::EnvOff);
    }

    // Hard-on tier
    if inputs.env.get("AL_CH_TELEMETRY").map(|s| s.as_str()) == Some("1") {
        return Decision::Enabled;
    }
    if let Some(true) = inputs.init_option {
        return Decision::Enabled;
    }
    if let Some(false) = inputs.init_option {
        return Decision::Disabled(DisabledReason::InitOptionOff);
    }
    if let Some(true) = inputs.config {
        return Decision::Enabled;
    }
    if let Some(false) = inputs.config {
        return Decision::Disabled(DisabledReason::ConfigOff);
    }

    // Default heuristics
    if inputs.is_test {
        return Decision::Disabled(DisabledReason::CfgTest);
    }
    if inputs.is_debug {
        return Decision::Disabled(DisabledReason::DebugBuild);
    }
    for var in CI_ENV_VARS {
        if inputs.env.contains_key(*var) {
            return Decision::Disabled(DisabledReason::CiEnvironment);
        }
    }

    Decision::Enabled
}

/// Snapshot the current process environment.
pub fn live_env() -> HashMap<String, String> {
    std::env::vars().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> Inputs {
        Inputs::default()
    }

    fn with_env(pairs: &[(&str, &str)]) -> Inputs {
        let mut i = empty();
        i.env = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        i
    }

    #[test]
    fn defaults_to_enabled_in_release_interactive() {
        // is_debug=false, is_test=false, no env, no config.
        let i = empty();
        assert_eq!(decide(&i), Decision::Enabled);
    }

    #[test]
    fn do_not_track_disables() {
        let i = with_env(&[("DO_NOT_TRACK", "1")]);
        assert_eq!(decide(&i), Decision::Disabled(DisabledReason::DoNotTrack));
    }

    #[test]
    fn cli_flag_disables() {
        let mut i = empty();
        i.cli_no_telemetry = true;
        assert_eq!(decide(&i), Decision::Disabled(DisabledReason::CliFlag));
    }

    #[test]
    fn env_zero_disables() {
        let i = with_env(&[("AL_CH_TELEMETRY", "0")]);
        assert_eq!(decide(&i), Decision::Disabled(DisabledReason::EnvOff));
    }

    #[test]
    fn env_one_overrides_ci_default() {
        let i = with_env(&[("CI", "true"), ("AL_CH_TELEMETRY", "1")]);
        assert_eq!(decide(&i), Decision::Enabled);
    }

    #[test]
    fn ci_env_disables_by_default() {
        let i = with_env(&[("CI", "true")]);
        assert_eq!(decide(&i), Decision::Disabled(DisabledReason::CiEnvironment));
    }

    #[test]
    fn github_actions_disables_by_default() {
        let i = with_env(&[("GITHUB_ACTIONS", "true")]);
        assert_eq!(decide(&i), Decision::Disabled(DisabledReason::CiEnvironment));
    }

    #[test]
    fn debug_build_disables_by_default() {
        let mut i = empty();
        i.is_debug = true;
        assert_eq!(decide(&i), Decision::Disabled(DisabledReason::DebugBuild));
    }

    #[test]
    fn cfg_test_disables_by_default() {
        let mut i = empty();
        i.is_test = true;
        assert_eq!(decide(&i), Decision::Disabled(DisabledReason::CfgTest));
    }

    #[test]
    fn dnt_beats_explicit_on() {
        let i = with_env(&[("DO_NOT_TRACK", "1"), ("AL_CH_TELEMETRY", "1")]);
        assert_eq!(decide(&i), Decision::Disabled(DisabledReason::DoNotTrack));
    }

    #[test]
    fn cli_flag_beats_config_on() {
        let mut i = empty();
        i.cli_no_telemetry = true;
        i.config = Some(true);
        assert_eq!(decide(&i), Decision::Disabled(DisabledReason::CliFlag));
    }

    #[test]
    fn init_option_overrides_config() {
        let mut i = empty();
        i.config = Some(true);
        i.init_option = Some(false);
        assert_eq!(decide(&i), Decision::Disabled(DisabledReason::InitOptionOff));
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test --lib telemetry::consent
```
Expected: 12 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/telemetry/consent.rs
git commit -m "feat(telemetry): add consent module with tiered resolution order"
```

---

### Task 0.8: Implement `events.rs` (struct definitions only, no exporter mapping yet)

**Files:**
- Create: `src/telemetry/events.rs`

Spec reference: §5 "Data Model".

- [ ] **Step 1: Write the failing tests**

Create `src/telemetry/events.rs`:

```rust
//! Event structs and the canonical leaf-kind enumeration.
//!
//! See spec §5 "Data Model". 6 outer `EventKind` variants encode 14 leaf
//! event types plus a session summary. `ALL_LEAF_KINDS` is the single source
//! of truth for the count — array sizes derive from `ALL_LEAF_KINDS.len()`.

use std::time::SystemTime;

pub const SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LeafKind {
    ResolutionObjectNotFound,
    ResolutionProcedureNotFound,
    ResolutionUnresolvedUnqualified,
    ResolutionAmbiguous,
    ResolutionUnsupportedConstruct,
    ParserTreeError,
    ParserParseFailed,
    ParserUnknownNodeKind,
    HandlerEmpty,
    IndexerMissingDependency,
    IndexerAppParseFailed,
    IndexerBrokenSymlink,
    IndexerIoError,
    SessionStart,
}

pub const ALL_LEAF_KINDS: [LeafKind; 14] = [
    LeafKind::ResolutionObjectNotFound,
    LeafKind::ResolutionProcedureNotFound,
    LeafKind::ResolutionUnresolvedUnqualified,
    LeafKind::ResolutionAmbiguous,
    LeafKind::ResolutionUnsupportedConstruct,
    LeafKind::ParserTreeError,
    LeafKind::ParserParseFailed,
    LeafKind::ParserUnknownNodeKind,
    LeafKind::HandlerEmpty,
    LeafKind::IndexerMissingDependency,
    LeafKind::IndexerAppParseFailed,
    LeafKind::IndexerBrokenSymlink,
    LeafKind::IndexerIoError,
    LeafKind::SessionStart,
];

impl LeafKind {
    pub fn index(self) -> usize {
        ALL_LEAF_KINDS
            .iter()
            .position(|&k| k == self)
            .expect("LeafKind must be in ALL_LEAF_KINDS")
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ResolutionObjectNotFound => "resolution.object_not_found",
            Self::ResolutionProcedureNotFound => "resolution.procedure_not_found",
            Self::ResolutionUnresolvedUnqualified => "resolution.unresolved_unqualified",
            Self::ResolutionAmbiguous => "resolution.ambiguous",
            Self::ResolutionUnsupportedConstruct => "resolution.unsupported_construct",
            Self::ParserTreeError => "parser.tree_error",
            Self::ParserParseFailed => "parser.parse_failed",
            Self::ParserUnknownNodeKind => "parser.unknown_node_kind",
            Self::HandlerEmpty => "handler.empty_result",
            Self::IndexerMissingDependency => "indexer.missing_dependency",
            Self::IndexerAppParseFailed => "indexer.app_parse_failed",
            Self::IndexerBrokenSymlink => "indexer.broken_symlink",
            Self::IndexerIoError => "indexer.io_error",
            Self::SessionStart => "session.start",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionFailure {
    ObjectNotFound,
    ProcedureNotFound,
    UnresolvedUnqualified,
    Ambiguous,
    UnsupportedConstruct,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallPattern {
    Qualified,
    Unqualified,
    MemberChain { depth: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectType {
    Codeunit,
    Table,
    Page,
    Report,
    Query,
    XmlPort,
    Enum,
    Interface,
    PageExtension,
    TableExtension,
    EnumExtension,
    ControlAddIn,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalleeSource {
    Workspace,
    AppDependency,
    System,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerContext {
    Procedure,
    Trigger,
    EventSubscriber,
    Layout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeBucket {
    Sub1k,      // < 1KB / < 100 files
    Sub10k,     // 1-10KB / 100-500 files
    Sub100k,    // 10-100KB / 500-2000 files
    Over100k,   // 100KB+ / 2000+ files
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParserErrorKind {
    TreeError,
    ParseFailed,
    UnknownNodeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexerIssueKind {
    MissingDependency,
    AppParseFailed,
    BrokenSymlink,
    IoError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefinitionKind {
    Procedure,
    Trigger,
    EventSubscriber,
}

#[derive(Debug, Clone)]
pub struct ConfigFlags {
    pub bits: u32,
}

#[derive(Debug, Clone)]
pub struct ResolutionMiss {
    pub failure: ResolutionFailure,
    pub call_pattern: CallPattern,
    pub callee_object_type: Option<ObjectType>,
    pub callee_source: CalleeSource,
    pub caller_object_type: ObjectType,
    pub caller_context: CallerContext,
    pub object_hash: Option<String>,
    pub procedure_hash: String,
    pub arg_count: u8,
    pub name_len_object: Option<u16>,
    pub name_len_procedure: u16,
    pub ts_node_path: String,
    pub repeat_count: u32,
}

#[derive(Debug, Clone)]
pub struct ParserError {
    pub kind: ParserErrorKind,
    pub node_kind_hash: Option<String>, // present for UnknownNodeKind
    pub file_hash: String,
    pub file_extension: String,
    pub file_size_bucket: SizeBucket,
    pub error_count: u32,
    pub repeat_count: u32,
}

#[derive(Debug, Clone)]
pub struct HandlerEmpty {
    pub method: &'static str,
    pub target_object_type: ObjectType,
    pub target_kind: DefinitionKind,
    pub object_hash: String,
    pub procedure_hash: String,
    pub repeat_count: u32,
}

#[derive(Debug, Clone)]
pub struct IndexerIssue {
    pub kind: IndexerIssueKind,
    pub app_id_hash: Option<String>,
    pub detail_code: u16,
}

#[derive(Debug, Clone)]
pub struct SessionStart {
    pub workspace_file_count: u32,
    pub al_file_count_bucket: SizeBucket,
    pub dependency_count: u8,
    pub has_app_dependencies: bool,
    pub config_flags: ConfigFlags,
    pub previous_session_unclean: bool,
}

#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub duration_secs: u64,
    pub unique_patterns: u32,
    pub queue_full_drops: u32,
    pub dedup_suppressed: u32,
    pub export_attempts: u32,
    pub export_failures: u32,
    pub observed_by_kind: [u32; 14],
    pub exported_by_kind: [u32; 14],
}

#[derive(Debug, Clone)]
pub enum EventKind {
    ResolutionMiss(ResolutionMiss),
    ParserError(ParserError),
    HandlerEmpty(HandlerEmpty),
    IndexerIssue(IndexerIssue),
    SessionStart(SessionStart),
    SessionSummary(SessionSummary),
}

impl EventKind {
    /// The leaf kind for counter indexing. `SessionSummary` is meta and
    /// returns `None` (it's not self-counted).
    pub fn leaf(&self) -> Option<LeafKind> {
        match self {
            Self::ResolutionMiss(m) => Some(match m.failure {
                ResolutionFailure::ObjectNotFound => LeafKind::ResolutionObjectNotFound,
                ResolutionFailure::ProcedureNotFound => LeafKind::ResolutionProcedureNotFound,
                ResolutionFailure::UnresolvedUnqualified => LeafKind::ResolutionUnresolvedUnqualified,
                ResolutionFailure::Ambiguous => LeafKind::ResolutionAmbiguous,
                ResolutionFailure::UnsupportedConstruct => LeafKind::ResolutionUnsupportedConstruct,
            }),
            Self::ParserError(e) => Some(match e.kind {
                ParserErrorKind::TreeError => LeafKind::ParserTreeError,
                ParserErrorKind::ParseFailed => LeafKind::ParserParseFailed,
                ParserErrorKind::UnknownNodeKind => LeafKind::ParserUnknownNodeKind,
            }),
            Self::HandlerEmpty(_) => Some(LeafKind::HandlerEmpty),
            Self::IndexerIssue(i) => Some(match i.kind {
                IndexerIssueKind::MissingDependency => LeafKind::IndexerMissingDependency,
                IndexerIssueKind::AppParseFailed => LeafKind::IndexerAppParseFailed,
                IndexerIssueKind::BrokenSymlink => LeafKind::IndexerBrokenSymlink,
                IndexerIssueKind::IoError => LeafKind::IndexerIoError,
            }),
            Self::SessionStart(_) => Some(LeafKind::SessionStart),
            Self::SessionSummary(_) => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EventEnvelope {
    pub schema_version: u8,
    pub timestamp: SystemTime,
    pub install_id: String,
    pub al_version: &'static str,
    pub grammar_version: &'static str,
    pub os: &'static str,
    pub session_id: u64,
    pub workspace_id: String,
    pub event: EventKind,
}

pub fn current_os() -> &'static str {
    match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "macos",
        "linux" => "linux",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_leaf_kinds_has_14_entries() {
        assert_eq!(ALL_LEAF_KINDS.len(), 14);
    }

    #[test]
    fn each_leaf_kind_has_unique_index() {
        for (i, k) in ALL_LEAF_KINDS.iter().enumerate() {
            assert_eq!(k.index(), i);
        }
    }

    #[test]
    fn each_leaf_kind_has_unique_string() {
        let mut seen = std::collections::HashSet::new();
        for k in ALL_LEAF_KINDS {
            assert!(seen.insert(k.as_str()), "duplicate label: {}", k.as_str());
        }
    }

    #[test]
    fn session_summary_has_no_leaf() {
        let s = SessionSummary {
            duration_secs: 0,
            unique_patterns: 0,
            queue_full_drops: 0,
            dedup_suppressed: 0,
            export_attempts: 0,
            export_failures: 0,
            observed_by_kind: [0; 14],
            exported_by_kind: [0; 14],
        };
        assert_eq!(EventKind::SessionSummary(s).leaf(), None);
    }
}
```

- [ ] **Step 2: Register module in `mod.rs`**

`mod events;` was already added in Task 0.2. No change.

- [ ] **Step 3: Run tests**

```bash
cargo test --lib telemetry::events
```
Expected: 4 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/telemetry/events.rs
git commit -m "feat(telemetry): add event structs and ALL_LEAF_KINDS source of truth"
```

---

### Task 0.9: Extend `config.rs` with `telemetry` block

**Files:**
- Modify: `src/config.rs`

Spec reference: §7 "Config file".

- [ ] **Step 1: Read existing config.rs structure**

Already read in plan setup. The file uses `ConfigFile { diagnostics: DiagnosticsSection }`. We add a sibling `telemetry` field that is `Option<TelemetrySection>`.

- [ ] **Step 2: Write the failing test**

In `src/config.rs` `#[cfg(test)] mod tests`, add:

```rust
#[test]
fn test_load_telemetry_section() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join(".al-call-hierarchy.json"),
        r#"{
            "telemetry": {
                "enabled": false,
                "connectionString": "InstrumentationKey=foo;IngestionEndpoint=https://x.azure.com/",
                "flushIntervalSecs": 7,
                "batchSize": 256,
                "queueCapacity": 1024,
                "dedupTtlSecs": 600,
                "handlerEmptySampleRate": 20
            }
        }"#,
    )
    .unwrap();
    // Telemetry config is loaded via a separate fn; existing DiagnosticConfig::load
    // doesn't touch it.
    let tcfg = TelemetryFileConfig::load_at(&dir.path().join(".al-call-hierarchy.json"));
    assert_eq!(tcfg.enabled, Some(false));
    assert_eq!(
        tcfg.connection_string.as_deref(),
        Some("InstrumentationKey=foo;IngestionEndpoint=https://x.azure.com/")
    );
    assert_eq!(tcfg.flush_interval_secs, Some(7));
    assert_eq!(tcfg.batch_size, Some(256));
    assert_eq!(tcfg.queue_capacity, Some(1024));
    assert_eq!(tcfg.dedup_ttl_secs, Some(600));
    assert_eq!(tcfg.handler_empty_sample_rate, Some(20));
}

#[test]
fn test_load_telemetry_missing_section_yields_empty() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join(".al-call-hierarchy.json"),
        r#"{ "diagnostics": {} }"#,
    )
    .unwrap();
    let tcfg = TelemetryFileConfig::load_at(&dir.path().join(".al-call-hierarchy.json"));
    assert!(tcfg.enabled.is_none());
    assert!(tcfg.connection_string.is_none());
}
```

- [ ] **Step 3: Add the telemetry config types to `src/config.rs`**

Add after the existing types (before `#[cfg(test)]`):

```rust
/// Telemetry section of `~/.al-call-hierarchy/config.json`. All fields optional;
/// the telemetry subsystem applies its own defaults from the spec.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TelemetryFileConfig {
    pub enabled: Option<bool>,
    pub connection_string: Option<String>,
    pub flush_interval_secs: Option<u64>,
    pub batch_size: Option<u32>,
    pub queue_capacity: Option<u32>,
    pub dedup_ttl_secs: Option<u64>,
    pub handler_empty_sample_rate: Option<u32>,
}

impl TelemetryFileConfig {
    /// Load from a config file path. Returns an empty config if missing/invalid.
    pub fn load_at(path: &Path) -> Self {
        // Reuse existing parsing helper. We need a wrapper struct because
        // `ConfigFile` only carries `diagnostics`.
        #[derive(Deserialize, Default)]
        struct Wrapper {
            #[serde(default)]
            telemetry: TelemetryFileConfig,
        }

        if !path.exists() {
            return Self::default();
        }
        let contents = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return Self::default(),
        };
        match serde_json::from_str::<Wrapper>(&contents) {
            Ok(w) => w.telemetry,
            Err(_) => Self::default(),
        }
    }

    /// Merge global + workspace files. Workspace overlays global per-field.
    pub fn load_merged(workspace_root: &Path) -> Self {
        let global = global_config_path()
            .map(|p| Self::load_at(&p))
            .unwrap_or_default();
        let workspace = Self::load_at(&workspace_root.join(".al-call-hierarchy.json"));
        Self::merge(global, workspace)
    }

    fn merge(base: Self, overlay: Self) -> Self {
        Self {
            enabled: overlay.enabled.or(base.enabled),
            connection_string: overlay.connection_string.or(base.connection_string),
            flush_interval_secs: overlay.flush_interval_secs.or(base.flush_interval_secs),
            batch_size: overlay.batch_size.or(base.batch_size),
            queue_capacity: overlay.queue_capacity.or(base.queue_capacity),
            dedup_ttl_secs: overlay.dedup_ttl_secs.or(base.dedup_ttl_secs),
            handler_empty_sample_rate: overlay
                .handler_empty_sample_rate
                .or(base.handler_empty_sample_rate),
        }
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test --lib config
```
Expected: existing tests pass + 2 new tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "feat(telemetry): extend config.rs with telemetry section parser"
```

---

### Task 0.10: Add `--no-telemetry` CLI flag and Inputs aggregation

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Add the CLI flag**

Edit `src/main.rs`. In the `Args` struct (around line 32-56), add a new field:

```rust
    /// Disable anonymous failure-diagnostics telemetry for this run.
    /// (Telemetry is also off by default in dev/CI builds.)
    #[arg(long)]
    no_telemetry: bool,
```

- [ ] **Step 2: Verify the binary parses the new flag**

```bash
cargo build
target/debug/al-call-hierarchy --help | grep -i telemetry
```
Expected: `--no-telemetry` appears in the help output.

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "feat(telemetry): add --no-telemetry CLI flag"
```

---

### Task 0.11: Wire `init`/`shutdown` into server.rs (still no-op)

**Files:**
- Modify: `src/server.rs`
- Modify: `src/main.rs` (pass `args.no_telemetry` into `run_server`)

Spec reference: §4 "Call sites".

- [ ] **Step 1: Update `run_server` signature in server.rs**

Find `pub fn run_server(no_watcher: bool) -> Result<()>` and change to:

```rust
pub fn run_server(no_watcher: bool, no_telemetry: bool) -> Result<()> {
    info!("Starting AL Call Hierarchy LSP server");

    let telemetry_handle = crate::telemetry::init();
    // Telemetry handle stays alive for the duration of the server.
    // We pass it to shutdown at the bottom of this function.
    let _ = no_telemetry; // Phase 1 wires this through consent::Inputs.

    let (connection, io_threads) = Connection::stdio();
    // ... rest unchanged ...
```

At the **end** of `run_server`, before the final `Ok(())`, add:

```rust
    crate::telemetry::shutdown(telemetry_handle);
    Ok(())
}
```

(Find the existing `Ok(())` at the end of the function and replace it with the two lines above.)

- [ ] **Step 2: Update the call site in main.rs**

Find `run_server(args.no_watcher)?;` (around line 100) and change to:

```rust
        run_server(args.no_watcher, args.no_telemetry)?;
```

- [ ] **Step 3: Verify both feature configs build**

```bash
cargo build
cargo build --no-default-features
```
Expected: both succeed.

- [ ] **Step 4: Verify the LSP starts and exits cleanly**

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}' | target/debug/al-call-hierarchy
```
Expected: server emits an initialize response, then exits when stdin closes. No telemetry-related output (init is still no-op).

- [ ] **Step 5: Commit**

```bash
git add src/server.rs src/main.rs
git commit -m "feat(telemetry): wire init/shutdown into LSP server lifecycle"
```

---

### Task 0.12: Privacy lint test

**Files:**
- Create: `tests/telemetry_privacy_lint.rs`

Spec reference: §10 "Privacy lint".

- [ ] **Step 1: Write the test**

Create `tests/telemetry_privacy_lint.rs`:

```rust
//! Privacy lint: `src/telemetry/events.rs` must not introduce raw-string fields
//! that could carry unhashed AL identifiers. Allowed `String` fields are those
//! whose names end in `_hash`, plus a small allowlist for non-identifier strings
//! (file_extension, ts_node_path, method, install_id, workspace_id).
//!
//! This is a hard CI gate. If you legitimately need a new `String` field, add
//! it to ALLOWED_FIELDS below with a justification.

use std::fs;

const ALLOWED_FIELDS: &[&str] = &[
    "file_extension",   // "al" / "dal"; closed enum encoded as String
    "ts_node_path",     // tree-sitter grammar shape; public
    "method",           // &'static str, but match is permissive
    "install_id",       // hashed in production; allowed
    "workspace_id",     // hashed; allowed
    "al_version",       // &'static str
    "grammar_version",  // &'static str
    "os",               // &'static str
];

#[test]
fn no_unhashed_string_fields_in_events_module() {
    let src = fs::read_to_string("src/telemetry/events.rs")
        .expect("events.rs must exist for privacy lint");

    // Collect every line that declares `pub xxx: String` or `pub xxx: Option<String>`.
    let mut violations = Vec::new();
    for (lineno, line) in src.lines().enumerate() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("pub ") else { continue };
        let Some(colon_pos) = rest.find(':') else { continue };
        let field_name = rest[..colon_pos].trim();
        let type_part = rest[colon_pos + 1..].trim();

        let is_string = type_part.starts_with("String")
            || type_part.starts_with("Option<String>")
            || type_part.starts_with("Option<&");

        if !is_string {
            continue;
        }
        // Allowed if the name ends with `_hash` or is in the allowlist.
        if field_name.ends_with("_hash") {
            continue;
        }
        if ALLOWED_FIELDS.contains(&field_name) {
            continue;
        }
        violations.push(format!("L{}: field `{}` of type {}", lineno + 1, field_name, type_part));
    }

    assert!(
        violations.is_empty(),
        "Privacy lint violations in src/telemetry/events.rs (raw String fields that may leak identifiers):\n{}\n\n\
        If a new field is intentionally a non-identifier String, add it to ALLOWED_FIELDS in this test with a justification.",
        violations.join("\n")
    );
}
```

- [ ] **Step 2: Run the test**

```bash
cargo test --test telemetry_privacy_lint
```
Expected: passes (existing fields all match the rule).

- [ ] **Step 3: Commit**

```bash
git add tests/telemetry_privacy_lint.rs
git commit -m "test(telemetry): add privacy lint for raw String fields in events"
```

---

### Task 0.13: Phase 0 wrap — verify --no-default-features still produces a working LSP

**Files:** none modified (verification only).

- [ ] **Step 1: Full build matrix**

```bash
cargo build
cargo build --release
cargo build --no-default-features
cargo build --release --no-default-features
```
Expected: all four succeed.

- [ ] **Step 2: Run all tests**

```bash
cargo test
cargo test --no-default-features
```
Expected: all pass.

- [ ] **Step 3: Run clippy with strict lints scoped to telemetry**

```bash
cargo clippy --all-targets --all-features -- -D warnings
```
Expected: no warnings.

- [ ] **Step 4: Tag end-of-phase commit**

```bash
git log --oneline -10
git tag -a phase-0-foundation -m "Telemetry Phase 0 foundation complete (skeleton + utilities)"
```

Phase 0 is now mergeable. The PR description should call out that all `record_*` calls are still no-ops and no telemetry is collected — Phase 1 wires the pipeline.

---

## Phase 0.5 — App Insights ingestion spike (gate)

**Phase goal:** Validate end-to-end that `opentelemetry-application-insights` ships events to a real Azure Application Insights resource with attributes intact and no backend sampling on summary events. This phase is a **blocking gate** before Phase 1 — its outcome determines whether Phase 1 proceeds with the planned exporter or falls back (direct breeze HTTP, OTel collector sidecar, or hand-rolled `ureq` client).

### Task 0.5.1: Create spike binary skeleton

**Files:**
- Create: `tools/telemetry-spike/main.rs`
- Modify: `Cargo.toml` (add `[[bin]]` entry)

- [ ] **Step 1: Add the binary entry to Cargo.toml**

Append to `Cargo.toml`:

```toml
[[bin]]
name = "telemetry-spike"
path = "tools/telemetry-spike/main.rs"
required-features = ["telemetry"]
```

- [ ] **Step 2: Write the spike binary**

Create `tools/telemetry-spike/main.rs`:

```rust
//! Phase 0.5 spike: send a handful of synthetic telemetry events to Azure
//! Application Insights and verify they arrive intact.
//!
//! Run with:
//!   AL_CH_SPIKE_CONNECTION_STRING="InstrumentationKey=...;IngestionEndpoint=..." \
//!     cargo run --bin telemetry-spike --release

use std::env;
use std::time::Duration;

fn main() {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .init();

    let cs = env::var("AL_CH_SPIKE_CONNECTION_STRING")
        .expect("set AL_CH_SPIKE_CONNECTION_STRING to run the spike");

    log::info!("Spike: initializing exporter against connection string");
    spike::run(&cs);
}

mod spike {
    use opentelemetry::{
        global,
        trace::{Tracer, TracerProvider as _},
        KeyValue,
    };
    use opentelemetry_application_insights::new_pipeline_from_connection_string;
    use std::time::Duration;

    pub fn run(connection_string: &str) {
        let tracer_provider = new_pipeline_from_connection_string(connection_string)
            .expect("valid connection string")
            .with_client(reqwest::blocking::Client::new())
            .build_simple();

        global::set_tracer_provider(tracer_provider.clone());
        let tracer = tracer_provider.tracer("al-call-hierarchy-spike");

        // 1. Synthetic resolution-miss event
        let mut span = tracer.start("resolution.procedure_not_found");
        span.set_attribute(KeyValue::new("telemetry.alch.failure", "ProcedureNotFound"));
        span.set_attribute(KeyValue::new("telemetry.alch.callee_object_type", "Codeunit"));
        span.set_attribute(KeyValue::new("telemetry.alch.callee_source", "AppDependency"));
        span.set_attribute(KeyValue::new(
            "telemetry.alch.object_hash",
            "deadbeefcafef00d1234567890abcdef",
        ));
        span.set_attribute(KeyValue::new(
            "telemetry.alch.procedure_hash",
            "0123456789abcdefdeadbeefcafef00d",
        ));
        span.set_attribute(KeyValue::new("telemetry.alch.arg_count", 2_i64));
        span.set_attribute(KeyValue::new("telemetry.alch.schema_version", 1_i64));
        drop(span);

        // 2. Synthetic session.summary (large attribute set)
        let mut sp = tracer.start("session.summary");
        sp.set_attribute(KeyValue::new("telemetry.alch.duration_secs", 600_i64));
        sp.set_attribute(KeyValue::new("telemetry.alch.queue_full_drops", 0_i64));
        sp.set_attribute(KeyValue::new("telemetry.alch.dedup_suppressed", 47_i64));
        sp.set_attribute(KeyValue::new("telemetry.alch.export_attempts", 12_i64));
        sp.set_attribute(KeyValue::new("telemetry.alch.export_failures", 0_i64));
        for i in 0..14 {
            sp.set_attribute(KeyValue::new(
                format!("telemetry.alch.observed.{}", i),
                (i * 7) as i64,
            ));
            sp.set_attribute(KeyValue::new(
                format!("telemetry.alch.exported.{}", i),
                (i * 5) as i64,
            ));
        }
        drop(sp);

        // 3. Burst of 100 summary events to test backend sampling
        for i in 0..100 {
            let mut s = tracer.start("session.summary.burst");
            s.set_attribute(KeyValue::new("burst_seq", i as i64));
            drop(s);
        }

        log::info!("Spike: spans emitted, flushing");
        tracer_provider
            .force_flush()
            .iter()
            .for_each(|r| {
                if let Err(e) = r {
                    log::error!("flush error: {:?}", e);
                }
            });
        std::thread::sleep(Duration::from_secs(2));
        log::info!("Spike: done. Check App Insights for arrived events.");
    }
}
```

- [ ] **Step 3: Add spike-only deps**

In `Cargo.toml` `[dev-dependencies]`, add:

```toml
opentelemetry = "0.27"
opentelemetry_sdk = "0.27"
opentelemetry-application-insights = { version = "0.36", features = ["reqwest-blocking-client"] }
reqwest = { version = "0.12", features = ["blocking", "rustls-tls"], default-features = false }
```

(Promote to `[dependencies]` in Phase 1 once the spike validates the choice.)

- [ ] **Step 4: Verify it compiles**

```bash
cargo build --bin telemetry-spike
```
Expected: succeeds.

- [ ] **Step 5: Commit**

```bash
git add tools/telemetry-spike/main.rs Cargo.toml Cargo.lock
git commit -m "spike(telemetry): add Phase 0.5 ingestion validation binary"
```

---

### Task 0.5.2: Run spike against real Azure App Insights (manual)

**Files:** none modified.

- [ ] **Step 1: Provision App Insights resource**

In Azure Portal, create a new Application Insights resource (workspace-based). Copy the **Connection String** (not the legacy instrumentation key) from the resource's overview page.

- [ ] **Step 2: Run the spike binary**

```bash
export AL_CH_SPIKE_CONNECTION_STRING="InstrumentationKey=...;IngestionEndpoint=https://...applicationinsights.azure.com/;LiveEndpoint=..."
cargo run --bin telemetry-spike --release
```
Expected output: `Spike: done. Check App Insights for arrived events.` within ~30 seconds.

- [ ] **Step 3: Verify in Azure Portal**

In the App Insights resource → Logs (Kusto):

```kusto
traces
| where timestamp > ago(10m)
| where customDimensions["telemetry.alch.failure"] == "ProcedureNotFound"
| project timestamp, message, customDimensions
```

Expected: 1 row with the synthetic resolution-miss event. All `telemetry.alch.*` attributes present in `customDimensions`.

```kusto
traces
| where timestamp > ago(10m)
| where message == "session.summary"
| project timestamp, message, customDimensions
```

Expected: 1 row, all 14 `observed.*` and 14 `exported.*` attributes preserved.

- [ ] **Step 4: Verify backend sampling does not drop summary burst**

```kusto
traces
| where timestamp > ago(10m)
| where message == "session.summary.burst"
| count
```

Expected: 100. If the count is lower, App Insights ingestion sampling is enabled — note this in spike findings (Step 5). To pin summary events at 100% sampling, configure ingestion-side sampling rules in the Azure resource.

---

### Task 0.5.3: Document spike findings and decision

**Files:**
- Create: `docs/telemetry-ingestion-spike.md`

- [ ] **Step 1: Write the findings doc**

Create `docs/telemetry-ingestion-spike.md`:

```markdown
# Telemetry Ingestion Spike — Findings

**Date:** <fill in run date>
**Spec:** docs/superpowers/specs/2026-05-06-telemetry-design.md
**Status:** <pass | fail | partial>

## Setup

- Azure Application Insights resource: <region, workspace>
- Connection string format: `InstrumentationKey=...;IngestionEndpoint=...;LiveEndpoint=...`
- Spike binary: `tools/telemetry-spike/main.rs`
- Crate under test: `opentelemetry-application-insights` <version>

## Test 1: synthetic resolution-miss event

- Event arrived: <yes/no>
- All custom dimensions present: <yes/no>
- Latency to Kusto: <minutes>
- Issues: <none | description>

## Test 2: session.summary with full attribute set

- Event arrived: <yes/no>
- All 14 `observed.*` attributes preserved: <yes/no>
- All 14 `exported.*` attributes preserved: <yes/no>
- Issues: <none | description>

## Test 3: backend sampling on summary burst

- Burst sent: 100 events
- Burst received: <count>
- Sampling rate inferred: <0% drops | <pct>>
- Mitigation if sampling active: <ingestion-rule, exporter-side filter, etc.>

## Decision

**<Proceed | Fall back to plan B | Block phase 1>**

If proceeding: continue with `opentelemetry-application-insights` exporter as designed in spec §3, §4, §13.

If falling back: chosen alternative is <Direct ureq breeze client | OTel collector sidecar>. Update spec §13 and Phase 1 Task 1.1 to reflect the new dependency set.

## Open questions

- <list any unresolved issues>
```

- [ ] **Step 2: Fill in the doc with real findings from Task 0.5.2**

Replace each `<...>` with actual results from the spike runs.

- [ ] **Step 3: Commit**

```bash
git add docs/telemetry-ingestion-spike.md
git commit -m "spike(telemetry): document Phase 0.5 ingestion findings"
```

- [ ] **Step 4: Decision gate**

If the spike passes (events arrive intact, no surprise sampling on summaries): proceed to Phase 1 as planned.

If the spike fails or partially passes: revise Phase 1 Task 1.1 dependency list per the spike doc's chosen fallback. Do not proceed to Phase 1 with an unverified exporter choice.

```bash
git tag -a phase-0.5-spike -m "Telemetry ingestion spike complete; Phase 1 unblocked"
```

---

## Phase 1 — Pipeline (PR #2)

**Phase goal:** Implement the full event pipeline: counters, dedup, mpsc channel, background-thread exporter, post-drain summary, and the `telemetryStatus` LSP request. End-to-end tests against a mock exporter pass. Hot-path benchmark passes the 5µs budget. `record_*` functions still have empty bodies — Phase 2 fills them in.

### Task 1.1: Promote spike deps to runtime deps

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Move OTel deps from `[dev-dependencies]` to `[dependencies]`**

In `Cargo.toml`, ensure `[dependencies]` includes (gated on the `telemetry` feature where appropriate via `optional = true`):

```toml
# Telemetry pipeline (only pulled when "telemetry" feature is on)
opentelemetry = { version = "0.27", optional = true }
opentelemetry_sdk = { version = "0.27", features = ["rt-tokio-current-thread"], optional = true }
opentelemetry-application-insights = { version = "0.36", features = ["reqwest-blocking-client"], optional = true }
tokio = { version = "1", features = ["rt", "macros", "time", "sync"], optional = true }
tracing = { version = "0.1", optional = true }
tracing-opentelemetry = { version = "0.28", optional = true }
lru = { version = "0.12", optional = true }
reqwest = { version = "0.12", features = ["blocking", "rustls-tls"], default-features = false, optional = true }
```

And update the feature flag:

```toml
[features]
default = ["telemetry"]
telemetry = [
    "dep:opentelemetry",
    "dep:opentelemetry_sdk",
    "dep:opentelemetry-application-insights",
    "dep:tokio",
    "dep:tracing",
    "dep:tracing-opentelemetry",
    "dep:lru",
    "dep:reqwest",
]
```

- [ ] **Step 2: Verify both feature configs build**

```bash
cargo build
cargo build --no-default-features
```
Expected: both succeed. `--no-default-features` should not download the OTel/tokio dep tree.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "feat(telemetry): promote OTel/tokio deps from spike to feature-gated runtime"
```

---

### Task 1.2: Implement `counters.rs` (atomic counter arrays)

**Files:**
- Create: `src/telemetry/counters.rs`

Spec reference: §6 "Counter dimensions".

- [ ] **Step 1: Write the failing test**

Create `src/telemetry/counters.rs`:

```rust
//! Atomic counters shared between producer threads (LSP request handlers)
//! and the background telemetry thread. See spec §6 "Counter dimensions".

use crate::telemetry::events::{LeafKind, ALL_LEAF_KINDS};
use std::sync::atomic::{AtomicU32, Ordering};

pub struct Counters {
    pub observed: [AtomicU32; 14],
    pub exported: [AtomicU32; 14],
    pub dedup_suppressed: AtomicU32,
    pub queue_full_drops: AtomicU32,
    pub export_attempts: AtomicU32,
    pub export_failures: AtomicU32,
}

impl Counters {
    pub const fn new() -> Self {
        // const-init is verbose pre-Rust-1.79; use a helper.
        const fn zero_array() -> [AtomicU32; 14] {
            [
                AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0),
                AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0),
                AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0),
                AtomicU32::new(0), AtomicU32::new(0),
            ]
        }
        Self {
            observed: zero_array(),
            exported: zero_array(),
            dedup_suppressed: AtomicU32::new(0),
            queue_full_drops: AtomicU32::new(0),
            export_attempts: AtomicU32::new(0),
            export_failures: AtomicU32::new(0),
        }
    }

    pub fn observe(&self, kind: LeafKind) {
        self.observed[kind.index()].fetch_add(1, Ordering::Relaxed);
    }

    pub fn export_succeeded(&self, kind: LeafKind) {
        self.exported[kind.index()].fetch_add(1, Ordering::Relaxed);
    }

    pub fn dedup_suppress(&self) {
        self.dedup_suppressed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn queue_full(&self) {
        self.queue_full_drops.fetch_add(1, Ordering::Relaxed);
    }

    pub fn export_attempted(&self) {
        self.export_attempts.fetch_add(1, Ordering::Relaxed);
    }

    pub fn export_failed(&self) {
        self.export_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> Snapshot {
        let read = |a: &[AtomicU32; 14]| -> [u32; 14] {
            let mut out = [0u32; 14];
            for (i, v) in a.iter().enumerate() {
                out[i] = v.load(Ordering::Relaxed);
            }
            out
        };
        Snapshot {
            observed_by_kind: read(&self.observed),
            exported_by_kind: read(&self.exported),
            dedup_suppressed: self.dedup_suppressed.load(Ordering::Relaxed),
            queue_full_drops: self.queue_full_drops.load(Ordering::Relaxed),
            export_attempts: self.export_attempts.load(Ordering::Relaxed),
            export_failures: self.export_failures.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub observed_by_kind: [u32; 14],
    pub exported_by_kind: [u32; 14],
    pub dedup_suppressed: u32,
    pub queue_full_drops: u32,
    pub export_attempts: u32,
    pub export_failures: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observe_increments_correct_slot() {
        let c = Counters::new();
        c.observe(LeafKind::ResolutionObjectNotFound);
        c.observe(LeafKind::ResolutionObjectNotFound);
        c.observe(LeafKind::ParserTreeError);
        let snap = c.snapshot();
        assert_eq!(snap.observed_by_kind[LeafKind::ResolutionObjectNotFound.index()], 2);
        assert_eq!(snap.observed_by_kind[LeafKind::ParserTreeError.index()], 1);
        assert_eq!(snap.observed_by_kind[LeafKind::HandlerEmpty.index()], 0);
    }

    #[test]
    fn pipeline_counters_independent_of_observed() {
        let c = Counters::new();
        c.queue_full();
        c.queue_full();
        c.dedup_suppress();
        let snap = c.snapshot();
        assert_eq!(snap.queue_full_drops, 2);
        assert_eq!(snap.dedup_suppressed, 1);
        assert_eq!(snap.observed_by_kind, [0u32; 14]);
    }

    #[test]
    fn snapshot_is_consistent_under_concurrent_writes() {
        use std::sync::Arc;
        use std::thread;

        let c = Arc::new(Counters::new());
        let mut handles = vec![];
        for _ in 0..16 {
            let c2 = c.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..1000 {
                    c2.observe(LeafKind::ResolutionProcedureNotFound);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let snap = c.snapshot();
        assert_eq!(
            snap.observed_by_kind[LeafKind::ResolutionProcedureNotFound.index()],
            16_000
        );
    }
}
```

- [ ] **Step 2: Register the module**

In `src/telemetry/mod.rs`, add under the existing `#[cfg(feature = "telemetry")]` declarations:

```rust
#[cfg(feature = "telemetry")]
pub mod counters;
```

- [ ] **Step 3: Run tests**

```bash
cargo test --lib telemetry::counters
```
Expected: 3 tests pass, including the 16-thread concurrency test.

- [ ] **Step 4: Commit**

```bash
git add src/telemetry/counters.rs src/telemetry/mod.rs
git commit -m "feat(telemetry): add atomic counter arrays for pipeline observability"
```

---

### Task 1.3: Implement `dedup.rs` with workspace-scoped LRU

**Files:**
- Create: `src/telemetry/dedup.rs`

Spec reference: §6 "Dedup".

- [ ] **Step 1: Write the failing tests**

Create `src/telemetry/dedup.rs`:

```rust
//! Workspace-scoped LRU dedup with TTL.
//!
//! Same call shape repeating within a session is suppressed and counted.
//! Different workspaces never cross-suppress (key includes workspace_id).

use crate::telemetry::events::LeafKind;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct DedupKey {
    pub kind: LeafKind,
    pub workspace_id: String,
    pub object_hash: Option<String>,
    pub procedure_hash: Option<String>,
    pub callee_object_type: Option<u8>, // discriminant of ObjectType
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub first_seen: Instant,
    pub last_seen: Instant,
    pub repeat_count: u32,
}

pub enum Decision {
    First,
    Repeat,
}

pub struct Dedup {
    cache: Mutex<LruCache<DedupKey, Entry>>,
    ttl: Duration,
}

impl Dedup {
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        let cap = NonZeroUsize::new(capacity.max(1)).unwrap();
        Self {
            cache: Mutex::new(LruCache::new(cap)),
            ttl,
        }
    }

    pub fn check(&self, key: &DedupKey, now: Instant) -> Decision {
        let mut cache = self.cache.lock().expect("dedup mutex poisoned");
        if let Some(entry) = cache.get_mut(key) {
            if now.saturating_duration_since(entry.last_seen) > self.ttl {
                // TTL expired — treat as new occurrence; reset the entry.
                *entry = Entry {
                    first_seen: now,
                    last_seen: now,
                    repeat_count: 0,
                };
                Decision::First
            } else {
                entry.last_seen = now;
                entry.repeat_count = entry.repeat_count.saturating_add(1);
                Decision::Repeat
            }
        } else {
            cache.put(
                key.clone(),
                Entry {
                    first_seen: now,
                    last_seen: now,
                    repeat_count: 0,
                },
            );
            Decision::First
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::events::LeafKind;

    fn key(workspace: &str, proc_hash: &str) -> DedupKey {
        DedupKey {
            kind: LeafKind::ResolutionProcedureNotFound,
            workspace_id: workspace.into(),
            object_hash: Some("obj".into()),
            procedure_hash: Some(proc_hash.into()),
            callee_object_type: Some(0),
        }
    }

    #[test]
    fn first_call_returns_first() {
        let d = Dedup::new(16, Duration::from_secs(60));
        match d.check(&key("ws", "p1"), Instant::now()) {
            Decision::First => {}
            _ => panic!(),
        }
    }

    #[test]
    fn repeat_within_ttl_returns_repeat() {
        let d = Dedup::new(16, Duration::from_secs(60));
        let now = Instant::now();
        let _ = d.check(&key("ws", "p1"), now);
        match d.check(&key("ws", "p1"), now) {
            Decision::Repeat => {}
            _ => panic!(),
        }
    }

    #[test]
    fn different_workspace_ids_do_not_cross_suppress() {
        let d = Dedup::new(16, Duration::from_secs(60));
        let now = Instant::now();
        match d.check(&key("ws_a", "p1"), now) {
            Decision::First => {}
            _ => panic!(),
        }
        match d.check(&key("ws_b", "p1"), now) {
            Decision::First => {}
            _ => panic!("ws_b should be First, not suppressed by ws_a"),
        }
    }

    #[test]
    fn ttl_expiry_resets_to_first() {
        let d = Dedup::new(16, Duration::from_millis(50));
        let t0 = Instant::now();
        let _ = d.check(&key("ws", "p1"), t0);
        let t1 = t0 + Duration::from_millis(100);
        match d.check(&key("ws", "p1"), t1) {
            Decision::First => {}
            _ => panic!("expired entry should be First again"),
        }
    }
}
```

- [ ] **Step 2: Register the module**

In `src/telemetry/mod.rs`:

```rust
#[cfg(feature = "telemetry")]
mod dedup;
```

- [ ] **Step 3: Run tests**

```bash
cargo test --lib telemetry::dedup
```
Expected: 4 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/telemetry/dedup.rs src/telemetry/mod.rs
git commit -m "feat(telemetry): add workspace-scoped LRU dedup with TTL"
```

---

### Task 1.4: Implement `pipeline.rs` (mpsc + try_send + producer counters)

**Files:**
- Create: `src/telemetry/pipeline.rs`

Spec reference: §6 "Hot path", §6 "Backpressure".

- [ ] **Step 1: Write the failing tests**

Create `src/telemetry/pipeline.rs`:

```rust
//! Producer-side mpsc channel and try_send wrapper.
//!
//! Hot path: one atomic load, one try_send. Drop-on-full is a counter bump.

use crate::telemetry::counters::Counters;
use crate::telemetry::events::{EventEnvelope, EventKind};
use std::sync::Arc;
use tokio::sync::mpsc::{self, error::TrySendError, Receiver, Sender};

pub struct Pipeline {
    tx: Sender<EventEnvelope>,
    counters: Arc<Counters>,
}

impl Pipeline {
    pub fn new(capacity: usize, counters: Arc<Counters>) -> (Self, Receiver<EventEnvelope>) {
        let (tx, rx) = mpsc::channel(capacity);
        (Self { tx, counters }, rx)
    }

    /// Non-blocking send. On full or closed channel, drops the event and bumps
    /// `queue_full_drops`. The hot path must call `counters.observe(...)` BEFORE
    /// calling this method so app-side observation is recorded regardless of fate.
    pub fn send(&self, env: EventEnvelope) {
        match self.tx.try_send(env) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) | Err(TrySendError::Closed(_)) => {
                self.counters.queue_full();
            }
        }
    }

    /// Used at shutdown to close the producer side and let the background
    /// thread observe channel disconnect.
    pub fn close(self) {
        drop(self.tx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::events::{EventKind, HandlerEmpty, ObjectType, DefinitionKind};
    use std::time::SystemTime;

    fn dummy_event() -> EventEnvelope {
        EventEnvelope {
            schema_version: 1,
            timestamp: SystemTime::now(),
            install_id: "0000000000000000".into(),
            al_version: env!("CARGO_PKG_VERSION"),
            grammar_version: "v2",
            os: "test",
            session_id: 0,
            workspace_id: "0000000000000000".into(),
            event: EventKind::HandlerEmpty(HandlerEmpty {
                method: "incomingCalls",
                target_object_type: ObjectType::Codeunit,
                target_kind: DefinitionKind::Procedure,
                object_hash: "x".into(),
                procedure_hash: "y".into(),
                repeat_count: 0,
            }),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn send_succeeds_when_capacity_available() {
        let counters = Arc::new(Counters::new());
        let (p, mut rx) = Pipeline::new(8, counters.clone());
        p.send(dummy_event());
        assert!(rx.try_recv().is_ok());
        assert_eq!(counters.snapshot().queue_full_drops, 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn send_drops_and_counts_when_full() {
        let counters = Arc::new(Counters::new());
        let (p, _rx) = Pipeline::new(2, counters.clone());
        // Don't read from rx; fill capacity.
        p.send(dummy_event());
        p.send(dummy_event());
        // Third send must drop.
        p.send(dummy_event());
        assert_eq!(counters.snapshot().queue_full_drops, 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn send_after_close_drops() {
        let counters = Arc::new(Counters::new());
        let (p, rx) = Pipeline::new(2, counters.clone());
        drop(rx);
        p.send(dummy_event());
        assert_eq!(counters.snapshot().queue_full_drops, 1);
    }
}
```

- [ ] **Step 2: Register module**

`mod pipeline;` in `src/telemetry/mod.rs`.

- [ ] **Step 3: Run tests**

```bash
cargo test --lib telemetry::pipeline
```
Expected: 3 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/telemetry/pipeline.rs src/telemetry/mod.rs
git commit -m "feat(telemetry): add bounded mpsc pipeline with drop-on-full"
```

---

### Task 1.5: Implement `exporter.rs` (background thread + AI exporter)

**Files:**
- Create: `src/telemetry/exporter.rs`

Spec reference: §3 architecture diagram, §6 "Background thread", §9 error rows for export failures.

- [ ] **Step 1: Write the failing tests**

Create `src/telemetry/exporter.rs`:

```rust
//! Background-thread exporter wrapping `opentelemetry-application-insights`.
//!
//! Owns the tokio current-thread runtime, the OTel SDK pipeline, and the
//! receiver end of the mpsc channel. Constructs and exports `session.summary`
//! at shutdown after queue drain.

use crate::telemetry::counters::Counters;
use crate::telemetry::events::{EventEnvelope, EventKind, SessionSummary};
use anyhow::{Context, Result};
use opentelemetry::{global, trace::{Tracer, TracerProvider as _}, KeyValue};
use opentelemetry_application_insights::new_pipeline_from_connection_string;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::Receiver;

pub struct ExporterConfig {
    pub connection_string: String,
    pub flush_interval: Duration,
    pub batch_size: u32,
}

/// Spawns a dedicated OS thread hosting a current-thread tokio runtime and
/// runs the exporter loop. Returns a join handle the caller awaits at shutdown.
pub fn spawn(
    config: ExporterConfig,
    rx: Receiver<EventEnvelope>,
    counters: Arc<Counters>,
    started_at: Instant,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("al-ch-telemetry".to_string())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio current-thread runtime");
            rt.block_on(run(config, rx, counters, started_at));
        })
        .expect("spawn telemetry thread")
}

async fn run(
    config: ExporterConfig,
    mut rx: Receiver<EventEnvelope>,
    counters: Arc<Counters>,
    started_at: Instant,
) {
    let tracer_provider = match new_pipeline_from_connection_string(&config.connection_string) {
        Ok(p) => p
            .with_client(reqwest::blocking::Client::new())
            .build_simple(),
        Err(e) => {
            log::warn!("telemetry: exporter init failed: {}; subsystem disabled", e);
            return;
        }
    };

    global::set_tracer_provider(tracer_provider.clone());
    let tracer = tracer_provider.tracer("al-call-hierarchy");

    while let Some(env) = rx.recv().await {
        export_event(&tracer, &env, &counters);
    }

    // Channel disconnected → producer side closed → drain done. Build summary.
    let summary = build_session_summary(started_at, &counters);
    export_summary(&tracer, &summary);

    if let Err(e) = tracer_provider.force_flush().into_iter().collect::<Result<Vec<_>, _>>() {
        log::warn!("telemetry: final flush error: {:?}", e);
    }
}

fn export_event(
    tracer: &impl Tracer,
    env: &EventEnvelope,
    counters: &Counters,
) {
    counters.export_attempted();
    let leaf = env.event.leaf();
    let label = match &env.event {
        EventKind::ResolutionMiss(_) => "resolution.miss",
        EventKind::ParserError(_) => "parser.error",
        EventKind::HandlerEmpty(_) => "handler.empty_result",
        EventKind::IndexerIssue(_) => "indexer.issue",
        EventKind::SessionStart(_) => "session.start",
        EventKind::SessionSummary(_) => "session.summary",
    };
    let mut span = tracer.start(label);
    span.set_attribute(KeyValue::new("telemetry.alch.schema_version", env.schema_version as i64));
    span.set_attribute(KeyValue::new("telemetry.alch.install_id", env.install_id.clone()));
    span.set_attribute(KeyValue::new("telemetry.alch.workspace_id", env.workspace_id.clone()));
    span.set_attribute(KeyValue::new("telemetry.alch.al_version", env.al_version));
    span.set_attribute(KeyValue::new("telemetry.alch.grammar_version", env.grammar_version));
    span.set_attribute(KeyValue::new("telemetry.alch.os", env.os));
    span.set_attribute(KeyValue::new("telemetry.alch.session_id", env.session_id as i64));
    crate::telemetry::events_attrs::apply(&mut span, &env.event);
    drop(span);
    if let Some(k) = leaf {
        counters.export_succeeded(k);
    }
}

fn export_summary(tracer: &impl Tracer, summary: &SessionSummary) {
    let mut span = tracer.start("session.summary");
    span.set_attribute(KeyValue::new("telemetry.alch.duration_secs", summary.duration_secs as i64));
    span.set_attribute(KeyValue::new("telemetry.alch.unique_patterns", summary.unique_patterns as i64));
    span.set_attribute(KeyValue::new("telemetry.alch.queue_full_drops", summary.queue_full_drops as i64));
    span.set_attribute(KeyValue::new("telemetry.alch.dedup_suppressed", summary.dedup_suppressed as i64));
    span.set_attribute(KeyValue::new("telemetry.alch.export_attempts", summary.export_attempts as i64));
    span.set_attribute(KeyValue::new("telemetry.alch.export_failures", summary.export_failures as i64));
    for (i, v) in summary.observed_by_kind.iter().enumerate() {
        span.set_attribute(KeyValue::new(
            format!("telemetry.alch.observed.{}", i),
            *v as i64,
        ));
    }
    for (i, v) in summary.exported_by_kind.iter().enumerate() {
        span.set_attribute(KeyValue::new(
            format!("telemetry.alch.exported.{}", i),
            *v as i64,
        ));
    }
    drop(span);
}

fn build_session_summary(started_at: Instant, counters: &Counters) -> SessionSummary {
    let snap = counters.snapshot();
    SessionSummary {
        duration_secs: started_at.elapsed().as_secs(),
        unique_patterns: 0, // populated by dedup module in Phase 2 instrumentation if desired
        queue_full_drops: snap.queue_full_drops,
        dedup_suppressed: snap.dedup_suppressed,
        export_attempts: snap.export_attempts,
        export_failures: snap.export_failures,
        observed_by_kind: snap.observed_by_kind,
        exported_by_kind: snap.exported_by_kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_summary_pulls_atomics() {
        let c = Counters::new();
        c.queue_full();
        c.queue_full();
        c.dedup_suppress();
        c.observe(crate::telemetry::events::LeafKind::ParserTreeError);
        let summary = build_session_summary(Instant::now(), &c);
        assert_eq!(summary.queue_full_drops, 2);
        assert_eq!(summary.dedup_suppressed, 1);
        assert_eq!(
            summary.observed_by_kind[crate::telemetry::events::LeafKind::ParserTreeError.index()],
            1
        );
    }
}
```

- [ ] **Step 2: Add `events_attrs.rs` for event-specific attributes**

Create `src/telemetry/events_attrs.rs`:

```rust
//! Translates `EventKind` payload fields to OTel `KeyValue` attributes.
//! Kept separate from `events.rs` so the privacy lint can scan event structs
//! without crossing into pipeline code.

use crate::telemetry::events::{EventKind, ResolutionMiss};
use opentelemetry::KeyValue;
use opentelemetry::trace::Span;

pub fn apply(span: &mut impl Span, event: &EventKind) {
    match event {
        EventKind::ResolutionMiss(m) => apply_resolution(span, m),
        EventKind::ParserError(e) => {
            span.set_attribute(KeyValue::new("telemetry.alch.parser_kind", format!("{:?}", e.kind)));
            span.set_attribute(KeyValue::new("telemetry.alch.file_extension", e.file_extension.clone()));
            span.set_attribute(KeyValue::new("telemetry.alch.file_size_bucket", format!("{:?}", e.file_size_bucket)));
            span.set_attribute(KeyValue::new("telemetry.alch.error_count", e.error_count as i64));
            span.set_attribute(KeyValue::new("telemetry.alch.repeat_count", e.repeat_count as i64));
            span.set_attribute(KeyValue::new("telemetry.alch.file_hash", e.file_hash.clone()));
            if let Some(ref h) = e.node_kind_hash {
                span.set_attribute(KeyValue::new("telemetry.alch.node_kind_hash", h.clone()));
            }
        }
        EventKind::HandlerEmpty(h) => {
            span.set_attribute(KeyValue::new("telemetry.alch.method", h.method));
            span.set_attribute(KeyValue::new("telemetry.alch.target_object_type", format!("{:?}", h.target_object_type)));
            span.set_attribute(KeyValue::new("telemetry.alch.target_kind", format!("{:?}", h.target_kind)));
            span.set_attribute(KeyValue::new("telemetry.alch.object_hash", h.object_hash.clone()));
            span.set_attribute(KeyValue::new("telemetry.alch.procedure_hash", h.procedure_hash.clone()));
            span.set_attribute(KeyValue::new("telemetry.alch.repeat_count", h.repeat_count as i64));
        }
        EventKind::IndexerIssue(i) => {
            span.set_attribute(KeyValue::new("telemetry.alch.indexer_kind", format!("{:?}", i.kind)));
            span.set_attribute(KeyValue::new("telemetry.alch.detail_code", i.detail_code as i64));
            if let Some(ref h) = i.app_id_hash {
                span.set_attribute(KeyValue::new("telemetry.alch.app_id_hash", h.clone()));
            }
        }
        EventKind::SessionStart(s) => {
            span.set_attribute(KeyValue::new("telemetry.alch.workspace_file_count", s.workspace_file_count as i64));
            span.set_attribute(KeyValue::new("telemetry.alch.al_file_count_bucket", format!("{:?}", s.al_file_count_bucket)));
            span.set_attribute(KeyValue::new("telemetry.alch.dependency_count", s.dependency_count as i64));
            span.set_attribute(KeyValue::new("telemetry.alch.has_app_dependencies", s.has_app_dependencies));
            span.set_attribute(KeyValue::new("telemetry.alch.config_flags_bits", s.config_flags.bits as i64));
            span.set_attribute(KeyValue::new("telemetry.alch.previous_session_unclean", s.previous_session_unclean));
        }
        EventKind::SessionSummary(_) => {
            // Handled in the exporter directly to avoid duplicating the attribute set.
        }
    }
}

fn apply_resolution(span: &mut impl Span, m: &ResolutionMiss) {
    span.set_attribute(KeyValue::new("telemetry.alch.failure", format!("{:?}", m.failure)));
    span.set_attribute(KeyValue::new("telemetry.alch.call_pattern", format!("{:?}", m.call_pattern)));
    if let Some(t) = m.callee_object_type {
        span.set_attribute(KeyValue::new("telemetry.alch.callee_object_type", format!("{:?}", t)));
    }
    span.set_attribute(KeyValue::new("telemetry.alch.callee_source", format!("{:?}", m.callee_source)));
    span.set_attribute(KeyValue::new("telemetry.alch.caller_object_type", format!("{:?}", m.caller_object_type)));
    span.set_attribute(KeyValue::new("telemetry.alch.caller_context", format!("{:?}", m.caller_context)));
    if let Some(ref h) = m.object_hash {
        span.set_attribute(KeyValue::new("telemetry.alch.object_hash", h.clone()));
    }
    span.set_attribute(KeyValue::new("telemetry.alch.procedure_hash", m.procedure_hash.clone()));
    span.set_attribute(KeyValue::new("telemetry.alch.arg_count", m.arg_count as i64));
    if let Some(n) = m.name_len_object {
        span.set_attribute(KeyValue::new("telemetry.alch.name_len_object", n as i64));
    }
    span.set_attribute(KeyValue::new("telemetry.alch.name_len_procedure", m.name_len_procedure as i64));
    span.set_attribute(KeyValue::new("telemetry.alch.ts_node_path", m.ts_node_path.clone()));
    span.set_attribute(KeyValue::new("telemetry.alch.repeat_count", m.repeat_count as i64));
}
```

- [ ] **Step 3: Register both modules**

In `src/telemetry/mod.rs`:

```rust
#[cfg(feature = "telemetry")]
mod exporter;
#[cfg(feature = "telemetry")]
mod events_attrs;
```

- [ ] **Step 4: Run tests**

```bash
cargo test --lib telemetry::exporter
```
Expected: 1 test passes.

- [ ] **Step 5: Commit**

```bash
git add src/telemetry/exporter.rs src/telemetry/events_attrs.rs src/telemetry/mod.rs
git commit -m "feat(telemetry): add background exporter with App Insights pipeline"
```

---

### Task 1.6: Wire `init`/`shutdown` to the real pipeline

**Files:**
- Modify: `src/telemetry/mod.rs`
- Modify: `src/server.rs` (pass `init_params`, `workspace_root`, `args.no_telemetry` through)

Spec reference: §4 "Public API", §6 "Shutdown".

- [ ] **Step 1: Replace the no-op `init`/`shutdown` with the real wiring**

Edit `src/telemetry/mod.rs`. Replace the existing `TelemetryHandle`, `init`, and `shutdown` definitions with:

```rust
#[cfg(feature = "telemetry")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(feature = "telemetry")]
use std::sync::Arc;
#[cfg(feature = "telemetry")]
use std::time::{Duration, Instant};

#[cfg(feature = "telemetry")]
pub struct TelemetryInputs {
    pub cli_no_telemetry: bool,
    pub init_option: Option<bool>,
    pub workspace_root: Option<std::path::PathBuf>,
    pub connection_string: Option<String>,
}

#[cfg(feature = "telemetry")]
pub struct TelemetryHandle {
    enabled: bool,
    counters: Option<Arc<counters::Counters>>,
    pipeline_close: Option<Box<dyn FnOnce() + Send>>,
    join: Option<std::thread::JoinHandle<()>>,
    started_at: Instant,
    salt: hash::Salt,
    workspace_id: String,
    install_id: String,
    session_id: u64,
}

#[cfg(feature = "telemetry")]
pub fn init(inputs: TelemetryInputs) -> TelemetryHandle {
    let env = consent::live_env();
    let config = inputs
        .workspace_root
        .as_ref()
        .map(|root| crate::config::TelemetryFileConfig::load_merged(root))
        .unwrap_or_default();
    let consent_inputs = consent::Inputs {
        cli_no_telemetry: inputs.cli_no_telemetry,
        init_option: inputs.init_option,
        config: config.enabled,
        env,
        is_debug: cfg!(debug_assertions),
        is_test: cfg!(test),
    };
    let decision = consent::decide(&consent_inputs);
    let connection_string = inputs.connection_string.or(config.connection_string);

    let enabled = matches!(decision, consent::Decision::Enabled) && connection_string.is_some();
    if !enabled {
        if let consent::Decision::Disabled(reason) = &decision {
            log::info!("telemetry: disabled ({:?})", reason);
        } else if connection_string.is_none() {
            log::info!("telemetry: disabled (no connection string configured)");
        }
        return disabled_handle();
    }

    let (salt, _persisted) = install_id::load_or_create();
    let install_id = hash::install_id_from_salt(&salt);
    let workspace_id = inputs
        .workspace_root
        .as_ref()
        .map(|p| hash::hash_short(&salt, hash::DOMAIN_WORKSPACE, p.to_string_lossy().as_bytes()))
        .unwrap_or_else(|| "0000000000000000".into());
    let marker = session_marker::record_session_start();
    let previous_session_unclean = marker.previous_session_unclean;
    let counters = Arc::new(counters::Counters::new());
    let started_at = Instant::now();
    let session_id: u64 = {
        let mut h = blake3::Hasher::new();
        h.update(&started_at.elapsed().as_nanos().to_le_bytes());
        h.update(&std::process::id().to_le_bytes());
        let d = h.finalize();
        u64::from_le_bytes(d.as_bytes()[..8].try_into().unwrap())
    };

    let (pipeline, rx) = pipeline::Pipeline::new(
        config.queue_capacity.unwrap_or(2048) as usize,
        counters.clone(),
    );
    let exporter_config = exporter::ExporterConfig {
        connection_string: connection_string.unwrap(),
        flush_interval: Duration::from_secs(config.flush_interval_secs.unwrap_or(5)),
        batch_size: config.batch_size.unwrap_or(512),
    };
    let join = exporter::spawn(exporter_config, rx, counters.clone(), started_at);
    runtime::install(pipeline, counters.clone(), salt, workspace_id.clone(), install_id.clone(), session_id, previous_session_unclean);

    log::info!(
        "telemetry: enabled (anonymous, hashed). install_id={}. Disable: AL_CH_TELEMETRY=0 or telemetry.enabled=false in ~/.al-call-hierarchy/config.json",
        install_id
    );

    TelemetryHandle {
        enabled: true,
        counters: Some(counters),
        pipeline_close: Some(Box::new(|| runtime::close_pipeline())),
        join: Some(join),
        started_at,
        salt,
        workspace_id,
        install_id,
        session_id,
    }
}

#[cfg(feature = "telemetry")]
fn disabled_handle() -> TelemetryHandle {
    TelemetryHandle {
        enabled: false,
        counters: None,
        pipeline_close: None,
        join: None,
        started_at: Instant::now(),
        salt: [0u8; 32],
        workspace_id: String::new(),
        install_id: String::new(),
        session_id: 0,
    }
}

#[cfg(feature = "telemetry")]
pub fn shutdown(handle: TelemetryHandle) {
    if !handle.enabled {
        return;
    }
    if let Some(close) = handle.pipeline_close {
        close();
    }
    // Background thread: drain mpsc, build+export summary, force-flush.
    if let Some(join) = handle.join {
        let _ = join.join();
    }
    session_marker::record_clean_shutdown();
}

#[cfg(not(feature = "telemetry"))]
pub struct TelemetryInputs {
    pub cli_no_telemetry: bool,
    pub init_option: Option<bool>,
    pub workspace_root: Option<std::path::PathBuf>,
    pub connection_string: Option<String>,
}

#[cfg(not(feature = "telemetry"))]
pub struct TelemetryHandle;

#[cfg(not(feature = "telemetry"))]
pub fn init(_inputs: TelemetryInputs) -> TelemetryHandle {
    TelemetryHandle
}

#[cfg(not(feature = "telemetry"))]
pub fn shutdown(_h: TelemetryHandle) {}
```

- [ ] **Step 2: Create the runtime singleton module**

Create `src/telemetry/runtime.rs`:

```rust
//! Process-wide singleton holding pipeline + salt + identifiers, so
//! `record_*` functions can be called without threading a handle everywhere.
//!
//! Set during `init`. Read by `record_*` functions on the hot path.

use crate::telemetry::counters::Counters;
use crate::telemetry::hash::Salt;
use crate::telemetry::pipeline::Pipeline;
use std::sync::{Arc, OnceLock, RwLock};

pub(super) struct Runtime {
    pub pipeline: RwLock<Option<Pipeline>>,
    pub counters: Arc<Counters>,
    pub salt: Salt,
    pub workspace_id: String,
    pub install_id: String,
    pub session_id: u64,
    pub previous_session_unclean: bool,
}

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

pub(super) fn install(
    pipeline: Pipeline,
    counters: Arc<Counters>,
    salt: Salt,
    workspace_id: String,
    install_id: String,
    session_id: u64,
    previous_session_unclean: bool,
) {
    let _ = RUNTIME.set(Runtime {
        pipeline: RwLock::new(Some(pipeline)),
        counters,
        salt,
        workspace_id,
        install_id,
        session_id,
        previous_session_unclean,
    });
}

pub(super) fn get() -> Option<&'static Runtime> {
    RUNTIME.get()
}

pub(super) fn close_pipeline() {
    if let Some(rt) = RUNTIME.get() {
        let mut guard = rt.pipeline.write().expect("runtime pipeline lock poisoned");
        if let Some(p) = guard.take() {
            p.close();
        }
    }
}
```

- [ ] **Step 3: Register `runtime` in `mod.rs`**

```rust
#[cfg(feature = "telemetry")]
mod runtime;
```

- [ ] **Step 4: Update `server.rs` to construct `TelemetryInputs`**

In `src/server.rs::run_server`, replace the existing `crate::telemetry::init();` call with:

```rust
    let workspace_root = init_params
        .workspace_folders
        .as_ref()
        .and_then(|folders| folders.first())
        .and_then(|f| crate::protocol::uri_to_path(&f.uri));
    let init_option_telemetry = init_params
        .initialization_options
        .as_ref()
        .and_then(|v| v.get("telemetry"))
        .and_then(|t| t.get("enabled"))
        .and_then(|b| b.as_bool());
    let telemetry_handle = crate::telemetry::init(crate::telemetry::TelemetryInputs {
        cli_no_telemetry: no_telemetry,
        init_option: init_option_telemetry,
        workspace_root,
        connection_string: option_env!("AL_CH_TELEMETRY_CONNECTION_STRING").map(String::from),
    });
```

(`option_env!` will return `None` when the env var was unset at build time. Phase 3 adds `build.rs` to bake it in.)

- [ ] **Step 5: Build both feature configs**

```bash
cargo build
cargo build --no-default-features
```
Expected: both succeed.

- [ ] **Step 6: Commit**

```bash
git add src/telemetry/mod.rs src/telemetry/runtime.rs src/server.rs
git commit -m "feat(telemetry): wire pipeline/exporter into init and shutdown"
```

---

### Task 1.7: Implement public `record_*` functions

**Files:**
- Modify: `src/telemetry/mod.rs`

Spec reference: §4 "Public API".

- [ ] **Step 1: Add the public record functions**

Append to `src/telemetry/mod.rs`:

```rust
#[cfg(feature = "telemetry")]
use crate::telemetry::events::{
    CallPattern, CallerContext, CalleeSource, ConfigFlags, DefinitionKind, EventEnvelope,
    EventKind, HandlerEmpty, IndexerIssue, IndexerIssueKind, ObjectType, ParserError,
    ParserErrorKind, ResolutionFailure, ResolutionMiss, SessionStart, SizeBucket,
};

#[cfg(feature = "telemetry")]
pub struct CallContext<'a> {
    pub failure: ResolutionFailure,
    pub call_pattern: CallPattern,
    pub callee_object_type: Option<ObjectType>,
    pub callee_source: CalleeSource,
    pub caller_object_type: ObjectType,
    pub caller_context: CallerContext,
    pub callee_object_name: Option<&'a str>,
    pub callee_procedure_name: &'a str,
    pub arg_count: u8,
    pub ts_node_path: &'a str,
}

#[cfg(feature = "telemetry")]
pub fn record_resolution_miss(ctx: &CallContext<'_>) {
    let Some(rt) = runtime::get() else { return };
    let leaf = match ctx.failure {
        ResolutionFailure::ObjectNotFound => events::LeafKind::ResolutionObjectNotFound,
        ResolutionFailure::ProcedureNotFound => events::LeafKind::ResolutionProcedureNotFound,
        ResolutionFailure::UnresolvedUnqualified => events::LeafKind::ResolutionUnresolvedUnqualified,
        ResolutionFailure::Ambiguous => events::LeafKind::ResolutionAmbiguous,
        ResolutionFailure::UnsupportedConstruct => events::LeafKind::ResolutionUnsupportedConstruct,
    };
    rt.counters.observe(leaf);

    let object_hash = ctx
        .callee_object_name
        .map(|n| hash::hash_identifier(&rt.salt, hash::DOMAIN_OBJECT, n));
    let procedure_hash = hash::hash_identifier(&rt.salt, hash::DOMAIN_PROCEDURE, ctx.callee_procedure_name);

    let env = EventEnvelope {
        schema_version: events::SCHEMA_VERSION,
        timestamp: std::time::SystemTime::now(),
        install_id: rt.install_id.clone(),
        al_version: env!("CARGO_PKG_VERSION"),
        grammar_version: "v2",
        os: events::current_os(),
        session_id: rt.session_id,
        workspace_id: rt.workspace_id.clone(),
        event: EventKind::ResolutionMiss(ResolutionMiss {
            failure: ctx.failure,
            call_pattern: ctx.call_pattern,
            callee_object_type: ctx.callee_object_type,
            callee_source: ctx.callee_source,
            caller_object_type: ctx.caller_object_type,
            caller_context: ctx.caller_context,
            object_hash,
            procedure_hash,
            arg_count: ctx.arg_count,
            name_len_object: ctx.callee_object_name.map(|n| n.len() as u16),
            name_len_procedure: ctx.callee_procedure_name.len() as u16,
            ts_node_path: ctx.ts_node_path.into(),
            repeat_count: 0,
        }),
    };

    if let Some(p) = rt.pipeline.read().ok().and_then(|g| g.as_ref().map(|p| p.clone_sender())) {
        p.send(env);
    }
}

// Stubs for the other record_* functions; Phase 2 fills them.
#[cfg(feature = "telemetry")]
pub fn record_parser_error(_kind: ParserErrorKind, _file: &std::path::Path) {}
#[cfg(feature = "telemetry")]
pub fn record_indexer_issue(_kind: IndexerIssueKind, _detail_code: u16, _app_id: Option<&str>) {}
#[cfg(feature = "telemetry")]
pub fn record_handler_empty(
    _method: &'static str,
    _target_object_type: ObjectType,
    _target_kind: DefinitionKind,
    _object_name: &str,
    _procedure_name: &str,
) {}
#[cfg(feature = "telemetry")]
pub fn record_session_start(
    _workspace_file_count: u32,
    _dependency_count: u8,
    _has_app_dependencies: bool,
) {}

#[cfg(not(feature = "telemetry"))]
pub fn record_resolution_miss<T>(_ctx: T) {}
#[cfg(not(feature = "telemetry"))]
pub fn record_parser_error<K, P>(_kind: K, _file: P) {}
#[cfg(not(feature = "telemetry"))]
pub fn record_indexer_issue<K>(_kind: K, _detail_code: u16, _app_id: Option<&str>) {}
#[cfg(not(feature = "telemetry"))]
pub fn record_handler_empty<O, K>(_method: &'static str, _t: O, _k: K, _o: &str, _p: &str) {}
#[cfg(not(feature = "telemetry"))]
pub fn record_session_start(_a: u32, _b: u8, _c: bool) {}
```

- [ ] **Step 2: Add `clone_sender` to `Pipeline`**

In `src/telemetry/pipeline.rs`, add to `impl Pipeline`:

```rust
    /// Cheap clone of the sender so multiple producer threads share the channel.
    pub fn clone_sender(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            counters: self.counters.clone(),
        }
    }
```

- [ ] **Step 3: Build both configs**

```bash
cargo build
cargo build --no-default-features
```
Expected: both succeed.

- [ ] **Step 4: Commit**

```bash
git add src/telemetry/mod.rs src/telemetry/pipeline.rs
git commit -m "feat(telemetry): expose record_* public API; record_resolution_miss live"
```

---

### Task 1.8: Add `al-call-hierarchy/telemetryStatus` LSP request handler

**Files:**
- Create: `src/telemetry/status.rs`
- Modify: `src/handlers.rs` (route the new method)
- Modify: `src/telemetry/mod.rs` (expose `status()`)

Spec reference: §7 "Transparency endpoint".

- [ ] **Step 1: Create the status module**

Create `src/telemetry/status.rs`:

```rust
//! Snapshot of telemetry runtime state for the transparency LSP request.

use crate::telemetry::runtime;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct TelemetryStatus {
    pub enabled: bool,
    pub install_id: String,
    pub workspace_id: String,
    pub events_sent_session: u32,
    pub events_dropped_queue_full: u32,
    pub events_dropped_dedup: u32,
    pub export_failures: u32,
    pub schema_version: u8,
}

pub fn snapshot() -> TelemetryStatus {
    let Some(rt) = runtime::get() else {
        return TelemetryStatus {
            enabled: false,
            install_id: String::new(),
            workspace_id: String::new(),
            events_sent_session: 0,
            events_dropped_queue_full: 0,
            events_dropped_dedup: 0,
            export_failures: 0,
            schema_version: crate::telemetry::events::SCHEMA_VERSION,
        };
    };
    let snap = rt.counters.snapshot();
    TelemetryStatus {
        enabled: true,
        install_id: rt.install_id.clone(),
        workspace_id: rt.workspace_id.clone(),
        events_sent_session: snap.exported_by_kind.iter().sum(),
        events_dropped_queue_full: snap.queue_full_drops,
        events_dropped_dedup: snap.dedup_suppressed,
        export_failures: snap.export_failures,
        schema_version: crate::telemetry::events::SCHEMA_VERSION,
    }
}
```

- [ ] **Step 2: Expose `status()` from `mod.rs`**

In `src/telemetry/mod.rs`:

```rust
#[cfg(feature = "telemetry")]
pub mod status;

#[cfg(feature = "telemetry")]
pub fn status() -> status::TelemetryStatus {
    status::snapshot()
}

#[cfg(not(feature = "telemetry"))]
#[derive(Debug, serde::Serialize)]
pub struct TelemetryStatus {
    pub enabled: bool,
}

#[cfg(not(feature = "telemetry"))]
pub fn status() -> TelemetryStatus {
    TelemetryStatus { enabled: false }
}
```

- [ ] **Step 3: Route the LSP method**

In `src/handlers.rs::handle_request`, add a new arm before the trailing `_ => { ... }`:

```rust
        "al-call-hierarchy/telemetryStatus" => {
            let result = crate::telemetry::status();
            Ok(serde_json::to_value(result)?)
        }
```

- [ ] **Step 4: Build**

```bash
cargo build
cargo build --no-default-features
```
Expected: both succeed.

- [ ] **Step 5: Commit**

```bash
git add src/telemetry/status.rs src/telemetry/mod.rs src/handlers.rs
git commit -m "feat(telemetry): add al-call-hierarchy/telemetryStatus LSP request"
```

---

### Task 1.9: Hot-path benchmark

**Files:**
- Create: `benches/telemetry_hot_path.rs`
- Modify: `Cargo.toml` (add criterion + bench harness)

Spec reference: §6 hot-path budget.

- [ ] **Step 1: Add criterion to dev-dependencies**

In `Cargo.toml`:

```toml
[dev-dependencies]
# (existing entries kept)
criterion = "0.5"

[[bench]]
name = "telemetry_hot_path"
harness = false
```

- [ ] **Step 2: Write the benchmark**

Create `benches/telemetry_hot_path.rs`:

```rust
//! Hot-path benchmark: `record_resolution_miss` must average ≤ 5µs per call
//! when telemetry is enabled, and effectively zero when disabled.

#![cfg(feature = "telemetry")]

use al_call_hierarchy::telemetry;
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_disabled(c: &mut Criterion) {
    c.bench_function("record_resolution_miss / disabled", |b| {
        b.iter(|| {
            // Without init, runtime::get() is None; record_* returns immediately.
            telemetry::record_resolution_miss(&black_box(make_ctx()));
        });
    });
}

fn make_ctx() -> telemetry::CallContext<'static> {
    telemetry::CallContext {
        failure: telemetry::events::ResolutionFailure::ProcedureNotFound,
        call_pattern: telemetry::events::CallPattern::Qualified,
        callee_object_type: Some(telemetry::events::ObjectType::Codeunit),
        callee_source: telemetry::events::CalleeSource::AppDependency,
        caller_object_type: telemetry::events::ObjectType::Page,
        caller_context: telemetry::events::CallerContext::Trigger,
        callee_object_name: Some("CustomerObj"),
        callee_procedure_name: "PostInvoice",
        arg_count: 2,
        ts_node_path: "method_call>member_expression>identifier",
    }
}

criterion_group!(benches, bench_disabled);
criterion_main!(benches);
```

- [ ] **Step 3: Make `telemetry` and submodules `pub` for benches to use them**

In `src/telemetry/mod.rs` ensure `pub mod events;` is present (it is, from Task 0.8).

In `src/main.rs`, the `mod telemetry;` line should be `pub mod telemetry;` so benches can access it via the library crate. If the project has no `lib.rs`, add a minimal one:

Create `src/lib.rs`:

```rust
//! Internal exports for benchmarks. The binary uses `main.rs`.
pub mod telemetry;
```

And change `src/main.rs`:

```rust
use al_call_hierarchy::telemetry; // replaces `mod telemetry;`
```

(Move `mod telemetry;` from `main.rs` into the new `lib.rs` if a lib doesn't already exist. Existing modules like `analysis`, `parser`, etc. stay in `main.rs` — only `telemetry` moves out for now.)

- [ ] **Step 4: Run the benchmark (release mode)**

```bash
cargo bench --bench telemetry_hot_path
```
Expected: criterion reports a per-call mean. With telemetry disabled (no init), it should be in the low-nanosecond range. The "enabled" benchmark is added in Task 1.11 once integration tests give us a reliable mock-init path.

- [ ] **Step 5: Commit**

```bash
git add benches/telemetry_hot_path.rs Cargo.toml Cargo.lock src/lib.rs src/main.rs
git commit -m "bench(telemetry): add hot-path criterion benchmark"
```

---

### Task 1.10: Integration test against mock exporter

**Files:**
- Create: `tests/telemetry_integration.rs`

- [ ] **Step 1: Write the integration test**

Create `tests/telemetry_integration.rs`:

```rust
//! End-to-end tests: queue overflow accounting, dedup workspace isolation,
//! shutdown drains and emits summary.

#![cfg(feature = "telemetry")]

use al_call_hierarchy::telemetry::counters::Counters;
use al_call_hierarchy::telemetry::events::LeafKind;
use al_call_hierarchy::telemetry::pipeline::Pipeline;
use std::sync::Arc;
use std::time::Duration;

#[tokio::test(flavor = "current_thread")]
async fn queue_full_distinguishable_from_dedup() {
    let counters = Arc::new(Counters::new());
    let (p, _rx) = Pipeline::new(2, counters.clone());
    counters.observe(LeafKind::ResolutionObjectNotFound);
    counters.dedup_suppress();
    counters.queue_full();

    let snap = counters.snapshot();
    assert_eq!(snap.queue_full_drops, 1);
    assert_eq!(snap.dedup_suppressed, 1);
    assert_eq!(snap.observed_by_kind[LeafKind::ResolutionObjectNotFound.index()], 1);

    drop(p);
}
```

- [ ] **Step 2: Run the test**

```bash
cargo test --test telemetry_integration
```
Expected: passes.

- [ ] **Step 3: Commit**

```bash
git add tests/telemetry_integration.rs
git commit -m "test(telemetry): integration test for counter dimensions"
```

---

### Task 1.11: Phase 1 wrap

**Files:** none modified (verification only).

- [ ] **Step 1: Build matrix**

```bash
cargo build
cargo build --release
cargo build --no-default-features
cargo test
cargo test --no-default-features
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: all green.

- [ ] **Step 2: Tag end-of-phase**

```bash
git tag -a phase-1-pipeline -m "Telemetry Phase 1 pipeline complete (counters/dedup/exporter wired)"
```

Phase 1 is mergeable. PR description: "Telemetry pipeline implemented end-to-end. `record_resolution_miss` is live; other `record_*` are stubs that get filled in Phase 2 instrumentation."

---

## Phase 2 — Instrumentation (PR #3)

**Phase goal:** Wire `record_*` calls into the actual code paths. Add fixture tests that assert specific failure shapes produce the expected events.

### Task 2.1: Identify resolution call sites

**Files:** none modified (research step).

- [ ] **Step 1: Read `parser.rs` resolution paths**

Search for the call resolution logic:

```bash
grep -n "resolve\|qualified\|callee" src/parser.rs | head -30
```

Document the call sites where:
- A qualified call (`Object.Method`) is parsed but its target is not in the graph → `ResolutionFailure::ObjectNotFound` or `ProcedureNotFound`.
- An unqualified procedure call is parsed but no local definition matches → `UnresolvedUnqualified`.
- A V2 grammar node shape is encountered that no query handles → `UnsupportedConstruct`.

Write a short note in your task tracker: file path + line range for each instrumentation point. (No code change yet.)

- [ ] **Step 2: Read `indexer.rs` for issue detection points**

```bash
grep -n "missing\|dependency\|not.found\|app_id" src/indexer.rs src/dependencies.rs
```

Identify:
- Where `app.json` declares a dependency not present in `.alpackages` → `MissingDependency`.
- Where `.app` parsing fails → `AppParseFailed`.

- [ ] **Step 3: Read `handlers.rs` for empty-result points**

```bash
grep -n "incoming_calls\|outgoing_calls" src/handlers.rs
```

Identify the points at the end of each handler where a result is being returned. The instrumentation needs to fire when the result is empty (after the 10% sampling check).

- [ ] **Step 4: Capture findings**

Write a short comment block at the top of `src/telemetry/mod.rs`:

```rust
//! Instrumentation map (Phase 2):
//! - parser.rs: <line ranges> for ResolutionMiss
//! - parser.rs: <line ranges> for ParserError
//! - indexer.rs: <line ranges> for IndexerIssue
//! - handlers.rs: <line ranges> for HandlerEmpty
```

- [ ] **Step 5: Commit notes**

```bash
git add src/telemetry/mod.rs
git commit -m "docs(telemetry): map Phase 2 instrumentation call sites"
```

---

### Task 2.2: Add fixture for unresolved app-dependency call

**Files:**
- Create: `tests/fixtures/telemetry/unresolved_app_dep/app.json`
- Create: `tests/fixtures/telemetry/unresolved_app_dep/UnresolvedCall.al`

- [ ] **Step 1: Create app.json declaring a dependency that won't be present**

Create `tests/fixtures/telemetry/unresolved_app_dep/app.json`:

```json
{
  "id": "11111111-1111-1111-1111-111111111111",
  "name": "TelemetryTestApp",
  "publisher": "Test",
  "version": "1.0.0.0",
  "dependencies": [
    {
      "id": "22222222-2222-2222-2222-222222222222",
      "name": "MissingExternalApp",
      "publisher": "External",
      "version": "1.0.0.0"
    }
  ]
}
```

- [ ] **Step 2: Create AL file calling into the missing dependency**

Create `tests/fixtures/telemetry/unresolved_app_dep/UnresolvedCall.al`:

```al
codeunit 50100 "Test Caller"
{
    procedure RunTest()
    var
        ExternalCodeunit: Codeunit "Missing External Codeunit";
    begin
        ExternalCodeunit.PostInvoice('CUST001', 100);
    end;
}
```

- [ ] **Step 3: Verify fixture parses**

```bash
cargo run -- --project tests/fixtures/telemetry/unresolved_app_dep --no-lsp
```

Expected: indexer runs, reports the call site, logs a missing-dependency warning. (No assertion yet — the next task adds a test.)

- [ ] **Step 4: Commit**

```bash
git add tests/fixtures/telemetry/unresolved_app_dep/
git commit -m "test(telemetry): fixture for unresolved app-dependency call"
```

---

### Task 2.3: Wire `record_resolution_miss` in parser

**Files:**
- Modify: `src/parser.rs`
- Modify: `src/telemetry/mod.rs` (already has `record_resolution_miss`)
- Create/extend: `tests/telemetry_integration.rs`

- [ ] **Step 1: Find the resolution failure branch in parser.rs**

(Use the locations identified in Task 2.1.) For each branch where a call cannot be resolved, insert:

```rust
#[cfg(feature = "telemetry")]
crate::telemetry::record_resolution_miss(&crate::telemetry::CallContext {
    failure: crate::telemetry::events::ResolutionFailure::ProcedureNotFound, // or appropriate variant
    call_pattern: /* derive from call_site */,
    callee_object_type: /* derive */,
    callee_source: /* derive: Workspace | AppDependency | System | Unknown */,
    caller_object_type: /* derive */,
    caller_context: /* derive */,
    callee_object_name: Some(&object_name),
    callee_procedure_name: &procedure_name,
    arg_count: arg_count as u8,
    ts_node_path: &ts_node_path_for(&node),
});
```

Add a small helper in `src/parser.rs` to compute `ts_node_path` from a tree-sitter node:

```rust
fn ts_node_path_for(node: &tree_sitter::Node) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let mut cur = Some(*node);
    let mut depth = 0;
    while let Some(n) = cur {
        if depth >= 4 {
            break;
        }
        parts.push(n.kind());
        cur = n.parent();
        depth += 1;
    }
    parts.reverse();
    parts.join(">")
}
```

- [ ] **Step 2: Add an integration test for the fixture**

Append to `tests/telemetry_integration.rs`:

```rust
#[test]
fn unresolved_app_dep_call_records_resolution_miss() {
    use al_call_hierarchy::telemetry::counters::Counters;
    use al_call_hierarchy::telemetry::events::LeafKind;
    use std::sync::Arc;

    // Arrange: install a runtime with a fresh Counters; do NOT spawn the exporter
    // (we only check that observed counters increment).
    let counters = Arc::new(Counters::new());
    al_call_hierarchy::telemetry::testing::install_runtime_for_test(counters.clone());

    // Act: index the fixture project, which triggers parser resolution paths.
    let project = std::path::Path::new("tests/fixtures/telemetry/unresolved_app_dep");
    let mut indexer = al_call_hierarchy::indexer::Indexer::new();
    indexer.index_directory(project).unwrap();

    // Assert: at least one resolution miss was observed.
    let snap = counters.snapshot();
    let total_resolution_misses: u32 = [
        LeafKind::ResolutionObjectNotFound,
        LeafKind::ResolutionProcedureNotFound,
        LeafKind::ResolutionUnresolvedUnqualified,
        LeafKind::ResolutionAmbiguous,
        LeafKind::ResolutionUnsupportedConstruct,
    ]
    .iter()
    .map(|k| snap.observed_by_kind[k.index()])
    .sum();
    assert!(total_resolution_misses > 0, "expected at least one resolution miss");
}
```

- [ ] **Step 3: Add a test-only helper to install a counters-only runtime**

In `src/telemetry/runtime.rs`, add at the bottom:

```rust
#[cfg(any(test, feature = "test-runtime"))]
pub mod testing {
    use super::*;
    use crate::telemetry::counters::Counters;
    use std::sync::Arc;

    /// For integration tests: install a no-exporter runtime so `record_*`
    /// calls increment counters but never block on a network/exporter.
    pub fn install_runtime_for_test(counters: Arc<Counters>) {
        let _ = RUNTIME.set(Runtime {
            pipeline: RwLock::new(None),
            counters,
            salt: [0u8; 32],
            workspace_id: "test_workspace".into(),
            install_id: "0000000000000000".into(),
            session_id: 0,
            previous_session_unclean: false,
        });
    }
}
```

Re-export from `mod.rs`:

```rust
#[cfg(any(test, feature = "test-runtime"))]
pub mod testing {
    pub use super::runtime::testing::install_runtime_for_test;
}
```

In `Cargo.toml`, add `test-runtime` to the feature list (no deps):

```toml
test-runtime = []
```

- [ ] **Step 4: Make `record_resolution_miss` tolerant of `pipeline = None`**

Edit `src/telemetry/mod.rs::record_resolution_miss` so that the `if let Some(p) = ...` block is skipped when the pipeline is None. The atomic counters still increment, which is what tests assert on.

- [ ] **Step 5: Run tests**

```bash
cargo test --test telemetry_integration --features test-runtime
```
Expected: passes.

- [ ] **Step 6: Commit**

```bash
git add src/parser.rs src/telemetry/runtime.rs src/telemetry/mod.rs tests/telemetry_integration.rs Cargo.toml Cargo.lock
git commit -m "feat(telemetry): instrument parser resolution misses + fixture test"
```

---

### Task 2.4: Wire `record_parser_error`, `record_indexer_issue`, `record_handler_empty`

**Files:**
- Modify: `src/parser.rs`, `src/indexer.rs`, `src/dependencies.rs`, `src/handlers.rs`
- Modify: `src/telemetry/mod.rs` (fill in the stub bodies)

- [ ] **Step 1: Implement `record_parser_error`**

In `src/telemetry/mod.rs`, replace the stub body:

```rust
#[cfg(feature = "telemetry")]
pub fn record_parser_error(kind: ParserErrorKind, file: &std::path::Path) {
    let Some(rt) = runtime::get() else { return };
    let leaf = match kind {
        ParserErrorKind::TreeError => events::LeafKind::ParserTreeError,
        ParserErrorKind::ParseFailed => events::LeafKind::ParserParseFailed,
        ParserErrorKind::UnknownNodeKind => events::LeafKind::ParserUnknownNodeKind,
    };
    rt.counters.observe(leaf);

    let path_str = file.to_string_lossy();
    let file_hash = hash::hash_short(&rt.salt, hash::DOMAIN_FILE, path_str.as_bytes());
    let file_extension = file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let size = std::fs::metadata(file).map(|m| m.len()).unwrap_or(0);
    let file_size_bucket = match size {
        0..=1024 => events::SizeBucket::Sub1k,
        1025..=10_240 => events::SizeBucket::Sub10k,
        10_241..=102_400 => events::SizeBucket::Sub100k,
        _ => events::SizeBucket::Over100k,
    };

    let env = EventEnvelope {
        schema_version: events::SCHEMA_VERSION,
        timestamp: std::time::SystemTime::now(),
        install_id: rt.install_id.clone(),
        al_version: env!("CARGO_PKG_VERSION"),
        grammar_version: "v2",
        os: events::current_os(),
        session_id: rt.session_id,
        workspace_id: rt.workspace_id.clone(),
        event: EventKind::ParserError(ParserError {
            kind,
            node_kind_hash: None,
            file_hash,
            file_extension,
            file_size_bucket,
            error_count: 0,
            repeat_count: 0,
        }),
    };
    if let Some(p) = rt.pipeline.read().ok().and_then(|g| g.as_ref().map(|p| p.clone_sender())) {
        p.send(env);
    }
}
```

- [ ] **Step 2: Implement `record_indexer_issue`**

```rust
#[cfg(feature = "telemetry")]
pub fn record_indexer_issue(kind: IndexerIssueKind, detail_code: u16, app_id: Option<&str>) {
    let Some(rt) = runtime::get() else { return };
    let leaf = match kind {
        IndexerIssueKind::MissingDependency => events::LeafKind::IndexerMissingDependency,
        IndexerIssueKind::AppParseFailed => events::LeafKind::IndexerAppParseFailed,
        IndexerIssueKind::BrokenSymlink => events::LeafKind::IndexerBrokenSymlink,
        IndexerIssueKind::IoError => events::LeafKind::IndexerIoError,
    };
    rt.counters.observe(leaf);

    let app_id_hash = app_id.map(|a| hash::hash_identifier(&rt.salt, hash::DOMAIN_APP_ID, a));
    let env = EventEnvelope {
        schema_version: events::SCHEMA_VERSION,
        timestamp: std::time::SystemTime::now(),
        install_id: rt.install_id.clone(),
        al_version: env!("CARGO_PKG_VERSION"),
        grammar_version: "v2",
        os: events::current_os(),
        session_id: rt.session_id,
        workspace_id: rt.workspace_id.clone(),
        event: EventKind::IndexerIssue(IndexerIssue {
            kind,
            app_id_hash,
            detail_code,
        }),
    };
    if let Some(p) = rt.pipeline.read().ok().and_then(|g| g.as_ref().map(|p| p.clone_sender())) {
        p.send(env);
    }
}
```

- [ ] **Step 3: Implement `record_handler_empty` with 10% sampling**

```rust
#[cfg(feature = "telemetry")]
pub fn record_handler_empty(
    method: &'static str,
    target_object_type: ObjectType,
    target_kind: DefinitionKind,
    object_name: &str,
    procedure_name: &str,
) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SAMPLE_COUNTER: AtomicU32 = AtomicU32::new(0);
    if SAMPLE_COUNTER.fetch_add(1, Ordering::Relaxed) % 10 != 0 {
        return;
    }
    let Some(rt) = runtime::get() else { return };
    rt.counters.observe(events::LeafKind::HandlerEmpty);

    let object_hash = hash::hash_identifier(&rt.salt, hash::DOMAIN_OBJECT, object_name);
    let procedure_hash = hash::hash_identifier(&rt.salt, hash::DOMAIN_PROCEDURE, procedure_name);
    let env = EventEnvelope {
        schema_version: events::SCHEMA_VERSION,
        timestamp: std::time::SystemTime::now(),
        install_id: rt.install_id.clone(),
        al_version: env!("CARGO_PKG_VERSION"),
        grammar_version: "v2",
        os: events::current_os(),
        session_id: rt.session_id,
        workspace_id: rt.workspace_id.clone(),
        event: EventKind::HandlerEmpty(HandlerEmpty {
            method,
            target_object_type,
            target_kind,
            object_hash,
            procedure_hash,
            repeat_count: 0,
        }),
    };
    if let Some(p) = rt.pipeline.read().ok().and_then(|g| g.as_ref().map(|p| p.clone_sender())) {
        p.send(env);
    }
}
```

- [ ] **Step 4: Implement `record_session_start`**

```rust
#[cfg(feature = "telemetry")]
pub fn record_session_start(
    workspace_file_count: u32,
    dependency_count: u8,
    has_app_dependencies: bool,
) {
    let Some(rt) = runtime::get() else { return };
    rt.counters.observe(events::LeafKind::SessionStart);

    let al_file_count_bucket = match workspace_file_count {
        0..=99 => events::SizeBucket::Sub1k,
        100..=499 => events::SizeBucket::Sub10k,
        500..=1999 => events::SizeBucket::Sub100k,
        _ => events::SizeBucket::Over100k,
    };
    let env = EventEnvelope {
        schema_version: events::SCHEMA_VERSION,
        timestamp: std::time::SystemTime::now(),
        install_id: rt.install_id.clone(),
        al_version: env!("CARGO_PKG_VERSION"),
        grammar_version: "v2",
        os: events::current_os(),
        session_id: rt.session_id,
        workspace_id: rt.workspace_id.clone(),
        event: EventKind::SessionStart(SessionStart {
            workspace_file_count,
            al_file_count_bucket,
            dependency_count,
            has_app_dependencies,
            config_flags: ConfigFlags { bits: 0 },
            previous_session_unclean: rt.previous_session_unclean,
        }),
    };
    if let Some(p) = rt.pipeline.read().ok().and_then(|g| g.as_ref().map(|p| p.clone_sender())) {
        p.send(env);
    }
}
```

- [ ] **Step 5: Wire instrumentation into call sites**

Per Task 2.1's mapping:

- In `src/parser.rs`, after a tree-sitter parse that produced ERROR nodes, call `crate::telemetry::record_parser_error(ParserErrorKind::TreeError, &path)`.
- In `src/dependencies.rs` (or wherever `MissingDependency` is detected), call `crate::telemetry::record_indexer_issue(IndexerIssueKind::MissingDependency, 0, Some(&app_id))`.
- In `src/handlers.rs::incoming_calls` and `outgoing_calls`, before returning, if the result is empty, call `crate::telemetry::record_handler_empty(...)`.
- In `src/server.rs::run_server`, after initial indexing completes, call `crate::telemetry::record_session_start(file_count, dep_count, has_deps)`.

Each call site requires the same `#[cfg(feature = "telemetry")]` guard.

- [ ] **Step 6: Build matrix**

```bash
cargo build
cargo build --no-default-features
cargo test --features test-runtime
```

Expected: green.

- [ ] **Step 7: Commit**

```bash
git add src/telemetry/mod.rs src/parser.rs src/indexer.rs src/dependencies.rs src/handlers.rs src/server.rs
git commit -m "feat(telemetry): instrument parser/indexer/handlers/lifecycle"
```

---

### Task 2.5: Phase 2 wrap

- [ ] **Step 1: Run privacy lint**

```bash
cargo test --test telemetry_privacy_lint
```
Expected: passes.

- [ ] **Step 2: Build matrix**

```bash
cargo build && cargo build --no-default-features && cargo test --features test-runtime && cargo clippy --all-targets --all-features -- -D warnings
```

- [ ] **Step 3: Tag**

```bash
git tag -a phase-2-instrumentation -m "Telemetry Phase 2 instrumentation complete"
```

---

## Phase 3 — Disclosure & release (PR #4)

**Phase goal:** README disclosure, schema documentation, CHANGELOG, build-time connection-string baking, smoke test, release tag.

### Task 3.1: build.rs to bake connection string

**Files:**
- Create: `build.rs`

- [ ] **Step 1: Write build.rs**

Create `build.rs` at the repo root:

```rust
fn main() {
    println!("cargo:rerun-if-env-changed=AL_CH_TELEMETRY_CONNECTION_STRING");
    if let Ok(cs) = std::env::var("AL_CH_TELEMETRY_CONNECTION_STRING") {
        println!("cargo:rustc-env=AL_CH_TELEMETRY_CONNECTION_STRING={}", cs);
    }
}
```

- [ ] **Step 2: Verify**

```bash
cargo build
AL_CH_TELEMETRY_CONNECTION_STRING="InstrumentationKey=test;IngestionEndpoint=https://x/" cargo build
```
Expected: both succeed; the second invocation embeds the env var.

- [ ] **Step 3: Commit**

```bash
git add build.rs
git commit -m "feat(telemetry): bake AL_CH_TELEMETRY_CONNECTION_STRING via build.rs"
```

---

### Task 3.2: README "Telemetry" section

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Add the section above "Installation"**

Insert into `README.md` immediately above the existing "Installation" heading (or at the top if no Installation section exists yet):

```markdown
## Telemetry

`al-call-hierarchy` ships with **anonymous, opt-out failure-diagnostics telemetry** so the maintainer can find resolution gaps that real-world AL projects hit. **No raw identifiers, file paths, or source code leave your machine.** All AL identifier names are hashed with a per-installation random 32-byte salt that stays on your machine; the maintainer sees only structural fingerprints (object types, failure categories, tree-sitter shapes) plus salted hashes.

**What's collected:** see [docs/telemetry.md](docs/telemetry.md). **Source code:** [src/telemetry/](src/telemetry/) — auditable in one directory.

**Telemetry is OFF by default in:**
- Debug builds (`cargo build` without `--release`)
- Test runs (`cargo test`)
- CI environments (CI, GITHUB_ACTIONS, GITLAB_CI, etc.)

**Three ways to disable** (any wins):

1. Environment variable: `AL_CH_TELEMETRY=0` or `DO_NOT_TRACK=1`
2. CLI flag: `al-call-hierarchy --no-telemetry`
3. Config file `~/.al-call-hierarchy/config.json`:
   ```json
   { "telemetry": { "enabled": false } }
   ```

To inspect what telemetry has been sent in the current session, send the LSP request `al-call-hierarchy/telemetryStatus` (also logged at startup).
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs(telemetry): add README disclosure section"
```

---

### Task 3.3: docs/telemetry.md schema reference

**Files:**
- Create: `docs/telemetry.md`

- [ ] **Step 1: Write the schema doc**

Create `docs/telemetry.md` with one section per event type, showing every attribute name and what it represents. Cross-reference `src/telemetry/events.rs` line numbers.

(Use the spec's §5 "Data Model" as the source — paraphrase for users, do not copy verbatim. The doc should answer "what does each field mean for a user reading their App Insights export?")

- [ ] **Step 2: Commit**

```bash
git add docs/telemetry.md
git commit -m "docs(telemetry): user-facing schema reference"
```

---

### Task 3.4: Smoke test procedure doc

**Files:**
- Create: `docs/telemetry-smoke-test.md`

- [ ] **Step 1: Write the procedure**

Create `docs/telemetry-smoke-test.md`:

```markdown
# Telemetry Smoke Test (manual, pre-release)

Run before publishing each release.

## Prerequisites

- App Insights resource with connection string
- An AL workspace fixture: `tests/fixtures/telemetry/unresolved_app_dep/`

## Steps

1. Build a release binary with the connection string baked in:
   ```bash
   AL_CH_TELEMETRY_CONNECTION_STRING="InstrumentationKey=...;IngestionEndpoint=..." \
     cargo build --release
   ```

2. Run the binary against the fixture in CLI mode:
   ```bash
   target/release/al-call-hierarchy --project tests/fixtures/telemetry/unresolved_app_dep --no-lsp
   ```

3. Wait 60s, then query App Insights:
   ```kusto
   traces
   | where timestamp > ago(5m)
   | where customDimensions["telemetry.alch.schema_version"] == 1
   | summarize count() by message
   ```

   Expected: at least one `resolution.miss` and one `session.start` row.

4. Verify hashes look like 32-char hex (ResolutionMiss) or 16-char hex (install/workspace IDs):
   ```kusto
   traces
   | where message == "resolution.miss"
   | extend obj = tostring(customDimensions["telemetry.alch.object_hash"])
   | project obj, strlen(obj)
   ```

5. Verify no field contains an obvious AL identifier (e.g., "Customer", "PostInvoice"):
   ```kusto
   traces
   | where timestamp > ago(5m)
   | where customDimensions has "PostInvoice" or customDimensions has "Customer"
   | count
   ```
   Expected: 0 rows.

## On failure

Do NOT release. File an issue, add the failing dimension to the privacy lint, fix the leak.
```

- [ ] **Step 2: Commit**

```bash
git add docs/telemetry-smoke-test.md
git commit -m "docs(telemetry): manual smoke test procedure"
```

---

### Task 3.5: CHANGELOG and version bump

**Files:**
- Modify: `CHANGELOG.md`
- Modify: `Cargo.toml` (version 0.6.0 → 0.7.0)

- [ ] **Step 1: Add CHANGELOG entry**

Insert at the top of `CHANGELOG.md` under a new `## [0.7.0]` section:

```markdown
## [0.7.0] - <release date>

### Added
- Anonymous, opt-out failure-diagnostics telemetry (App Insights).
  - Captures resolution misses, parser errors, indexer issues, and handler outcomes.
  - All AL identifier names hashed with a per-installation 32-byte salt that stays local.
  - Three disable mechanisms: `DO_NOT_TRACK=1`, `--no-telemetry`, `~/.al-call-hierarchy/config.json` `telemetry.enabled=false`.
  - Off by default in debug, test, and CI builds.
  - LSP request `al-call-hierarchy/telemetryStatus` for runtime introspection.
  - Schema documented in `docs/telemetry.md`.
```

- [ ] **Step 2: Bump version**

Edit `Cargo.toml`:
```toml
version = "0.7.0"
```

- [ ] **Step 3: Verify build matrix once more**

```bash
cargo build --release
cargo test --features test-runtime
cargo clippy --all-targets --all-features -- -D warnings
```

- [ ] **Step 4: Run smoke test (Task 3.4 procedure)**

Manual. If smoke test fails, do not proceed.

- [ ] **Step 5: Commit and tag**

```bash
git add CHANGELOG.md Cargo.toml Cargo.lock
git commit -m "chore: release 0.7.0 with telemetry"
git tag -a v0.7.0 -m "Release 0.7.0 — anonymous failure-diagnostics telemetry"
```

---

## Self-Review (run before handing the plan back)

### Spec coverage

- §1 Problem & Goal — addressed across all phases.
- §2 Constraints & Decisions — embodied in tasks (hot path budget = bench, hashing = Task 0.4, opt-out = Task 0.7 + Task 0.10 + Task 1.6, App Insights = Task 0.5 spike + Task 1.5).
- §3 Architecture — implemented in Tasks 1.4/1.5/1.6.
- §4 Module Layout — every file in the table maps to a task.
- §5 Data Model — Task 0.8 (events) + Task 1.5 (event_attrs) + privacy lint Task 0.12.
- §6 Pipeline — Tasks 1.2/1.3/1.4/1.5/1.6.
- §7 Configuration — Task 0.9 + Task 1.6 (resolution order via consent::Inputs).
- §8 Privacy — Tasks 0.4/0.5/0.6/0.12 + README Task 3.2.
- §9 Error Handling — handled inline in Tasks 0.5/0.6/1.5; explicit `WARN once` is left as an implementation detail under each module's `log::warn!`.
- §10 Testing — Tasks 0.4/0.5/0.6/0.7/0.8/1.2/1.3/1.4/1.5/1.10/2.3/2.5; bench Task 1.9; smoke Task 3.4.
- §11 Rollout — phase boundaries (Tasks 0.13/0.5.3/1.11/2.5/3.5) match spec phases.
- §12 Open Decision — Phase 0.5 spike (Task 0.5.3) is the gate.
- §13 Dependencies — Task 1.1 promotes deps from spike to runtime feature-gated.

### Placeholder scan

- All steps include the actual code or commands. No "TBD", no "implement later".
- Task 2.1 and parts of Task 2.4 instruct the engineer to identify call sites in existing code (since precise line numbers depend on the current working tree). This is intentional: the plan provides the instrumentation pattern in full and the engineer applies it at sites the spec defines (parser, indexer, handlers, server). This is a known judgement call; not a placeholder.
- Step 1 of Task 3.3 (docs/telemetry.md) is described rather than written verbatim — the doc is user-facing prose derived from the spec; engineer paraphrases. This is acceptable for a docs task.

### Type consistency

- `LeafKind` (Task 0.8) and `Counters` (Task 1.2) agree on the 14-element array.
- `EventKind` variants in Task 0.8 match the `match` arms in Task 1.5 (`exporter`/`events_attrs`).
- `Pipeline::send` (Task 1.4) signature matches `record_resolution_miss` callsite (Task 1.7).
- `TelemetryInputs` (Task 1.6) consumed in `init` matches the construction in `server.rs`.
- `record_resolution_miss` parameter `CallContext<'_>` is used identically in Tasks 1.7 and 2.3.
- `hash_identifier` returns `String` (32 hex), `hash_short` returns `String` (16 hex) — uses match the consumers in `record_*` and `runtime`.

No inconsistencies found.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-06-telemetry.md`. Two execution options:

1. **Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration.
2. **Inline Execution** — Execute tasks in this session using `executing-plans`, batch execution with checkpoints.

Which approach?

