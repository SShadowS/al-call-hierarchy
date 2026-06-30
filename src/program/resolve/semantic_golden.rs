//! 1B.3a Task 4 (L3-validated semantic edge golden + route-applicability
//! contract) + 1B.3b Task 1 (committed ANONYMIZED frozen goldens + the
//! load-frozen audits + the `ENFORCE_CDO_WS` guard).
//!
//! # Golden floor
//!
//! [`mint_l3_validated_golden`]/[`mint_l3_trigger_golden`] capture the
//! L3-oracle target set per call site into a [`SemanticGolden`] (a sorted
//! list keyed by column-ignoring [`GoldenSiteKey`]).
//! [`assert_against_semantic_golden`] compares a fresh canonical edge batch
//! against a (plaintext, in-repo-fixture-scale) golden and classifies every
//! site into: `match`, `fresh_wrong`, `fresh_missing`, `fresh_extra`,
//! `fresh_novel`, or `golden_missing`.
//!
//! # The critical invariant
//!
//! **`fresh_wrong.is_empty()`** — fresh must never confidently emit a target
//! that L3 says is wrong.  A per-site Histogram cannot catch this: it can
//! count "resolved" or "unknown" but cannot tell you WHICH target was chosen.
//! This golden does.
//!
//! # 1B.3b: committed, anonymized, frozen — no live L3 in the gate path
//!
//! The CDO-scale golden is too large and too proprietary to mint live on
//! every run (and CDO is being retired as a live dependency of the gate
//! module — see the 1B.3b plan). Instead: [`mint_l3_validated_golden`] /
//! [`mint_l3_trigger_golden`] / `differential::project_l3_event_rows` run
//! ONCE, on a dev machine with CDO access, via the dev-mint tool
//! (`src/bin/mint-goldens.rs`, OUTSIDE `src/program/resolve`). The tool
//! ANONYMIZES every identifying string (via [`anon::anon`] — see that
//! module's docs for the full domain-separation + HMAC-governance writeup)
//! and writes the result to three COMMITTED files under
//! `tests/goldens/semantic-edges/`: `cdo-anon.json` (Member/Interface),
//! `cdo-trigger-anon.json` (ImplicitTrigger), `cdo-event-anon.json`
//! (EventFlow). [`run_cdo_semantic_audit`]/[`run_cdo_trigger_audit`]/
//! [`run_cdo_event_audit`] LOAD these committed goldens and anonymize the
//! FRESH side with the SAME function at audit time — `engine::l3` is NOT
//! imported by any of the three `run_cdo_*_audit` functions; the gate module
//! still depends on it only through the sanctioned mint functions above
//! (removed entirely in 1B.3b Task 3).
//!
//! # Route-applicability contract
//!
//! [`route_applicability`] verifies the structural witness↔evidence contract
//! on every route and delegates the ABI ingestion check to
//! [`abi_ingestion_integrity`].
//!
//! # CDO audits
//!
//! [`run_cdo_semantic_audit`]/[`run_cdo_trigger_audit`]/[`run_cdo_event_audit`]
//! run the load-frozen comparison over a real workspace (env-gated; the
//! caller checks `CDO_WS` and applies the `ENFORCE_CDO_WS` hard-fail guard —
//! see `tests/program_resolve_harness.rs`).

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::program::node::ObjKey;
use crate::program::node_extract::ObjectNode;
use crate::program::resolve::abi_check::{
    abi_ingestion_integrity, build_raw_abi_index_from_snapshot,
};
use crate::program::resolve::anon::{self, AnonId};
use crate::program::resolve::differential::{
    CanonicalEdge, CanonicalEventRow, CanonicalKey, CanonicalTarget, project_fresh,
    project_fresh_event_rows, project_l3, project_l3_implicit_trigger_in_scope,
    witness_contract_holds,
};
use crate::program::resolve::edge::{Edge, EdgeKind};

// ---------------------------------------------------------------------------
// Column-ignoring site key (serde-able)
// ---------------------------------------------------------------------------

/// Serde-able, column-ignoring key for one call site in the semantic golden.
///
/// Omits the column offset because L3 uses UTF-16 columns while the fresh
/// side uses byte columns — they agree on ASCII but may differ by a small
/// delta on non-ASCII identifiers.  The strong key `(unit, line, callee_fp)`
/// mirrors the invariant used by [`crate::program::resolve::differential::match_sites`].
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GoldenSiteKey {
    pub from_app_guid: String,
    pub from_object_kind: String,
    pub from_object_lc: String,
    pub from_routine_lc: String,
    /// `EdgeKind` discriminant: 0=Call, 1=Run, 2=ImplicitTrigger, 3=EventFlow.
    pub edge_kind: u8,
    pub unit: String,
    pub line: u32,
    pub callee_fp: u64,
}

/// Serde-able mirror of
/// [`CanonicalTarget`][crate::program::resolve::differential::CanonicalTarget].
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GoldenTarget {
    pub kind: u8,
    pub app: Option<String>,
    pub object_lc: String,
    pub routine_lc: Option<String>,
}

// ---------------------------------------------------------------------------
// SemanticGolden
// ---------------------------------------------------------------------------

/// One entry in the semantic golden: a call-site key paired with the set of
/// targets the L3 oracle resolved for that site.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GoldenEntry {
    pub site: GoldenSiteKey,
    /// Targets L3 resolved for this site.  Empty when L3 could not resolve.
    pub targets: BTreeSet<GoldenTarget>,
}

/// The L3-validated semantic golden: a sorted list of (site, targets) pairs.
///
/// Stored as a `Vec` so serde_json can serialize it (JSON maps require string
/// keys; `GoldenSiteKey` is a struct).  The list is always sorted by `site`
/// for determinism and binary-search lookups.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SemanticGolden {
    pub entries: Vec<GoldenEntry>,
}

impl SemanticGolden {
    /// Build from a `BTreeMap` (already sorted, so insertion order is preserved).
    fn from_map(map: std::collections::BTreeMap<GoldenSiteKey, BTreeSet<GoldenTarget>>) -> Self {
        SemanticGolden {
            entries: map
                .into_iter()
                .map(|(site, targets)| GoldenEntry { site, targets })
                .collect(),
        }
    }

    /// Lookup targets for `key` (binary search on sorted `entries`).
    fn get(&self, key: &GoldenSiteKey) -> Option<&BTreeSet<GoldenTarget>> {
        self.entries
            .binary_search_by(|e| e.site.cmp(key))
            .ok()
            .map(|i| &self.entries[i].targets)
    }
}

// ---------------------------------------------------------------------------
// 1B.3b Task 1: anonymized frozen-golden types (committed, no plaintext)
//
// See `anon.rs`'s module docs for the full governance writeup (HMAC vs salt,
// domain separation, the re-hash-don't-decrypt principle). In short:
// [`AnonSiteKey`]/[`AnonTarget`] are the SAME shape as [`GoldenSiteKey`]/
// [`GoldenTarget`] with every identifying string field replaced by an
// [`AnonId`]; non-sensitive labels (`from_object_kind`, `edge_kind`, `line`,
// `kind`) stay in CLEARTEXT so an anonymized diff still has semantic anchors.
// These are what gets WRITTEN to the committed `cdo-anon.json` /
// `cdo-trigger-anon.json` (same shape, different `site_domain` — see
// [`anon::SITE_DOMAIN_V1`] vs [`anon::TRIGGER_OP_DOMAIN_V1`]).
// ---------------------------------------------------------------------------

/// Schema version stamped into every anonymized committed golden
/// (`cdo-anon.json` / `cdo-trigger-anon.json` / `cdo-event-anon.json`). Bump
/// when the anonymization scheme or a golden's field shape changes; the
/// public-CI metadata-validation test asserts every committed golden carries
/// this value.
///
/// Bumped 1 -> 2 by the 1B.3b Task 1 fix: switched [`anon::anon`]'s key from a
/// lost session-local `CDO_ANON_KEY` to the fixed, committed `ANON_SALT` (see
/// `anon.rs`'s module docs), which changes every emitted [`AnonId`] for the
/// same plaintext, AND added the [`MintMetadata`] field to both
/// [`AnonSemanticGolden`] and [`AnonEventGolden`] (a golden's field shape
/// change).
pub const ANON_GOLDEN_SCHEMA_VERSION: u32 = 2;

