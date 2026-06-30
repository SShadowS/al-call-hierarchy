//! 1B.3b Task 1: domain-separated, versioned, stable anonymization for the
//! committed CDO-derived goldens (`tests/goldens/semantic-edges/cdo-*.json`).
//!
//! # Why anonymize
//!
//! CDO is a real customer workspace. A committed per-site golden's site keys
//! (file path, routine name, object name/number) and target identities leak
//! proprietary names if stored as plaintext. [`anon`] replaces every
//! identifying string with a stable, deterministic, opaque [`AnonId`] so the
//! committed golden encodes GRAPH STRUCTURE (which site resolves to which
//! target, which dispatch kind, which classification) without the names. The
//! same function re-anonymizes the FRESH side at audit time with the SAME key,
//! so the diff aligns on opaque ids — a regression shows as a reviewable
//! anonymized ±edge diff.
//!
//! # Governance: HMAC with a non-committed key, not a fixed committed salt
//!
//! A fixed, committed salt is deterministic and adequate purely for DIFFING
//! (stable id ↔ stable id across runs) but is weak against an adversary: AL
//! object/procedure names are drawn from a small, guessable vocabulary
//! (`OnInsert`, `PostInvoice`, `"Sales Header"`, …), so a fixed public salt
//! lets anyone dictionary-attack the committed golden back to plaintext.
//!
//! **Decision: HMAC-SHA256 keyed by [`ANON_KEY_ENV`], a NON-COMMITTED secret.**
//! The key lives in the gated/internal CDO runner's secret store (and a
//! developer's local shell when reproducing locally with real CDO access) —
//! never in the repo. Both the dev-mint tool (writes the committed golden) and
//! the runtime audit (anonymizes the fresh side to compare against it) read the
//! SAME env var, so the two sides agree without the key ever touching git.
//!
//! When [`ANON_KEY_ENV`] is unset — i.e. every `cargo test --workspace` run
//! without CDO access, including public CI — [`anon`] falls back to
//! [`TEST_KEY`], a key that IS committed (it has to be, to make the fallback
//! deterministic). This is safe specifically because public CI never has
//! `CDO_WS` either, so it never mints or audits real CDO data with this
//! fallback; the fallback only has to make [`anon`]'s own unit tests and the
//! synthetic, non-proprietary fixtures deterministic.
//!
//! # Domain separation
//!
//! [`anon`] takes a `domain` tag as well as the value to hash:
//! `anon(domain, s) = HMAC-SHA256(key, domain || 0x00 || s)`, truncated to 16
//! bytes (128 bits — collision-safe for the ~13k-site CDO golden, keeps the
//! minified JSON compact). The SAME plaintext string hashed under two
//! different domains yields two DIFFERENT, uncorrelatable ids. This matters
//! because several plaintext roles can share literal values — e.g. a call
//! site's caller object name and a resolved target's object name can be the
//! identical string ("Codeunit 50100 runs Codeunit 50100"); without domain
//! separation the same hash would appear in both the site-identity and the
//! target-identity positions, leaking a correlation the anonymization is
//! supposed to remove. Each domain is `:v1`-suffixed so a future scheme change
//! is reviewable as a version bump, not a silent reinterpretation of old ids.
//!
//! Fixed domains (shared by the dev-mint tool and every runtime audit):
//! - [`SITE_DOMAIN_V1`] — regular call-site identity fields (the Member/
//!   Interface semantic golden, `cdo-anon.json`).
//! - [`TARGET_DOMAIN_V1`] — resolved target identity fields, shared by every
//!   golden (a "target" means the same thing — a resolved object+routine —
//!   regardless of which golden it appears in).
//! - [`TRIGGER_OP_DOMAIN_V1`] — `ImplicitTrigger` site identity fields
//!   (`cdo-trigger-anon.json`). Kept separate from `site:v1` even though the
//!   site SHAPE is identical, because the underlying identity is a synthesized
//!   `PRecordOperation` site, not a real call site — collapsing the two
//!   domains would let an attacker correlate "this record-op text equals that
//!   call-site text" across categories that are not actually comparable.
//! - [`EVENT_PAIR_DOMAIN_V1`] — `EventFlow` publisher/event-name/subscriber
//!   identity fields (`cdo-event-anon.json`).
//!
//! # The re-hash-don't-decrypt principle
//!
//! [`AnonId`] is one-way: there is no "de-anonymize this id" operation here.
//! Every consumer that needs to test "does this committed opaque id correspond
//! to plaintext value X?" must hold a CANDIDATE plaintext X (e.g. from a live,
//! local re-resolution against the real CDO source) and re-hash it with
//! [`anon`] under the same domain, then compare ids for equality. This is how
//! the genuine-wrong manifest membership check and the interface-implements
//! adjudication both work post-anonymization (see `semantic_golden.rs`) — they
//! never need to invert a committed id, only confirm a guess.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Env var carrying the keyed-hash secret used by [`anon`]. Set on the gated/
/// internal CDO runner (and a developer's local shell, when reproducing with
/// real CDO access) to a private value; NEVER committed.
pub const ANON_KEY_ENV: &str = "CDO_ANON_KEY";