/// Mint-time provenance metadata stamped into every committed golden (1B.3b
/// Task 1 fix, Fix 4): the CDO workspace's git HEAD SHA and dirty state at
/// mint time, captured by [`workspace_git_info`]. Audit time re-probes the
/// CURRENT workspace and WARNS (does not fail — drift is operational, not a
/// correctness signal) when it differs from the stamp; see
/// `run_cdo_semantic_audit`/`run_cdo_trigger_audit`/`run_cdo_event_audit`'s
/// drift-warning step. `#[serde(default)]` on both fields so a golden minted
/// before this field existed (or from a non-git workspace export) still
/// deserializes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MintMetadata {
    /// `git -C <CDO_WS> rev-parse HEAD` at mint time. `None` when the
    /// workspace isn't inside a git repo, `git` isn't on `PATH`, or the
    /// command failed — this is best-effort provenance, never a hard
    /// requirement (mint/audit must still work against a non-git workspace
    /// export).
    #[serde(default)]
    pub workspace_git_sha: Option<String>,
    /// `true` when `git -C <CDO_WS> status --porcelain` produced non-empty
    /// output at mint time (uncommitted changes present). `None` when the
    /// git probe failed (same best-effort caveat as `workspace_git_sha`).
    #[serde(default)]
    pub workspace_dirty: Option<bool>,
}

/// Probe `workspace_root`'s git HEAD SHA + dirty state via the `git` CLI
/// (1B.3b Task 1 fix, Fix 4). Best-effort: returns `(None, None)` fields when
/// `workspace_root` isn't inside a git repo, `git` isn't on `PATH`, or either
/// command fails — this is provenance metadata, not a hard requirement. Used
/// by the dev-mint tool (to STAMP [`MintMetadata`] at mint time) and by the
/// `run_cdo_*_audit` functions (to compare the CURRENT workspace against the
/// loaded golden's stamp and warn on drift).
#[must_use]
pub fn workspace_git_info(workspace_root: &Path) -> (Option<String>, Option<bool>) {
    let sha = std::process::Command::new("git")
        .arg("-C")
        .arg(workspace_root)
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let dirty = std::process::Command::new("git")
        .arg("-C")
        .arg(workspace_root)
        .arg("status")
        .arg("--porcelain")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| !s.trim().is_empty());

    (sha, dirty)
}

/// Emit a `WARNING` on stderr when the CURRENT `workspace_root`'s git SHA/
/// dirty state differs from `stamped` (the golden's mint-time
/// [`MintMetadata`]). Never fails the audit — drift is operational (a
/// developer pointing `CDO_WS` at a workspace that has moved on since the
/// last mint), not a resolver regression; see [`MintMetadata`]'s doc comment.
fn warn_on_workspace_drift(stamped: &MintMetadata, workspace_root: &Path) {
    let (current_sha, current_dirty) = workspace_git_info(workspace_root);
    if current_sha != stamped.workspace_git_sha || current_dirty != stamped.workspace_dirty {
        eprintln!(
            "WARNING: CDO workspace drifted from mint-time SHA {:?} (dirty={:?}); \
             current SHA {:?} (dirty={:?}). Audit diffs may reflect workspace drift, \
             not engine regressions — re-mint to advance the pin (see \
             src/bin/mint-goldens.rs).",
            stamped.workspace_git_sha, stamped.workspace_dirty, current_sha, current_dirty,
        );
    }
}

/// Anonymized, serde-able mirror of [`GoldenSiteKey`]. The four identifying
/// string fields (`from_app_guid`, `from_object_lc`, `from_routine_lc`,
/// `unit`) and the `callee_fp` fingerprint are EACH individually hashed via
/// [`anon::anon`] under `site_domain`. `from_object_kind` (an object-type
/// category, e.g. `"codeunit"`), `edge_kind` (the `EdgeKind` discriminant),
/// and `line` (a bare source line number, meaningless without the now-hashed
/// `unit`) are non-sensitive and stay CLEARTEXT.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AnonSiteKey {
    pub from_app_id: AnonId,
    pub from_object_kind: String,
    pub from_object_id: AnonId,
    pub from_routine_id: AnonId,
    pub edge_kind: u8,
    pub unit_id: AnonId,
    pub line: u32,
    pub callee_id: AnonId,
}

/// Anonymized, serde-able mirror of [`GoldenTarget`]. `kind` (the object-kind
/// tag, or the 254/255 AbiSymbol/Builtin sentinels) is a non-sensitive label
/// kept CLEARTEXT; `app`+`object_lc` are combined into one [`AnonId`]
/// (app-scoped object identity) and `routine_lc` is hashed separately — both
/// under [`anon::TARGET_DOMAIN_V1`], shared by every golden (a "target" means
/// the same thing regardless of which golden it appears in).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AnonTarget {
    pub kind: u8,
    pub object_id: AnonId,
    pub routine_id: Option<AnonId>,
}

/// One entry in an anonymized golden: an [`AnonSiteKey`] paired with its
/// anonymized target set.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnonGoldenEntry {
    pub site: AnonSiteKey,
    pub targets: BTreeSet<AnonTarget>,
}

/// The committed anonymized golden shape shared by `cdo-anon.json`
/// (`site_domain = `[`anon::SITE_DOMAIN_V1`]) and `cdo-trigger-anon.json`
/// (`site_domain = `[`anon::TRIGGER_OP_DOMAIN_V1`]). Always sorted by `site`
/// (determinism + binary-search lookups), same convention as
/// [`SemanticGolden`].
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AnonSemanticGolden {
    pub schema_version: u32,
    /// Mint-time CDO workspace provenance (1B.3b Task 1 fix). `#[serde(default)]`
    /// so a pre-fix golden without this field still deserializes (though it
    /// would fail the `schema_version` check first in practice).
    #[serde(default)]
    pub metadata: MintMetadata,
    pub entries: Vec<AnonGoldenEntry>,
}

impl AnonSemanticGolden {
    fn get(&self, key: &AnonSiteKey) -> Option<&BTreeSet<AnonTarget>> {
        self.entries
            .binary_search_by(|e| e.site.cmp(key))
            .ok()
            .map(|i| &self.entries[i].targets)
    }
}

/// Anonymized, serde-able EventFlow pair key — see
/// `differential.rs::CanonicalEventRow`'s docs for why this is keyed by
/// `CanonicalKey` rather than L3's proprietary `stable_routine_id` scheme.
/// Both `publisher_id` and `subscriber_id` hash the FULL `CanonicalKey`
/// (app_guid + object_kind + object_lc + routine_lc) under
/// [`anon::EVENT_PAIR_DOMAIN_V1`]; `event_name_id` hashes the bare event name
/// under the same domain.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AnonEventPairKey {
    pub publisher_id: AnonId,
    pub event_name_id: AnonId,
    pub subscriber_id: AnonId,
}

/// One entry in the committed `cdo-event-anon.json`: an anonymized pub→sub
/// pair plus the CLEARTEXT resolved publisher arity (a bare parameter count —
/// non-identifying).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnonEventEntry {
    pub pair: AnonEventPairKey,
    pub publisher_arity: Option<usize>,
}

/// The committed `cdo-event-anon.json` shape.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AnonEventGolden {
    pub schema_version: u32,
    /// Mint-time CDO workspace provenance (1B.3b Task 1 fix) — see
    /// [`AnonSemanticGolden::metadata`].
    #[serde(default)]
    pub metadata: MintMetadata,
    pub entries: Vec<AnonEventEntry>,
}

/// Hash `(app, object_lc)` into one [`AnonId`] under [`anon::TARGET_DOMAIN_V1`]
/// — app-scoped object identity (combined, not two separate ids, so a small
/// numeric `object_lc` from two different apps cannot collide).
fn anon_target_object_id(app: &Option<String>, object_lc: &str) -> AnonId {
    let canon = format!("{}\u{1}{object_lc}", app.as_deref().unwrap_or(""));
    anon::anon(anon::TARGET_DOMAIN_V1, &canon)
}

fn anon_target_routine_id(routine_lc: &str) -> AnonId {
    anon::anon(anon::TARGET_DOMAIN_V1, routine_lc)
}

/// Anonymize one [`GoldenTarget`] under [`anon::TARGET_DOMAIN_V1`].
#[must_use]
pub fn anonymize_target(t: &GoldenTarget) -> AnonTarget {
    AnonTarget {
        kind: t.kind,
        object_id: anon_target_object_id(&t.app, &t.object_lc),
        routine_id: t.routine_lc.as_deref().map(anon_target_routine_id),
    }
}

/// Anonymize one [`GoldenSiteKey`] under `site_domain` (either
/// [`anon::SITE_DOMAIN_V1`] or [`anon::TRIGGER_OP_DOMAIN_V1`]).
#[must_use]
pub fn anonymize_site_key(key: &GoldenSiteKey, site_domain: &str) -> AnonSiteKey {
    AnonSiteKey {
        from_app_id: anon::anon(site_domain, &key.from_app_guid),
        from_object_kind: key.from_object_kind.clone(),
        from_object_id: anon::anon(site_domain, &key.from_object_lc),
        from_routine_id: anon::anon(site_domain, &key.from_routine_lc),
        edge_kind: key.edge_kind,
        unit_id: anon::anon(site_domain, &key.unit),
        line: key.line,
        callee_id: anon::anon(site_domain, &format!("{}", key.callee_fp)),
    }
}

/// Record every hashed field of `(plain, anon_key)` into `deanon`
/// (`AnonId.0 -> human-readable plaintext`) — the local, GITIGNORED
/// `cdo-deanon-map.json` accumulator. See `anon.rs`'s module docs ("the
/// re-hash-don't-decrypt principle"): this is the ONLY place plaintext is
/// ever written back out, and only to a local, never-committed file.
fn record_site_deanon(
    plain: &GoldenSiteKey,
    anon_key: &AnonSiteKey,
    deanon: &mut BTreeMap<String, String>,
) {
    deanon
        .entry(anon_key.from_app_id.0.clone())
        .or_insert_with(|| format!("app_guid={}", plain.from_app_guid));
    deanon
        .entry(anon_key.from_object_id.0.clone())
        .or_insert_with(|| {
            format!(
                "object_lc={} (kind={})",
                plain.from_object_lc, plain.from_object_kind
            )
        });
    deanon
        .entry(anon_key.from_routine_id.0.clone())
        .or_insert_with(|| format!("routine_lc={}", plain.from_routine_lc));
    deanon
        .entry(anon_key.unit_id.0.clone())
        .or_insert_with(|| format!("unit={}", plain.unit));
    deanon
        .entry(anon_key.callee_id.0.clone())
        .or_insert_with(|| format!("callee_fp={}", plain.callee_fp));
}

fn record_target_deanon(
    plain: &GoldenTarget,
    anon_t: &AnonTarget,
    deanon: &mut BTreeMap<String, String>,
) {
    deanon.entry(anon_t.object_id.0.clone()).or_insert_with(|| {
        format!(
            "app={:?} object_lc={} (kind={})",
            plain.app, plain.object_lc, plain.kind
        )
    });
    if let (Some(rid), Some(rlc)) = (&anon_t.routine_id, &plain.routine_lc) {
        deanon
            .entry(rid.0.clone())
            .or_insert_with(|| format!("routine_lc={rlc}"));
    }
}

/// Anonymize `golden` under `site_domain`, ALSO recording every hashed
/// field's plaintext into `deanon`. The dev-mint tool's primary entry point —
/// mint + anonymize + populate the local de-anon map in one pass.
#[must_use]
pub fn anonymize_golden_with_deanon(
    golden: &SemanticGolden,
    site_domain: &str,
    deanon: &mut BTreeMap<String, String>,
) -> AnonSemanticGolden {
    let mut entries: Vec<AnonGoldenEntry> = golden
        .entries
        .iter()
        .map(|e| {
            let asite = anonymize_site_key(&e.site, site_domain);
            record_site_deanon(&e.site, &asite, deanon);
            let atargets: BTreeSet<AnonTarget> = e
                .targets
                .iter()
                .map(|t| {
                    let at = anonymize_target(t);
                    record_target_deanon(t, &at, deanon);
                    at
                })
                .collect();
            AnonGoldenEntry {
                site: asite,
                targets: atargets,
            }
        })
        .collect();
    entries.sort_by(|a, b| a.site.cmp(&b.site));
    AnonSemanticGolden {
        schema_version: ANON_GOLDEN_SCHEMA_VERSION,
        // Caller (the dev-mint tool) overwrites this with the real mint-time
        // stamp via `workspace_git_info`; callers that don't care (e.g. the
        // runtime audits anonymizing the fresh side for an in-memory
        // comparison, never serialized) leave the default.
        metadata: MintMetadata::default(),
        entries,
    }
}

/// Anonymize `golden` under `site_domain` without recording a de-anon map
/// (callers that don't have/want a local map — e.g. a one-off comparison).
#[must_use]
pub fn anonymize_golden(golden: &SemanticGolden, site_domain: &str) -> AnonSemanticGolden {
    let mut scratch = BTreeMap::new();
    anonymize_golden_with_deanon(golden, site_domain, &mut scratch)
}

/// Hash a [`CanonicalKey`] (all four fields, joined) into one [`AnonId`]
/// under [`anon::EVENT_PAIR_DOMAIN_V1`].
fn anon_canonical_key(k: &CanonicalKey, domain: &str) -> AnonId {
    let s = format!(
        "{}\u{1}{}\u{1}{}\u{1}{}",
        k.app_guid, k.object_kind, k.object_lc, k.routine_lc
    );
    anon::anon(domain, &s)
}

/// Anonymize a batch of [`CanonicalEventRow`]s into the committed
/// `cdo-event-anon.json` shape, recording plaintext into `deanon`.
#[must_use]
pub fn anonymize_event_rows_with_deanon(
    rows: &[CanonicalEventRow],
    deanon: &mut BTreeMap<String, String>,
) -> AnonEventGolden {
    let mut entries: Vec<AnonEventEntry> = rows
        .iter()
        .map(|r| {
            let pair = AnonEventPairKey {
                publisher_id: anon_canonical_key(&r.publisher, anon::EVENT_PAIR_DOMAIN_V1),
                event_name_id: anon::anon(anon::EVENT_PAIR_DOMAIN_V1, &r.event_name_lc),
                subscriber_id: anon_canonical_key(&r.subscriber, anon::EVENT_PAIR_DOMAIN_V1),
            };
            deanon
                .entry(pair.publisher_id.0.clone())
                .or_insert_with(|| {
                    format!(
                        "publisher={}:{}:{}",
                        r.publisher.object_kind, r.publisher.object_lc, r.publisher.routine_lc
                    )
                });
            deanon
                .entry(pair.event_name_id.0.clone())
                .or_insert_with(|| format!("event_name_lc={}", r.event_name_lc));
            deanon
                .entry(pair.subscriber_id.0.clone())
                .or_insert_with(|| {
                    format!(
                        "subscriber={}:{}:{}",
                        r.subscriber.object_kind, r.subscriber.object_lc, r.subscriber.routine_lc
                    )
                });
            AnonEventEntry {
                pair,
                publisher_arity: r.publisher_arity,
            }
        })
        .collect();
    entries.sort_by(|a, b| a.pair.cmp(&b.pair));
    AnonEventGolden {
        schema_version: ANON_GOLDEN_SCHEMA_VERSION,
        // See the analogous comment in `anonymize_golden_with_deanon` —
        // overwritten by the dev-mint tool's caller with the real stamp.
        metadata: MintMetadata::default(),
        entries,
    }
}

// ---------------------------------------------------------------------------
// Diff types
// ---------------------------------------------------------------------------

/// A site where the fresh resolver emitted confident (non-Unresolved) targets
/// that differ from the L3-oracle targets.
///
/// This is the **confidently-wrong** class — a Histogram cannot detect it.
#[derive(Clone, Debug)]
pub struct FreshWrong {
    pub site: GoldenSiteKey,
    pub fresh_targets: BTreeSet<GoldenTarget>,
    pub l3_targets: BTreeSet<GoldenTarget>,
}

/// A site formerly in `fresh_wrong` where fresh's targets REFINE L3's target —
/// fresh is MORE precise (Phase-4 Interface/Polymorphic fan-out or superset).
/// Not a bug; the graph's `implements` relationship confirms the refinement.
pub type FreshAheadDispatch = FreshWrong;

/// A site where L3 resolved to a concrete target but fresh emitted empty targets.
#[derive(Clone, Debug)]
pub struct FreshMissing {
    pub site: GoldenSiteKey,
    pub l3_targets: BTreeSet<GoldenTarget>,
}

/// A site where fresh resolved to targets but L3 had an empty target set.
/// Fresh was ahead of L3 — a verified improvement.
#[derive(Clone, Debug)]
pub struct FreshExtra {
    pub site: GoldenSiteKey,
    pub fresh_targets: BTreeSet<GoldenTarget>,
}