/// Domain for regular call-site identity fields — the Member/Interface
/// semantic golden (`cdo-anon.json`). See the module docs' "Domain
/// separation" section.
pub const SITE_DOMAIN_V1: &str = "site:v1";

/// Domain for resolved-target identity fields, shared across every golden.
pub const TARGET_DOMAIN_V1: &str = "target:v1";

/// Domain for `ImplicitTrigger` (native `PRecordOperation`-keyed) site
/// identity fields (`cdo-trigger-anon.json`).
pub const TRIGGER_OP_DOMAIN_V1: &str = "trigger-op:v1";

/// Domain for `EventFlow` publisher/event-name/subscriber identity fields
/// (`cdo-event-anon.json`).
pub const EVENT_PAIR_DOMAIN_V1: &str = "event-pair:v1";

/// Deterministic fallback key used when [`ANON_KEY_ENV`] is unset — i.e.
/// every `cargo test --workspace` run without CDO access, including public
/// CI. INTENTIONALLY committed/public: it exists only to make [`anon`]'s unit
/// tests and the L3-independent synthetic fixtures deterministic. It must
/// NEVER anonymize real CDO data — the dev-mint tool and the gated CDO audit
/// MUST run with [`ANON_KEY_ENV`] set to a real secret on the internal
/// runner, or the resulting committed golden's ids are dictionary-attackable
/// (see the module docs' governance section).
const TEST_KEY: &[u8] = b"al-call-hierarchy/1B.3b/anon-test-key/NOT-FOR-CDO-DATA/v1";

/// Number of HMAC-SHA256 output bytes kept per [`AnonId`] (truncated from the
/// full 32-byte digest). 128 bits is collision-safe for the ~13k-site CDO
/// golden and keeps the minified JSON compact.
const ID_BYTES: usize = 16;

/// An opaque, deterministic, domain-separated identifier produced by [`anon`].
///
/// Hex-encoded (lowercase), `serde(transparent)` so it round-trips as a bare
/// JSON string in the committed goldens (e.g. `"3f9a2b71c44e08d5..."`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AnonId(pub String);

impl std::fmt::Display for AnonId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Resolve the HMAC key: [`ANON_KEY_ENV`] when set and non-empty, else
/// [`TEST_KEY`]. See the module docs' governance section.
fn anon_key() -> Vec<u8> {
    match std::env::var(ANON_KEY_ENV) {
        Ok(k) if !k.is_empty() => k.into_bytes(),
        _ => TEST_KEY.to_vec(),
    }
}