/// Full classification from comparing fresh edges against the semantic golden.
#[derive(Clone, Debug, Default)]
pub struct SemanticDiff {
    /// Total paired sites (present in both fresh and golden on the same key).
    pub total_paired: usize,
    /// Paired sites where fresh and L3 targets agree exactly.
    pub matches: usize,
    /// Paired sites where fresh confidently resolved to the WRONG target.
    pub fresh_wrong: Vec<FreshWrong>,
    /// Paired sites where L3 resolved but fresh emitted empty (a gap).
    pub fresh_missing: Vec<FreshMissing>,
    /// Paired sites where fresh resolved and L3 had empty (a win).
    pub fresh_extra: Vec<FreshExtra>,
    /// Fresh sites that have no golden entry (edges L3 never saw, e.g.
    /// `EventFlow`, `ImplicitTrigger`, dynamic ObjectRun sites).
    pub fresh_novel: usize,
    /// Golden sites with no fresh peer (fresh emitted no site for this key).
    pub golden_missing: usize,
}

// ---------------------------------------------------------------------------
// CDO audit report
// ---------------------------------------------------------------------------

/// Result of the CDO/L3 semantic audit over a real workspace.
#[derive(Clone, Debug, Default)]
pub struct CdoSemanticAuditReport {
    /// 1B.3b Task 1: `true` when the committed `cdo-anon.json` golden loaded
    /// and parsed successfully. The `ENFORCE_CDO_WS=1` guard hard-fails on
    /// `false` — see `tests/program_resolve_harness.rs`'s `cdo_ws_or_enforce`.
    pub golden_loaded: bool,
    pub l3_total: usize,
    pub fresh_total: usize,
    pub paired: usize,
    /// Total sites where fresh and L3 differ and both are non-empty.
    /// Equals `fresh_ahead_dispatch_count + genuine_wrong_count`.
    pub fresh_wrong_count: usize,
    /// Sites adjudicated as "fresh is more precise" (interface fan-out / superset).
    pub fresh_ahead_dispatch_count: usize,
    /// Sites adjudicated as genuinely wrong (disjoint target — a real bug).
    pub genuine_wrong_count: usize,
    /// Genuine_wrong site keys exposed for the HARD GATE set-membership check.
    /// The test asserts every site's `(unit, line, callee_fp)` is present in
    /// the committed manifest
    /// (`tests/goldens/semantic-edges/known-genuine-divergences.json`).
    pub genuine_wrong_sites: Vec<GoldenSiteKey>,
    pub fresh_missing_count: usize,
    pub fresh_extra_count: usize,
    pub fresh_novel: usize,
    pub golden_missing: usize,
    /// SHA-256 hex digest over the sorted site→(l3_targets, fresh_targets) pairs.
    /// Deterministic across runs; used as a pinnable CDO audit fingerprint.
    pub digest: String,
}

/// Result of the L3/fresh ImplicitTrigger frozen-golden audit
/// (`cdo-trigger-anon.json`, `site_domain = `[`anon::TRIGGER_OP_DOMAIN_V1`]).
///
/// 1B.3b scope note: unlike [`CdoSemanticAuditReport`], this report does NOT
/// adjudicate `fresh_wrong` into fresh-ahead-dispatch vs genuine-wrong — that
/// classification (and the `known-genuine-divergences.json` manifest) is
/// scoped to the Member/Interface golden only. The live, CDO-gated
/// `run_implicit_trigger_harness` (`differential.rs`) remains the
/// zero-tolerance gate for ImplicitTrigger correctness (unchanged this task —
/// removed only in 1B.3b Task 3). This audit exists to PROVE the
/// frozen-load-and-anonymize mechanism works for the ImplicitTrigger dispatch
/// kind and to back the `ENFORCE_CDO_WS` guard's `checked_sites>0` requirement.
#[derive(Clone, Debug, Default)]
pub struct AnonTriggerAuditReport {
    pub golden_loaded: bool,
    pub l3_total: usize,
    pub fresh_total: usize,
    pub total_paired: usize,
    pub matches: usize,
    pub fresh_wrong_count: usize,
    pub fresh_missing: usize,
    pub fresh_extra: usize,
    pub fresh_novel: usize,
    pub golden_missing: usize,
    pub digest: String,
}

/// Result of the L3/fresh EventFlow frozen-golden audit
/// (`cdo-event-anon.json`). Arity-agnostic pair-set comparison only (mirrors
/// `run_event_flow_gate`'s Stage-1 join, not its Stage-2 arity adjudication) —
/// see [`AnonTriggerAuditReport`]'s scope note; the same reasoning applies
/// here. The live, CDO-gated `run_event_flow_gate` remains the zero-tolerance
/// EventFlow gate (unchanged this task).
#[derive(Clone, Debug, Default)]
pub struct AnonEventAuditReport {
    pub golden_loaded: bool,
    pub l3_total: usize,
    pub fresh_total: usize,
    pub matched_pairs: usize,
    pub pair_l3_only: usize,
    pub pair_fresh_only: usize,
    pub digest: String,
}

// ---------------------------------------------------------------------------
// 1B.3b Task 1: load-frozen audit infrastructure (anonymized diff)
// ---------------------------------------------------------------------------

/// An anonymized [`FreshWrong`] — same meaning, [`AnonTarget`] instead of
/// [`GoldenTarget`].
#[derive(Clone, Debug)]
pub struct AnonFreshWrong {
    pub site: AnonSiteKey,
    pub fresh_targets: BTreeSet<AnonTarget>,
    pub l3_targets: BTreeSet<AnonTarget>,
}

/// Anonymized counterpart of [`SemanticDiff`]. `fresh_missing`/`fresh_extra`
/// are plain counts (not `Vec`s) — the load-frozen audits don't need the
/// per-site detail beyond `fresh_wrong` (which DOES carry detail, because
/// that's the bucket the genuine-wrong adjudication and the deanon map need).
#[derive(Clone, Debug, Default)]
pub struct AnonSemanticDiff {
    pub total_paired: usize,
    pub matches: usize,
    pub fresh_wrong: Vec<AnonFreshWrong>,
    pub fresh_missing: usize,
    pub fresh_extra: usize,
    pub fresh_novel: usize,
    pub golden_missing: usize,
}

fn semantic_edges_golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/goldens/semantic-edges")
}

/// Path to the committed Member/Interface anonymized golden.
#[must_use]
pub fn cdo_anon_golden_path() -> PathBuf {
    semantic_edges_golden_dir().join("cdo-anon.json")
}

/// Path to the committed ImplicitTrigger anonymized golden.
#[must_use]
pub fn cdo_trigger_anon_golden_path() -> PathBuf {
    semantic_edges_golden_dir().join("cdo-trigger-anon.json")
}

/// Path to the committed EventFlow anonymized golden.
#[must_use]
pub fn cdo_event_anon_golden_path() -> PathBuf {
    semantic_edges_golden_dir().join("cdo-event-anon.json")
}

/// Path to the GITIGNORED local de-anonymization map
/// (`AnonId.0 -> human-readable plaintext`). NEVER committed — see `anon.rs`'s
/// module docs.
#[must_use]
pub fn cdo_deanon_map_path() -> PathBuf {
    semantic_edges_golden_dir().join("cdo-deanon-map.json")
}

/// Load + validate a committed [`AnonSemanticGolden`] (`cdo-anon.json` /
/// `cdo-trigger-anon.json`). Returns `None` when the file is missing,
/// unparseable, or carries a `schema_version` other than
/// [`ANON_GOLDEN_SCHEMA_VERSION`] — fail-closed, never panics; the
/// `ENFORCE_CDO_WS` guard is the caller's responsibility (see
/// `tests/program_resolve_harness.rs`).
#[must_use]
pub fn load_anon_golden(path: &Path) -> Option<AnonSemanticGolden> {
    let json = std::fs::read_to_string(path).ok()?;
    let golden: AnonSemanticGolden = serde_json::from_str(&json).ok()?;
    if golden.schema_version != ANON_GOLDEN_SCHEMA_VERSION {
        return None;
    }
    Some(golden)
}

/// Load + validate a committed [`AnonEventGolden`] (`cdo-event-anon.json`).
/// Same fail-closed contract as [`load_anon_golden`].
#[must_use]
pub fn load_anon_event_golden(path: &Path) -> Option<AnonEventGolden> {
    let json = std::fs::read_to_string(path).ok()?;
    let golden: AnonEventGolden = serde_json::from_str(&json).ok()?;
    if golden.schema_version != ANON_GOLDEN_SCHEMA_VERSION {
        return None;
    }
    Some(golden)
}

/// Merge `new_entries` into the GITIGNORED local de-anonymization map at
/// `path`, creating it if absent. Existing entries win on key collision
/// (first writer's plaintext is kept — there should never be a genuine
/// disagreement since the SAME plaintext always re-hashes to the SAME id).
/// Best-effort: I/O failures are swallowed — the map is a LOCAL debugging
/// aid, never required for correctness (see `anon.rs`'s module docs).
pub fn merge_deanon_map(path: &Path, new_entries: &BTreeMap<String, String>) {
    if new_entries.is_empty() {
        return;
    }
    let mut map: BTreeMap<String, String> = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    for (k, v) in new_entries {
        map.entry(k.clone()).or_insert_with(|| v.clone());
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(&map) {
        let _ = std::fs::write(path, json);
    }
}

/// Build the anonymized fresh-side site→targets map AND a reverse
/// `AnonSiteKey -> GoldenSiteKey` index. The reverse index is what lets
/// `run_cdo_semantic_audit` recover PLAINTEXT fresh identity for a failing
/// `fresh_wrong`/`genuine_wrong` site (for the deanon map and for
/// `CdoSemanticAuditReport::genuine_wrong_sites`, which stays plaintext
/// `GoldenSiteKey` because it only ever needs FRESH's own identity — see the
/// module-level "re-hash-don't-decrypt" principle in `anon.rs`).
fn anonymize_fresh_map(
    fresh_plain: &BTreeMap<GoldenSiteKey, BTreeSet<GoldenTarget>>,
    site_domain: &str,
) -> (
    BTreeMap<AnonSiteKey, BTreeSet<AnonTarget>>,
    HashMap<AnonSiteKey, GoldenSiteKey>,
) {
    let mut anon_map: BTreeMap<AnonSiteKey, BTreeSet<AnonTarget>> = BTreeMap::new();
    let mut reverse: HashMap<AnonSiteKey, GoldenSiteKey> = HashMap::new();
    for (site, targets) in fresh_plain {
        let asite = anonymize_site_key(site, site_domain);
        let atargets: BTreeSet<AnonTarget> = targets.iter().map(anonymize_target).collect();
        reverse.insert(asite.clone(), site.clone());
        anon_map.entry(asite).or_default().extend(atargets);
    }
    (anon_map, reverse)
}

/// Diff an anonymized fresh site→targets map against a loaded committed
/// golden. Same classification rule as [`assert_against_semantic_golden`],
/// operating on [`AnonSiteKey`]/[`AnonTarget`] instead of the plaintext types.
#[must_use]
fn diff_against_anon_golden(
    fresh_anon: &BTreeMap<AnonSiteKey, BTreeSet<AnonTarget>>,
    golden: &AnonSemanticGolden,
) -> AnonSemanticDiff {
    let mut diff = AnonSemanticDiff::default();
    for entry in &golden.entries {
        let l3_targets = &entry.targets;
        if let Some(fresh_targets) = fresh_anon.get(&entry.site) {
            diff.total_paired += 1;
            if fresh_targets == l3_targets {
                diff.matches += 1;
            } else if !l3_targets.is_empty() && !fresh_targets.is_empty() {
                diff.fresh_wrong.push(AnonFreshWrong {
                    site: entry.site.clone(),
                    fresh_targets: fresh_targets.clone(),
                    l3_targets: l3_targets.clone(),
                });
            } else if !l3_targets.is_empty() {
                diff.fresh_missing += 1;
            } else {
                diff.fresh_extra += 1;
            }
        } else {
            diff.golden_missing += 1;
        }
    }
    for key in fresh_anon.keys() {
        if golden.get(key).is_none() {
            diff.fresh_novel += 1;
        }
    }
    diff
}

// ---------------------------------------------------------------------------
// Route-applicability report
// ---------------------------------------------------------------------------

/// Result of the structural route-applicability contract check.
#[derive(Clone, Debug, Default)]
pub struct ApplicabilityReport {
    pub total_routes: usize,
    /// Routes where the `evidence`/`witness` pair is not valid.
    pub witness_contract_violations: usize,
    /// `AbiSymbol` routes whose key is absent from the raw-ABI index.
    pub abi_unmapped: usize,
}

impl ApplicabilityReport {
    pub fn is_clean(&self) -> bool {
        self.witness_contract_violations == 0 && self.abi_unmapped == 0
    }
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

fn canonical_to_golden_key(e: &CanonicalEdge) -> GoldenSiteKey {
    GoldenSiteKey {
        from_app_guid: e.from.app_guid.clone(),
        from_object_kind: e.from.object_kind.clone(),
        from_object_lc: e.from.object_lc.clone(),
        from_routine_lc: e.from.routine_lc.clone(),
        edge_kind: match e.kind {
            EdgeKind::Call => 0,
            EdgeKind::Run => 1,
            EdgeKind::ImplicitTrigger => 2,
            EdgeKind::EventFlow => 3,
        },
        unit: e.site.span.unit.clone(),
        line: e.site.span.start.line,
        callee_fp: e.site.callee_fp,
    }
}

fn canonical_targets_to_golden(targets: &BTreeSet<CanonicalTarget>) -> BTreeSet<GoldenTarget> {
    targets
        .iter()
        .map(|t| GoldenTarget {
            kind: t.kind,
            app: t.app.clone(),
            object_lc: t.object_lc.clone(),
            routine_lc: t.routine_lc.clone(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Adjudication helper
// ---------------------------------------------------------------------------

/// Adjudicate a `FreshWrong` site: are fresh's and L3's target sets in a
/// REFINEMENT relationship (allowed), or genuinely disjoint (a real divergence)?
///
/// Returns `true` (fresh_ahead_dispatch — allowed) when ANY of these holds:
/// 1. `l3 ⊆ fresh` — fresh is a superset that includes all of L3's answer
///    (Interface/Polymorphic fan-out: fresh is MORE precise).
/// 2. `fresh ⊆ l3` — fresh partially resolved a call site that L3 captured more
///    broadly (multiple physical calls can share one `(line, callee_fp)` bucket).
///    Every target fresh emitted IS in L3's set, so none are confidently wrong —
///    fresh is merely less complete, not wrong.
/// 3. Every L3 target is an interface (kind=11) AND every fresh target implements
///    it (verified via the graph's `ObjectNode.implements` field).
///
/// Returns `false` (genuine_wrong) only when the two non-empty target sets are
/// DISJOINT (or partially overlap with neither a subset nor interface-implements
/// relationship) — fresh and L3 confidently resolved the same site to unrelated
/// targets. NOTE: this is symmetric — it does NOT assert which side is correct;
/// adjudicating that is deferred to 1B.3b.
///
/// # Known partial-recall blind spot (named: 1B.3b-disambiguation)
///
/// Case 2 (`fresh ⊆ l3`) creates a partial-recall blind spot: when fresh finds
/// only a strict subset of the correct targets in a multi-target bucket (e.g.,
/// resolves 2 of 3 interface implementers), the site is classified
/// `fresh_ahead_dispatch` here — NOT as `fresh_missing` or `genuine_wrong`.
/// The dropped target is silently masked by this gate.
///
/// **Mitigation while L3 is the oracle**: the resolution/member harnesses assert
/// `regression_unexplained == 0` independently — any unexplained resolution
/// regression fires there and acts as defense-in-depth covering this blind spot.
///
/// Full per-target recall validation is a named 1B.3b-disambiguation follow-up.
///
/// # 1B.3b: ported to the anonymized identity space
///
/// `run_cdo_semantic_audit` no longer holds L3's plaintext target set (it
/// LOADS the committed anonymized golden) — only [`AnonTarget`]s. The THREE
/// CASES above are preserved EXACTLY; only the identity type changed, per
/// `anon.rs`'s "re-hash-don't-decrypt" principle: `obj_lookup_anon` is built
/// ONCE from the LIVE graph (real `ObjectNode`s, each keyed by its OWN
/// re-hashed identity), so both `l3` (anonymized, loaded from the frozen
/// golden) and `fresh` (anonymized, computed live) can look themselves up by
/// anonymized identity without ever inverting a committed id.
fn is_fresh_ahead_dispatch_anon(
    fresh: &BTreeSet<AnonTarget>,
    l3: &BTreeSet<AnonTarget>,
    obj_lookup_anon: &HashMap<AnonId, &ObjectNode>,
) -> bool {
    if fresh.is_empty() || l3.is_empty() {
        return false;
    }

    // Case 1: L3's targets ⊆ fresh's targets (fresh is a superset: includes all of L3's answer).
    if l3.is_subset(fresh) {
        return true;
    }

    // Case 3: fresh's targets ⊆ L3's targets — fresh partially resolved a compound call
    // that L3 captured more broadly (e.g. L3 follows both the primary dispatch and an
    // EventFlow edge on the same callee_fp).  Fresh is NOT wrong — every target it emitted
    // is in L3's set — it simply emitted fewer.  Classify as fresh_ahead_dispatch (really
    // "fresh_partial_correct") rather than genuine_wrong.
    if fresh.is_subset(l3) {
        return true;
    }

    // Case 2: All L3 targets are interfaces (kind=11) and all fresh targets implement them.
    if !l3.iter().all(|t| t.kind == 11) {
        return false;
    }

    for l3_target in l3 {
        let Some(l3_obj) = obj_lookup_anon.get(&l3_target.object_id) else {
            // Cannot find the interface object in the live graph → cannot verify → genuine_wrong.
            return false;
        };
        let iface_name_lc = l3_obj.name.to_ascii_lowercase();

        for fresh_target in fresh {
            // Routine names should agree for a valid interface dispatch.
            if fresh_target.routine_id != l3_target.routine_id {
                return false;
            }
            let Some(fresh_obj) = obj_lookup_anon.get(&fresh_target.object_id) else {
                return false;
            };
            // The concrete object must declare it implements the interface.
            if !fresh_obj
                .implements
                .iter()
                .any(|i| i.to_ascii_lowercase() == iface_name_lc)
            {
                return false;
            }
        }
    }
    true
}

/// Build `AnonId -> &ObjectNode` over every object in `graph`, keyed the SAME
/// way [`anon_target_object_id`] keys an [`AnonTarget`] — i.e. re-hashing
/// `(app_guid, object_lc)` for every LIVE object so an anonymized target's
/// `object_id` (whether from the loaded golden or freshly computed) can find
/// its `ObjectNode` without ever inverting a committed id.
fn build_obj_lookup_anon(
    graph: &crate::program::graph::ProgramGraph,
) -> HashMap<AnonId, &ObjectNode> {
    let mut lookup: HashMap<AnonId, &ObjectNode> = HashMap::new();
    for obj in &graph.objects {
        let guid = graph
            .apps
            .try_resolve(obj.id.app)
            .map(|a| a.guid.clone())
            .unwrap_or_default();
        let lc = match &obj.id.key {
            ObjKey::Id(n) => format!("{n}"),
            ObjKey::Name(s) => s.clone(),
        };
        lookup.insert(anon_target_object_id(&Some(guid), &lc), obj);
    }
    lookup
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Build a [`SemanticGolden`] from a batch of canonical edges. Oracle-
/// agnostic — the caller decides whether `edges` came from L3 or from the
/// fresh resolver. Shared by [`mint_l3_validated_golden`],
/// [`mint_l3_trigger_golden`], and [`mint_fresh_golden_for_kind`].
fn build_golden_from_canonical(edges: &[CanonicalEdge]) -> SemanticGolden {
    let mut map: BTreeMap<GoldenSiteKey, BTreeSet<GoldenTarget>> = BTreeMap::new();
    for edge in edges {
        let key = canonical_to_golden_key(edge);
        let targets = canonical_targets_to_golden(&edge.targets);
        map.entry(key).or_default().extend(targets);
    }
    SemanticGolden::from_map(map)
}

/// **SANCTIONED L3 ORACLE USE (1B.3b: the dev-mint tool is the only caller
/// post-freeze; the in-repo fixture's `REGEN_TEMP_GOLDENS` path also still
/// calls this directly — see `tests/program_resolve_harness.rs` Test 14)**:
/// mint the Member/Interface semantic golden from the L3 oracle.
///
/// Calls [`project_l3`] over `workspace_root`, collects per-site target sets into
/// a [`SemanticGolden`] keyed by column-ignoring [`GoldenSiteKey`].
///
/// Empty target sets (L3 Unknown/Unresolved) are retained — they record sites
/// that L3 extracted but could not resolve, so the golden covers them.
#[must_use]
pub fn mint_l3_validated_golden(workspace_root: &Path) -> SemanticGolden {
    build_golden_from_canonical(&project_l3(workspace_root))
}

/// **SANCTIONED L3 ORACLE USE (1B.3b dev-mint tool only)**: mint the
/// ImplicitTrigger semantic golden from L3's native `PRecordOperation`-keyed
/// edges ([`project_l3_implicit_trigger_in_scope`]). Backs `cdo-trigger-anon.json`.
#[must_use]
pub fn mint_l3_trigger_golden(workspace_root: &Path) -> SemanticGolden {
    build_golden_from_canonical(&project_l3_implicit_trigger_in_scope(workspace_root))
}

/// L3-INDEPENDENT: mint a [`SemanticGolden`] from the FRESH resolver's OWN
/// output, filtered to one [`EdgeKind`]. Used to freeze fresh's own
/// resolution as a committed regression baseline for dispatch kinds a small
/// synthetic fixture exercises end-to-end without L3 at all (the
/// ImplicitTrigger target-set fixture — 1B.3b Task 1 Step 4). NOT used for
/// the CDO-derived goldens (those freeze the L3 VERDICT, not fresh's own
/// output — see [`mint_l3_validated_golden`]/[`mint_l3_trigger_golden`]).
#[must_use]
pub fn mint_fresh_golden_for_kind(workspace_root: &Path, kind: EdgeKind) -> SemanticGolden {
    use crate::program::abi_ingest::AbiCache;
    use crate::program::build::build_program_graph;
    use crate::program::resolve::full::resolve_full_program;
    use crate::snapshot::SnapshotBuilder;

    let snap = match (SnapshotBuilder {
        workspace_root: workspace_root.to_path_buf(),
        local_providers: vec![],
    })
    .build()
    {
        Ok(s) => s,
        Err(_) => return SemanticGolden::default(),
    };
    let graph = build_program_graph(&snap, &AbiCache::new());
    let Some(report) = resolve_full_program(workspace_root) else {
        return SemanticGolden::default();
    };
    let edges: Vec<Edge> = report
        .edges
        .into_iter()
        .map(|ce| ce.edge)
        .filter(|e| e.kind == kind)
        .collect();
    let canonical = project_fresh(&edges, &graph.apps);
    build_golden_from_canonical(&canonical)
}

/// Compare a fresh canonical edge batch against the L3-minted golden.
///
/// Returns a [`SemanticDiff`] classifying every site.
///
/// **The critical invariant is `fresh_wrong.is_empty()`** — fresh must never
/// confidently emit a target that L3 says is wrong.  `fresh_missing` tracks
/// the Task-3 Unknown gap where L3 resolved but fresh did not (acceptable
/// progress gap — reduce it, never introduce new ones).
#[must_use]
pub fn assert_against_semantic_golden(
    fresh: &[CanonicalEdge],
    golden: &SemanticGolden,
) -> SemanticDiff {
    // Build fresh key → targets map (union duplicate keys).
    let mut fresh_map: BTreeMap<GoldenSiteKey, BTreeSet<GoldenTarget>> = BTreeMap::new();
    for edge in fresh {
        let key = canonical_to_golden_key(edge);
        let targets = canonical_targets_to_golden(&edge.targets);
        fresh_map.entry(key).or_default().extend(targets);
    }

    let mut diff = SemanticDiff::default();

    // Walk golden entries and classify.
    for entry in &golden.entries {
        let key = &entry.site;
        let l3_targets = &entry.targets;
        if let Some(fresh_targets) = fresh_map.get(key) {
            diff.total_paired += 1;
            if fresh_targets == l3_targets {
                diff.matches += 1;
            } else if !l3_targets.is_empty() && !fresh_targets.is_empty() {
                // Both sides resolved but to different targets — the confidently-wrong class.
                diff.fresh_wrong.push(FreshWrong {
                    site: key.clone(),
                    fresh_targets: fresh_targets.clone(),
                    l3_targets: l3_targets.clone(),
                });
            } else if !l3_targets.is_empty() {
                // L3 resolved; fresh did not — a gap.
                diff.fresh_missing.push(FreshMissing {
                    site: key.clone(),
                    l3_targets: l3_targets.clone(),
                });
            } else {
                // L3 empty; fresh resolved — fresh is ahead of L3 (a win).
                diff.fresh_extra.push(FreshExtra {
                    site: key.clone(),
                    fresh_targets: fresh_targets.clone(),
                });
            }
        } else {
            // Golden site has no fresh peer.
            diff.golden_missing += 1;
        }
    }

    // Count fresh sites not in the golden (EventFlow, ImplicitTrigger, etc.).
    for key in fresh_map.keys() {
        if golden.get(key).is_none() {
            diff.fresh_novel += 1;
        }
    }

    diff
}

/// Route-applicability structural contract.
///
/// Checks the witness↔evidence contract on every route in `edges` and
/// delegates the ABI ingestion integrity check to [`abi_ingestion_integrity`].
/// Both must be zero for [`ApplicabilityReport::is_clean`] to return `true`.
#[must_use]
pub fn route_applicability(
    edges: &[Edge],
    raw_abi: &crate::program::resolve::abi_check::RawAbiIndex,
) -> ApplicabilityReport {
    let mut total_routes = 0usize;
    let mut witness_contract_violations = 0usize;
    for edge in edges {
        for route in edge.all_routes() {
            total_routes += 1;
            if !witness_contract_holds(route) {
                witness_contract_violations += 1;
            }
        }
    }
    let abi_report = abi_ingestion_integrity(edges, raw_abi);
    ApplicabilityReport {
        total_routes,
        witness_contract_violations,
        abi_unmapped: abi_report.abi_unmapped,
    }
}

/// Compare the fresh resolver's output for `workspace_root` against `golden`.
///
/// Internally builds the snapshot + graph (for `AppRegistry`) and calls
/// `resolve_full_program`.  Filters fresh edges to the workspace app before
/// projecting.  Used by the in-repo fixture assertion.
#[must_use]
pub fn run_semantic_diff(workspace_root: &Path, golden: &SemanticGolden) -> SemanticDiff {
    use crate::program::abi_ingest::AbiCache;
    use crate::program::build::build_program_graph;
    use crate::program::resolve::full::resolve_full_program;
    use crate::snapshot::SnapshotBuilder;

    let snap = match (SnapshotBuilder {
        workspace_root: workspace_root.to_path_buf(),
        local_providers: vec![],
    })
    .build()
    {
        Ok(s) => s,
        Err(_) => return SemanticDiff::default(),
    };
    let graph = build_program_graph(&snap, &AbiCache::new());
    let Some(ws_ref) = graph.apps.find(&snap.workspace_app) else {
        return SemanticDiff::default();
    };
    let Some(report) = resolve_full_program(workspace_root) else {
        return SemanticDiff::default();
    };
    // Filter to workspace app (matches L3's workspace-only scope).
    let ws_edges: Vec<Edge> = report
        .edges
        .into_iter()
        .filter(|ce| ce.edge.from.object.app == ws_ref)
        .map(|ce| ce.edge)
        .collect();
    let fresh_canonical = project_fresh(&ws_edges, &graph.apps);
    assert_against_semantic_golden(&fresh_canonical, golden)
}

/// Run the route-applicability check over `workspace_root`.
///
/// Builds the snapshot and raw-ABI index internally.
#[must_use]
pub fn run_route_applicability(workspace_root: &Path) -> ApplicabilityReport {
    use crate::program::abi_ingest::AbiCache;
    use crate::program::build::build_program_graph;
    use crate::program::resolve::full::resolve_full_program;
    use crate::snapshot::SnapshotBuilder;

    let snap = match (SnapshotBuilder {
        workspace_root: workspace_root.to_path_buf(),
        local_providers: vec![],
    })
    .build()
    {
        Ok(s) => s,
        Err(_) => return ApplicabilityReport::default(),
    };
    let graph = build_program_graph(&snap, &AbiCache::new());
    let raw_abi = build_raw_abi_index_from_snapshot(&snap, &graph.apps);
    let Some(report) = resolve_full_program(workspace_root) else {
        return ApplicabilityReport::default();
    };
    let all_edges: Vec<Edge> = report.edges.into_iter().map(|ce| ce.edge).collect();
    route_applicability(&all_edges, &raw_abi)
}

/// CDO semantic audit: compare the fresh resolver against the COMMITTED,
/// ANONYMIZED, FROZEN L3 verdict (`cdo-anon.json`) over a real workspace.
///
/// 1B.3b Task 1: this NO LONGER calls [`project_l3`] (or builds an L3
/// workspace at all) — it LOADS the committed golden and anonymizes the
/// fresh side with the SAME [`anon::anon`] so the two align. The gate module
/// (`src/program/resolve`) has exactly ONE remaining `engine::l3` import
/// chain after this swap: [`mint_l3_validated_golden`]/[`mint_l3_trigger_golden`]
/// (the dev-mint tool's sanctioned callers; also Test 14's `REGEN_TEMP_GOLDENS`
/// path) — `run_cdo_semantic_audit` itself touches neither.
///
/// Callers should gate this on `CDO_WS` env var before calling — this
/// function still does a real fresh-resolution build, which is expensive on
/// CDO-scale workspaces.
///
/// Returns a [`CdoSemanticAuditReport`]. `golden_loaded == false` means
/// `cdo-anon.json` is missing/invalid (the `ENFORCE_CDO_WS` guard in
/// `tests/program_resolve_harness.rs` hard-fails on this).
#[must_use]
pub fn run_cdo_semantic_audit(workspace_root: &Path) -> CdoSemanticAuditReport {
    use crate::program::abi_ingest::AbiCache;
    use crate::program::build::build_program_graph;
    use crate::program::resolve::full::resolve_full_program;
    use crate::snapshot::SnapshotBuilder;

    // ── Load the committed, anonymized golden (NO project_l3 call here) ──────
    let golden = load_anon_golden(&cdo_anon_golden_path());
    let golden_loaded = golden.is_some();
    let golden = golden.unwrap_or_default();
    let l3_total = golden.entries.len();
    // 1B.3b Task 1 fix (Fix 4): warn (never fail) when CDO_WS has drifted
    // from the golden's mint-time stamp.
    if golden_loaded {
        warn_on_workspace_drift(&golden.metadata, workspace_root);
    }

    // ── Build graph for AppRegistry (needed for project_fresh) ───────────────
    let snap = match (SnapshotBuilder {
        workspace_root: workspace_root.to_path_buf(),
        local_providers: vec![],
    })
    .build()
    {
        Ok(s) => s,
        Err(_) => {
            return CdoSemanticAuditReport {
                golden_loaded,
                l3_total,
                ..Default::default()
            };
        }
    };
    let graph = build_program_graph(&snap, &AbiCache::new());
    let Some(ws_ref) = graph.apps.find(&snap.workspace_app) else {
        return CdoSemanticAuditReport {
            golden_loaded,
            l3_total,
            ..Default::default()
        };
    };

    // ── Fresh resolver ────────────────────────────────────────────────────────
    let Some(report) = resolve_full_program(workspace_root) else {
        return CdoSemanticAuditReport {
            golden_loaded,
            l3_total,
            ..Default::default()
        };
    };
    // Filter to workspace app (L3 is workspace-scoped).
    let ws_edges: Vec<Edge> = report
        .edges
        .into_iter()
        .filter(|ce| ce.edge.from.object.app == ws_ref)
        .map(|ce| ce.edge)
        .collect();
    let fresh_total = ws_edges.len();

    // ── Project fresh → canonical → plaintext map → anonymized map ───────────
    let fresh_canonical = project_fresh(&ws_edges, &graph.apps);
    let mut fresh_plain: BTreeMap<GoldenSiteKey, BTreeSet<GoldenTarget>> = BTreeMap::new();
    for e in &fresh_canonical {
        let key = canonical_to_golden_key(e);
        let targets = canonical_targets_to_golden(&e.targets);
        fresh_plain.entry(key).or_default().extend(targets);
    }
    let (fresh_anon, reverse_site) = anonymize_fresh_map(&fresh_plain, anon::SITE_DOMAIN_V1);

    // ── Diff (anonymized) ─────────────────────────────────────────────────────
    let diff = diff_against_anon_golden(&fresh_anon, &golden);

    // ── Adjudicate fresh_wrong into fresh_ahead_dispatch vs genuine_wrong ────
    let obj_lookup_anon = build_obj_lookup_anon(&graph);

    let mut fresh_ahead_dispatch_count = 0usize;
    let mut genuine_wrong_sites: Vec<GoldenSiteKey> = Vec::new();
    let mut deanon: BTreeMap<String, String> = BTreeMap::new();
    // Record plaintext for every fresh site/target this run touched, not just
    // the failures — cheap (already in memory) and keeps the local deanon map
    // maximally useful for root-causing ANY future failure, not just today's.
    for (site, targets) in &fresh_plain {
        let asite = anonymize_site_key(site, anon::SITE_DOMAIN_V1);
        record_site_deanon(site, &asite, &mut deanon);
        for t in targets {
            let at = anonymize_target(t);
            record_target_deanon(t, &at, &mut deanon);
        }
    }

    for fw in &diff.fresh_wrong {
        if is_fresh_ahead_dispatch_anon(&fw.fresh_targets, &fw.l3_targets, &obj_lookup_anon) {
            fresh_ahead_dispatch_count += 1;
        } else if let Some(plain_site) = reverse_site.get(&fw.site) {
            genuine_wrong_sites.push(plain_site.clone());
        }
        // A fw.site with no `reverse_site` entry cannot happen: `fw` is built
        // from `fresh_anon`'s keys, and `reverse_site` is populated 1:1 with
        // `fresh_anon` by `anonymize_fresh_map`.
    }
    merge_deanon_map(&cdo_deanon_map_path(), &deanon);

    eprintln!(
        "\nAdjudication: fresh_wrong={} → fresh_ahead_dispatch={} genuine_wrong={}",
        diff.fresh_wrong.len(),
        fresh_ahead_dispatch_count,
        genuine_wrong_sites.len(),
    );
    for site in &genuine_wrong_sites {
        eprintln!("  GENUINE_WRONG site={site:?}");
    }

    // ── Deterministic digest (over the ANONYMIZED comparison) ────────────────
    let mut hasher = Sha256::new();
    for entry in &golden.entries {
        let fresh_targets = fresh_anon.get(&entry.site).cloned().unwrap_or_default();
        let k_json = serde_json::to_string(&entry.site).unwrap_or_default();
        let l_json = serde_json::to_string(&entry.targets).unwrap_or_default();
        let f_json = serde_json::to_string(&fresh_targets).unwrap_or_default();
        hasher.update(format!("{k_json}|{l_json}|{f_json}\n").as_bytes());
    }
    let digest_bytes = hasher.finalize();
    let digest: String = digest_bytes.iter().map(|b| format!("{b:02x}")).collect();

    CdoSemanticAuditReport {
        golden_loaded,
        l3_total,
        fresh_total,
        paired: diff.total_paired,
        fresh_wrong_count: diff.fresh_wrong.len(),
        fresh_ahead_dispatch_count,
        genuine_wrong_count: genuine_wrong_sites.len(),
        genuine_wrong_sites,
        fresh_missing_count: diff.fresh_missing,
        fresh_extra_count: diff.fresh_extra,
        fresh_novel: diff.fresh_novel,
        golden_missing: diff.golden_missing,
        digest,
    }
}

/// CDO ImplicitTrigger frozen-golden audit: compare the fresh resolver's
/// `ImplicitTrigger` edges against the committed, anonymized L3 verdict
/// (`cdo-trigger-anon.json`). See [`AnonTriggerAuditReport`]'s doc comment for
/// this audit's scope (mechanism proof + `ENFORCE_CDO_WS` backing — NOT a
/// genuine-wrong adjudication gate; that stays in the live
/// `run_implicit_trigger_harness` until Task 3).
#[must_use]
pub fn run_cdo_trigger_audit(workspace_root: &Path) -> AnonTriggerAuditReport {
    let golden = load_anon_golden(&cdo_trigger_anon_golden_path());
    let golden_loaded = golden.is_some();
    let golden = golden.unwrap_or_default();
    let l3_total = golden.entries.len();
    if golden_loaded {
        warn_on_workspace_drift(&golden.metadata, workspace_root);
    }

    let fresh_golden = mint_fresh_golden_for_kind(workspace_root, EdgeKind::ImplicitTrigger);
    let fresh_total = fresh_golden.entries.len();

    let mut fresh_plain: BTreeMap<GoldenSiteKey, BTreeSet<GoldenTarget>> = BTreeMap::new();
    for e in &fresh_golden.entries {
        fresh_plain.insert(e.site.clone(), e.targets.clone());
    }
    let (fresh_anon, _reverse_site) = anonymize_fresh_map(&fresh_plain, anon::TRIGGER_OP_DOMAIN_V1);

    let diff = diff_against_anon_golden(&fresh_anon, &golden);

    let mut deanon: BTreeMap<String, String> = BTreeMap::new();
    for (site, targets) in &fresh_plain {
        let asite = anonymize_site_key(site, anon::TRIGGER_OP_DOMAIN_V1);
        record_site_deanon(site, &asite, &mut deanon);
        for t in targets {
            let at = anonymize_target(t);
            record_target_deanon(t, &at, &mut deanon);
        }
    }
    merge_deanon_map(&cdo_deanon_map_path(), &deanon);

    let mut hasher = Sha256::new();
    for entry in &golden.entries {
        let fresh_targets = fresh_anon.get(&entry.site).cloned().unwrap_or_default();
        let k_json = serde_json::to_string(&entry.site).unwrap_or_default();
        let l_json = serde_json::to_string(&entry.targets).unwrap_or_default();
        let f_json = serde_json::to_string(&fresh_targets).unwrap_or_default();
        hasher.update(format!("{k_json}|{l_json}|{f_json}\n").as_bytes());
    }
    let digest: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    AnonTriggerAuditReport {
        golden_loaded,
        l3_total,
        fresh_total,
        total_paired: diff.total_paired,
        matches: diff.matches,
        fresh_wrong_count: diff.fresh_wrong.len(),
        fresh_missing: diff.fresh_missing,
        fresh_extra: diff.fresh_extra,
        fresh_novel: diff.fresh_novel,
        golden_missing: diff.golden_missing,
        digest,
    }
}

/// CDO EventFlow frozen-golden audit: compare the fresh resolver's resolved
/// EventFlow publisher→subscriber pairs against the committed, anonymized L3
/// verdict (`cdo-event-anon.json`). Arity-agnostic pair-set comparison only —
/// see [`AnonEventAuditReport`]'s doc comment for scope.
#[must_use]
pub fn run_cdo_event_audit(workspace_root: &Path) -> AnonEventAuditReport {
    let golden = load_anon_event_golden(&cdo_event_anon_golden_path());
    let golden_loaded = golden.is_some();
    let golden = golden.unwrap_or_default();
    let l3_total = golden.entries.len();
    if golden_loaded {
        warn_on_workspace_drift(&golden.metadata, workspace_root);
    }

    let fresh_rows = project_fresh_event_rows(workspace_root);
    let fresh_total = fresh_rows.len();

    let mut deanon: BTreeMap<String, String> = BTreeMap::new();
    let fresh_golden = anonymize_event_rows_with_deanon(&fresh_rows, &mut deanon);
    merge_deanon_map(&cdo_deanon_map_path(), &deanon);

    let l3_pairs: BTreeSet<AnonEventPairKey> =
        golden.entries.iter().map(|e| e.pair.clone()).collect();
    let fresh_pairs: BTreeSet<AnonEventPairKey> = fresh_golden
        .entries
        .iter()
        .map(|e| e.pair.clone())
        .collect();

    let matched_pairs = l3_pairs.intersection(&fresh_pairs).count();
    let pair_l3_only = l3_pairs.difference(&fresh_pairs).count();
    let pair_fresh_only = fresh_pairs.difference(&l3_pairs).count();

    let mut hasher = Sha256::new();
    for pair in l3_pairs.union(&fresh_pairs) {
        let in_l3 = l3_pairs.contains(pair);
        let in_fresh = fresh_pairs.contains(pair);
        let p_json = serde_json::to_string(pair).unwrap_or_default();
        hasher.update(format!("{p_json}|{in_l3}|{in_fresh}\n").as_bytes());
    }
    let digest: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    AnonEventAuditReport {
        golden_loaded,
        l3_total,
        fresh_total,
        matched_pairs,
        pair_l3_only,
        pair_fresh_only,
        digest,
    }
}