/// Domain-separated, versioned, stable (deterministic) anonymization.
///
/// `anon(domain, s) = HMAC-SHA256(key, domain || 0x00 || s)[..16]`, hex-encoded.
/// `key` is [`ANON_KEY_ENV`] when set, else the committed [`TEST_KEY`] (see the
/// module docs). Deterministic: the same `(domain, s)` pair under the same key
/// always yields the same [`AnonId`] — across processes, across runs, at both
/// mint time and audit time.
///
/// The `domain` value is expected to be one of the `*_DOMAIN_V1` constants in
/// this module (or a future versioned successor); it is not itself validated
/// here so callers remain free to add new domains without touching this
/// function.
#[must_use]
pub fn anon(domain: &str, s: &str) -> AnonId {
    let key = anon_key();
    // `Hmac::new_from_slice` only fails for algorithms with a fixed/maximum key
    // size; HMAC-SHA256 accepts any key length, so this cannot fail.
    let mut mac =
        <HmacSha256 as Mac>::new_from_slice(&key).expect("HMAC-SHA256 accepts any key length");
    mac.update(domain.as_bytes());
    mac.update(&[0u8]); // domain/input separator — `:v1` domain tags never contain NUL.
    mac.update(s.as_bytes());
    let digest = mac.finalize().into_bytes();
    let hex: String = digest[..ID_BYTES]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    AnonId(hex)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same `(domain, input)` pair → same `AnonId`, repeatedly. Determinism is
    /// the entire point: the dev-mint tool and the runtime audit must agree.
    #[test]
    fn same_domain_and_input_yields_same_id() {
        let a = anon(SITE_DOMAIN_V1, "Codeunit 50100 PostInvoice");
        let b = anon(SITE_DOMAIN_V1, "Codeunit 50100 PostInvoice");
        assert_eq!(a, b);
        // And a third call, well after the first two, to rule out any
        // process-local mutable state leaking into the result.
        let c = anon(SITE_DOMAIN_V1, "Codeunit 50100 PostInvoice");
        assert_eq!(a, c);
    }

    /// The SAME plaintext under DIFFERENT domains must yield DIFFERENT ids —
    /// the core domain-separation / no-cross-namespace-collision property.
    #[test]
    fn same_input_under_different_domains_yields_different_ids() {
        let s = "Codeunit 50100";
        let domains = [
            SITE_DOMAIN_V1,
            TARGET_DOMAIN_V1,
            TRIGGER_OP_DOMAIN_V1,
            EVENT_PAIR_DOMAIN_V1,
        ];
        let ids: Vec<AnonId> = domains.iter().map(|d| anon(d, s)).collect();
        for i in 0..ids.len() {
            for j in (i + 1)..ids.len() {
                assert_ne!(
                    ids[i], ids[j],
                    "domains {:?} and {:?} collided on input {s:?}: {ids:?}",
                    domains[i], domains[j]
                );
            }
        }
    }

    /// Different inputs under the SAME domain must (overwhelmingly) yield
    /// different ids — sanity-checks that the hash actually depends on `s`,
    /// not just `domain`.
    #[test]
    fn different_inputs_under_same_domain_yield_different_ids() {
        let a = anon(SITE_DOMAIN_V1, "ProcA");
        let b = anon(SITE_DOMAIN_V1, "ProcB");
        assert_ne!(a, b);
    }

    /// Domain values are NOT just concatenated with `s` — a naive
    /// `domain + s` scheme would collide `("site:v", "1x")` with
    /// `("site:v1", "x")`. The NUL separator must prevent this class of
    /// boundary-shift collision.
    #[test]
    fn domain_value_boundary_is_not_naively_concatenable() {
        let a = anon("d", "ab");
        let b = anon("da", "b");
        assert_ne!(a, b, "domain/input boundary must be unambiguous");
    }

    /// `AnonId`s are lowercase hex of the expected truncated length (16 bytes
    /// → 32 hex chars), and visibly opaque (not a recognizable transform of
    /// the input — i.e. not a no-op/identity stand-in).
    #[test]
    fn id_shape_is_fixed_length_lowercase_hex() {
        let id = anon(SITE_DOMAIN_V1, "anything");
        assert_eq!(id.0.len(), ID_BYTES * 2);
        assert!(
            id.0.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "AnonId must be lowercase hex: {:?}",
            id.0
        );
        assert_ne!(id.0, "anything");
    }

    /// Anonymizing a small synthetic "workspace" (a handful of site→target
    /// records) twice in a row and serializing both runs to minified JSON
    /// must produce BYTE-IDENTICAL output — the anon-determinism property the
    /// dev-mint tool's "verify frozen==live" step (1B.3b Task 1 Step 5)
    /// depends on. This does not touch CDO/L3 — it is a pure exercise of
    /// `anon` + serde, standing in for the dev tool's mint pipeline.
    #[test]
    fn two_mints_of_a_synthetic_workspace_are_byte_identical_minified_json() {
        #[derive(Serialize)]
        struct SyntheticEntry {
            site: AnonId,
            edge_kind: u8,
            targets: Vec<AnonId>,
        }

        fn mint() -> String {
            let sites = [
                (
                    "Codeunit 50100 PostInvoice#L42",
                    0u8,
                    vec!["Codeunit 50101 Helper#Run"],
                ),
                (
                    "Codeunit 50100 PostInvoice#L43",
                    1u8,
                    vec!["Codeunit 50102 Other#Post"],
                ),
                (
                    "Page 50200 Card#L10",
                    2u8,
                    vec![
                        "Table 50300 MyTable#OnInsert",
                        "TableExt 50301 MyExt#OnInsert",
                    ],
                ),
            ];
            let mut entries: Vec<SyntheticEntry> = sites
                .iter()
                .map(|(site, kind, targets)| SyntheticEntry {
                    site: anon(SITE_DOMAIN_V1, site),
                    edge_kind: *kind,
                    targets: targets.iter().map(|t| anon(TARGET_DOMAIN_V1, t)).collect(),
                })
                .collect();
            entries.sort_by(|a, b| a.site.cmp(&b.site));
            serde_json::to_string(&entries).expect("minified serialize")
        }

        let first = mint();
        let second = mint();
        assert_eq!(
            first, second,
            "anonymizing the same synthetic workspace twice must be byte-identical"
        );
    }

    /// `AnonId` round-trips through serde as a bare JSON string (not an
    /// object wrapper) — required for the committed goldens to stay compact
    /// and for `cdo-deanon-map.json` (`AnonId → plaintext`) to use it as a
    /// natural JSON object key string.
    #[test]
    fn anon_id_serializes_as_bare_json_string() {
        let id = anon(SITE_DOMAIN_V1, "x");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{}\"", id.0));
        let back: AnonId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    /// The committed fallback [`TEST_KEY`] path is exercised whenever
    /// [`ANON_KEY_ENV`] is unset — confirm `anon_key()` does not panic and
    /// produces the documented fallback when the env var is absent. Run
    /// serially with the env-var-set test would race; this test only reads
    /// state when the var happens to be unset, which is the default for
    /// `cargo test --workspace` (no CDO secret configured).
    #[test]
    fn anon_key_falls_back_when_env_var_unset() {
        // Best-effort: only assert the fallback path when the var is not
        // already set in this process's environment (avoids flaking under
        // `cargo test -- --test-threads=1` with a real secret exported).
        if std::env::var(ANON_KEY_ENV).is_err() {
            assert_eq!(anon_key(), TEST_KEY.to_vec());
        }
    }
}
