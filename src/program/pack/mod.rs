//! Dependency pack codec — one app's extraction output, to bytes and back.
//!
//! Persistence only. No engine wiring lives here: obtaining a pack, keying it
//! and loading it into `build_dep_layer` is spec step 5. This module exists so
//! the format's round-trip cost can be measured before that work starts, per
//! `docs/superpowers/specs/2026-08-07-dependency-pack-cache-design.md` §13.
//!
//! Kept separate from `node_extract.rs` deliberately: extraction logic and the
//! persistence format have different reasons to change, and the spec's
//! EXTRACTION_FINGERPRINT closure (§8) names this module in its own right.
//!
//! # Two costs a reader of the measurement should know about
//!
//! 1. **`decode` re-serializes the whole pack to verify `self_hash`.** The hash
//!    is defined as blake3 over the postcard encoding of every field except
//!    itself, so checking it means producing those bytes again. That is a real,
//!    inherent cost of THIS container shape, not of postcard: an envelope that
//!    stored the hash over the already-encoded body would verify at
//!    memcpy-speed and yield the identical `self_hash` value, without changing
//!    this module's public interface. Recorded here so a slow decode number is
//!    attributed to the right thing rather than to the format as a whole.
//! 2. **Nothing here writes to a cache directory, and nothing should yet.** The
//!    key (spec §7) and the behavioural canary (§8) are what make a real cache
//!    directory safe; until they exist, a pack on disk under a wrong key is a
//!    stale pack served as fresh. `encode`/`decode` are pure byte functions —
//!    keep them that way.

use serde::{Deserialize, Serialize};

use al_syntax::ir::{Origin, Point};
use al_syntax::raw::RawKind;

use crate::program::node::RoutineNodeId;
use crate::program::node_extract::{ObjectNode, RoutineNode};
use crate::program::resolve::decl_surface::RoutineMeta;

/// Bump when `DepPack`'s or `PackedFile`'s shape changes. Old packs then fail
/// the check in [`decode`] and are recomputed. Never migrate in place.
pub const PACK_SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackedFile {
    pub virtual_path: String,
    /// `AlFile::parse_status == ParseStatus::Recovered` for this file. Stored
    /// rather than recomputed: on a pack hit the file is never parsed, and
    /// `snapshot::parse::recovered_file_paths` is the load-bearing
    /// absence-proof diagnostic that must still see it (spec §11.2).
    pub parse_status_recovered: bool,
    pub objects: Vec<ObjectNode>,
    pub routines: Vec<RoutineNode>,
    /// Per-routine decl metadata, keyed as `DeclSurface` keys it. NOT derivable
    /// on a pack hit: `RoutineMeta::from_decl` consumes a `RoutineDecl`, and the
    /// `ParsedUnit`s those come from are exactly what a hit avoids building.
    /// Nothing in `RoutineNode` can reconstruct `virtual_path`, the two
    /// `Origin`s, or per-param `ty`/`by_ref` (spec §4).
    pub routine_meta: Vec<(RoutineNodeId, RoutineMeta)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DepPack {
    pub schema: u32,
    /// App identity, SYMBOLIC. Never an `AppRef` — that is a per-run interning
    /// index (`src/program/build.rs`) and persisting one yields silently-wrong
    /// graphs rather than errors.
    pub app_guid: String,
    pub app_name: String,
    pub app_publisher: String,
    pub app_version: String,
    /// Per-file contributions in extraction order, pre-dedup. Order is
    /// load-bearing: `dedup_routines_preserving_genuine_overloads` keeps the
    /// first occurrence per key, so a reordered pack changes which survivor
    /// wins (spec §12).
    pub files: Vec<PackedFile>,
    /// blake3 over the postcard encoding of everything above. Cheap
    /// belt-and-suspenders against bit-rot and hand edits, and what makes the
    /// corrupted-payload test expressible.
    pub self_hash: String,
}

#[derive(Debug)]
pub enum PackError {
    Codec(String),
    SchemaMismatch { found: u32, expected: u32 },
    SelfHashMismatch,
}

impl std::fmt::Display for PackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackError::Codec(e) => write!(f, "pack codec error: {e}"),
            PackError::SchemaMismatch { found, expected } => {
                write!(f, "pack schema {found}, expected {expected}")
            }
            PackError::SelfHashMismatch => write!(f, "pack self-hash mismatch"),
        }
    }
}

impl std::error::Error for PackError {}

// ---------------------------------------------------------------------------
// Origin wire codec
// ---------------------------------------------------------------------------

/// The wire shape of an [`Origin`].
///
/// `Origin` itself stays exactly as the IR defines it: its `kind_text` is a
/// `&'static str` fed verbatim to anchor `syntax_kind` for parity, so widening
/// it to `String` (≈2 heap allocations per routine, forever, to carry a value
/// from a closed 389-member set) or dropping it (an unaudited lossy field) were
/// both rejected. `&'static str` cannot `Deserialize`; a `RawKind` can, and
/// [`RawKind::as_str`] hands back a `&'static str` on the way out — so decoding
/// an `Origin` allocates NOTHING and the wire carries a varint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackedOrigin {
    /// `kind_text` as a closed-set discriminant.
    #[serde(with = "raw_kind_wire")]
    pub kind: RawKind,
    pub byte_start: usize,
    pub byte_end: usize,
    pub start: (u32, u32),
    pub end: (u32, u32),
}

impl PackedOrigin {
    /// `None` when `kind_text` is not a named grammar kind — the app is then
    /// not packed. Cold path, fail-closed, never a panic. (`RawNode::kind_str`
    /// returns the raw kind of ANY node, named or anonymous, and nothing at the
    /// type level restricts which nodes an `Origin` is minted from — which is
    /// why this is fallible rather than a `RawKind` field in the IR.)
    #[must_use]
    pub fn try_from_origin(o: &Origin) -> Option<Self> {
        Some(PackedOrigin {
            kind: RawKind::try_from_raw(o.kind_text)?,
            byte_start: o.byte.start,
            byte_end: o.byte.end,
            start: (o.start.row, o.start.column),
            end: (o.end.row, o.end.column),
        })
    }

    /// Infallible: every `RawKind` has a canonical kind string.
    #[must_use]
    pub fn into_origin(self) -> Origin {
        Origin {
            kind_text: self.kind.as_str(),
            byte: self.byte_start..self.byte_end,
            start: Point {
                row: self.start.0,
                column: self.start.1,
            },
            end: Point {
                row: self.end.0,
                column: self.end.1,
            },
        }
    }
}

/// `RawKind` as a varint index into [`RawKind::ALL`].
///
/// POSITIONAL, and therefore not stable across grammar revisions — a kind added
/// mid-alphabet shifts every later index. That is sound here only because
/// `crates/` is inside the pack fingerprint closure (spec §8), so regenerating
/// the raw vocabulary invalidates every pack. An out-of-range index decodes to
/// an error, never to a wrong kind.
mod raw_kind_wire {
    use al_syntax::raw::RawKind;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    // `&RawKind` rather than `RawKind` because serde's `with` module requires
    // this exact signature.
    pub fn serialize<S: Serializer>(k: &RawKind, s: S) -> Result<S::Ok, S::Error> {
        // `RawKind` is fieldless with implicit discriminants, so `as u32` is
        // its declaration index; `RawKind::ALL[k as usize] == k` is asserted
        // exhaustively in al-syntax's own `raw` tests.
        (*k as u32).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<RawKind, D::Error> {
        let i = u32::deserialize(d)?;
        RawKind::ALL
            .get(i as usize)
            .copied()
            .ok_or_else(|| serde::de::Error::custom(format!("raw kind index {i} out of range")))
    }
}

/// `Origin` through [`PackedOrigin`], for `#[serde(with = ...)]` on any field
/// holding one. Fallible on encode (an unencodable kind means the app is not
/// packed), infallible on decode.
pub mod origin_wire {
    use super::PackedOrigin;
    use al_syntax::ir::Origin;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(o: &Origin, s: S) -> Result<S::Ok, S::Error> {
        match PackedOrigin::try_from_origin(o) {
            Some(p) => p.serialize(s),
            None => Err(serde::ser::Error::custom(format!(
                "origin kind {:?} is not a named grammar kind",
                o.kind_text
            ))),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Origin, D::Error> {
        Ok(PackedOrigin::deserialize(d)?.into_origin())
    }
}

// ---------------------------------------------------------------------------
// Codec
// ---------------------------------------------------------------------------

/// Every `DepPack` field except `self_hash`, borrowed. Hashing this rather than
/// a blanked clone keeps [`compute_self_hash`] off the O(pack) copy that a
/// `pack.clone()` would pay on the very path the gate is timing.
#[derive(Serialize)]
struct PackBody<'a> {
    schema: u32,
    app_guid: &'a str,
    app_name: &'a str,
    app_publisher: &'a str,
    app_version: &'a str,
    files: &'a [PackedFile],
}

/// blake3 over every field except `self_hash` itself.
///
/// Returns the hash of an empty input if the pack contains an unencodable
/// `Origin` kind; that pack cannot be [`encode`]d either, so the failure is
/// still surfaced — loudly and with the offending kind named — there.
#[must_use]
pub fn compute_self_hash(pack: &DepPack) -> String {
    // Exhaustive destructure ON PURPOSE: a field added to `DepPack` becomes a
    // compile error here rather than a field that silently escapes the hash.
    let DepPack {
        schema,
        app_guid,
        app_name,
        app_publisher,
        app_version,
        files,
        self_hash: _,
    } = pack;
    let body = PackBody {
        schema: *schema,
        app_guid,
        app_name,
        app_publisher,
        app_version,
        files,
    };
    let bytes = postcard::to_stdvec(&body).unwrap_or_default();
    blake3::hash(&bytes).to_hex().to_string()
}

/// Serialize a pack. The caller must have set `self_hash` via
/// [`compute_self_hash`] first.
pub fn encode(pack: &DepPack) -> Result<Vec<u8>, PackError> {
    postcard::to_stdvec(pack).map_err(|e| match e {
        // postcard DISCARDS `ser::Error::custom` messages (its `Error::custom`
        // takes `_msg`), so the only way to say WHICH origin failed is to go
        // find it. Done here, on the failure path only, so the diagnostic costs
        // nothing on the path that succeeds.
        postcard::Error::SerdeSerCustom => PackError::Codec(unencodable_origin_report(pack)),
        other => PackError::Codec(other.to_string()),
    })
}

/// Name the first `Origin` whose kind cannot be encoded. Failure path only.
fn unencodable_origin_report(pack: &DepPack) -> String {
    for file in &pack.files {
        for (_, meta) in &file.routine_meta {
            for (which, origin) in [("origin", &meta.origin), ("name_origin", &meta.name_origin)] {
                if RawKind::try_from_raw(origin.kind_text).is_none() {
                    return format!(
                        "{}: routine {} {which} kind {:?} is not a named grammar kind, so this \
                         app is not packable",
                        file.virtual_path, meta.name, origin.kind_text
                    );
                }
            }
        }
    }
    "serde rejected the pack, but no unencodable Origin kind was found".to_string()
}

/// Deserialize a pack, checking schema then integrity.
///
/// Every abnormal state is an `Err`, never a partial `Ok` — the consumer in
/// spec step 5 treats any `Err` as a cache miss and recomputes.
pub fn decode(bytes: &[u8]) -> Result<DepPack, PackError> {
    let pack: DepPack = postcard::from_bytes(bytes).map_err(|e| PackError::Codec(e.to_string()))?;
    if pack.schema != PACK_SCHEMA {
        return Err(PackError::SchemaMismatch {
            found: pack.schema,
            expected: PACK_SCHEMA,
        });
    }
    if compute_self_hash(&pack) != pack.self_hash {
        return Err(PackError::SelfHashMismatch);
    }
    Ok(pack)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::node::{ObjKey, ObjectNodeId};
    use crate::program::resolve::decl_surface::ParamMeta;
    use al_syntax::ir::ObjectKind;

    fn origin(kind_text: &'static str, lo: usize, hi: usize) -> Origin {
        Origin {
            kind_text,
            byte: lo..hi,
            start: Point { row: 3, column: 4 },
            end: Point { row: 5, column: 6 },
        }
    }

    fn sample_routine_id() -> RoutineNodeId {
        RoutineNodeId {
            object: ObjectNodeId {
                app: crate::program::node::AppRef(0),
                kind: ObjectKind::Codeunit,
                key: ObjKey::Id(80),
            },
            name_lc: "postsalesdocument".into(),
            enclosing_member_lc: Some("no.".into()),
            // 2^53 + 1 — outside JSON's exactly-representable integer range, so
            // this also keeps `RoutineNodeId`'s decimal-string `sig_fp` codec
            // under a value that would round if it were ever treated as a
            // double.
            params_count: 2,
            sig_fp: 9_007_199_254_740_993,
        }
    }

    /// A fully-populated `RoutineMeta`: both `Origin`s and a non-empty
    /// `params` with a `Some(ty)` and a `None` ty. An empty fixture here would
    /// make the entire `Origin` codec — the substance of this module —
    /// round-trip vacuously.
    fn sample_routine_meta() -> RoutineMeta {
        RoutineMeta {
            name: "PostSalesDocument".into(),
            enclosing_member: Some("No.".into()),
            parse_incomplete: true,
            params: vec![
                ParamMeta {
                    ty: Some("Record \"Sales Header\"".into()),
                    by_ref: true,
                },
                ParamMeta {
                    ty: None,
                    by_ref: false,
                },
            ],
            origin: origin("procedure", 100, 240),
            name_origin: origin("identifier", 110, 127),
            virtual_path: "src/Sales/SalesPost.Codeunit.al".into(),
        }
    }

    fn sample_pack() -> DepPack {
        DepPack {
            schema: PACK_SCHEMA,
            app_guid: "437dbf0e-84ff-417a-965d-ed2bb9650972".to_string(),
            app_name: "Base Application".to_string(),
            app_publisher: "Microsoft".to_string(),
            app_version: "28.2.0.0".to_string(),
            files: vec![PackedFile {
                virtual_path: "src/Sales/SalesPost.Codeunit.al".to_string(),
                parse_status_recovered: true,
                objects: vec![],
                routines: vec![],
                routine_meta: vec![(sample_routine_id(), sample_routine_meta())],
            }],
            self_hash: String::new(),
        }
    }

    #[test]
    fn encode_decode_round_trips() {
        let mut pack = sample_pack();
        pack.self_hash = compute_self_hash(&pack);
        let bytes = encode(&pack).expect("encode");
        let back = decode(&bytes).expect("decode");
        assert_eq!(back.app_guid, pack.app_guid);
        assert_eq!(back.files.len(), 1);
        assert_eq!(back.files[0].virtual_path, pack.files[0].virtual_path);
        assert!(back.files[0].parse_status_recovered);
        // Total, not field-by-field: a field added to `DepPack`/`PackedFile`/
        // `RoutineMeta` later is covered without editing this assertion.
        assert_eq!(back, pack);
    }

    /// The `Origin` codec specifically: a decoded `kind_text` must be the same
    /// `&'static str` the IR would have produced, and every position field must
    /// survive. `assert_eq!(back, pack)` above covers this too, but this test
    /// says WHICH property is load-bearing, and fails with a readable message.
    #[test]
    fn origin_round_trips_through_the_raw_kind_discriminant() {
        let o = origin("trigger_declaration", 7, 91);
        let packed = PackedOrigin::try_from_origin(&o).expect("named kind must encode");
        assert_eq!(packed.kind, RawKind::TriggerDeclaration);
        let back = packed.into_origin();
        assert_eq!(back, o);
        // The decoded string is the grammar's own `&'static str`, not a copy —
        // this is the property that makes decode allocation-free.
        assert_eq!(back.kind_text, RawKind::TriggerDeclaration.as_str());
    }

    /// Hand-stated precondition: an `Origin` whose `kind_text` is not a named
    /// grammar kind. Production cannot currently produce one (see
    /// `every_decl_origin_kind_is_a_named_grammar_kind`), which is exactly why
    /// the fixture is constructed literally rather than obtained from a parse.
    #[test]
    fn an_unencodable_origin_kind_fails_the_pack_instead_of_panicking() {
        assert_eq!(
            PackedOrigin::try_from_origin(&origin("definitely_not_a_real_kind", 0, 1)),
            None
        );

        let mut pack = sample_pack();
        pack.files[0].routine_meta[0].1.origin = origin("definitely_not_a_real_kind", 0, 1);
        pack.self_hash = compute_self_hash(&pack);
        match encode(&pack) {
            Err(PackError::Codec(msg)) => {
                // Fail-closed AND diagnosable: the message must name the kind,
                // or an unpackable app becomes a mystery cache-miss rate.
                assert!(
                    msg.contains("definitely_not_a_real_kind"),
                    "codec error must name the offending kind, got: {msg}"
                );
                assert!(
                    msg.contains("PostSalesDocument"),
                    "codec error must name the offending routine, got: {msg}"
                );
            }
            other => panic!("expected PackError::Codec, got {other:?}"),
        }
    }

    /// STATED LIMIT of `a_corrupted_payload_is_rejected_not_misread`: deleting
    /// the `self_hash` check leaves that test GREEN, because the `^= 0xFF` flip
    /// it makes lands in UTF-8 string content and postcard's own decoder
    /// rejects it. That is a real property of the code, not a broken test — so
    /// the guard is pinned HERE instead, on the corruption class postcard
    /// ACCEPTS. postcard is not self-describing: most single-bit flips are a
    /// perfectly well-formed encoding of DIFFERENT values, and nothing but the
    /// integrity check distinguishes them from the truth.
    #[test]
    fn the_self_hash_catches_corruption_postcard_accepts() {
        let mut pack = sample_pack();
        pack.self_hash = compute_self_hash(&pack);
        let bytes = encode(&pack).expect("encode");

        let mut caught_by_hash = 0usize;
        for i in 0..bytes.len() {
            let mut c = bytes.clone();
            c[i] ^= 0x01;
            // Only the bytes postcard is happy to decode are this test's
            // population; the rest are already covered by the Codec arm.
            let Ok(decoded) = postcard::from_bytes::<DepPack>(&c) else {
                continue;
            };
            // A flip that decodes back to the identical pack corrupted nothing
            // observable, so accepting it is correct.
            if decoded == pack {
                continue;
            }
            // A flip landing in the leading `schema` varint is the SCHEMA
            // check's population (pinned by
            // `a_pack_from_a_different_schema_is_rejected`), and it fires
            // first. Excluded so this test isolates the self-hash.
            if decoded.schema != PACK_SCHEMA {
                continue;
            }
            assert!(
                matches!(decode(&c), Err(PackError::SelfHashMismatch)),
                "byte {i}: postcard accepted the corruption and decoded a DIFFERENT pack, \
                 so only the self-hash can reject it — but decode() did not"
            );
            caught_by_hash += 1;
        }
        assert!(
            caught_by_hash > 0,
            "no single-bit flip survived postcard, so this test proved nothing about the \
             self-hash — the fixture is too small or too fragile to carry the guard"
        );
        println!(
            "the_self_hash_catches_corruption_postcard_accepts: {caught_by_hash} of {} \
             single-bit flips were caught by the self-hash alone",
            bytes.len()
        );
    }

    #[test]
    fn a_corrupted_payload_is_rejected_not_misread() {
        let mut pack = sample_pack();
        pack.self_hash = compute_self_hash(&pack);
        let mut bytes = encode(&pack).expect("encode");
        // Flip one byte deep in the payload, past the schema prefix.
        // NOTE: this specific flip is caught by postcard itself, not by the
        // self-hash — see `the_self_hash_catches_corruption_postcard_accepts`,
        // which pins the integrity check on the class postcard accepts.
        let idx = bytes.len() / 2;
        bytes[idx] ^= 0xFF;
        match decode(&bytes) {
            Err(PackError::SelfHashMismatch) | Err(PackError::Codec(_)) => {}
            Err(other) => panic!("unexpected error: {other:?}"),
            Ok(_) => panic!("a corrupted pack decoded successfully — integrity check is absent"),
        }
    }

    #[test]
    fn a_pack_from_a_different_schema_is_rejected() {
        let mut pack = sample_pack();
        pack.schema = PACK_SCHEMA + 1;
        pack.self_hash = compute_self_hash(&pack);
        let bytes = encode(&pack).expect("encode");
        match decode(&bytes) {
            Err(PackError::SchemaMismatch { found, expected }) => {
                assert_eq!(found, PACK_SCHEMA + 1);
                assert_eq!(expected, PACK_SCHEMA);
            }
            other => panic!("expected SchemaMismatch, got {other:?}"),
        }
    }

    /// `self_hash` must cover every field of `DepPack`. The exhaustive
    /// destructure in `compute_self_hash` makes a MISSING field a compile
    /// error; this makes a field that IS hashed observably load-bearing.
    #[test]
    fn the_self_hash_covers_the_body_fields() {
        let base = sample_pack();
        let h = compute_self_hash(&base);
        for mutate in [
            (|p: &mut DepPack| p.app_guid.push('x')) as fn(&mut DepPack),
            |p: &mut DepPack| p.app_version.push('x'),
            |p: &mut DepPack| p.files[0].virtual_path.push('x'),
            |p: &mut DepPack| p.files[0].parse_status_recovered = false,
            |p: &mut DepPack| p.files[0].routine_meta[0].1.name.push('x'),
            |p: &mut DepPack| p.files[0].routine_meta[0].1.origin.byte.end += 1,
            |p: &mut DepPack| p.files[0].routine_meta[0].1.params[0].by_ref = false,
            |p: &mut DepPack| p.files[0].routine_meta[0].0.sig_fp += 1,
        ] {
            let mut p = base.clone();
            mutate(&mut p);
            assert_ne!(compute_self_hash(&p), h, "a body change must move the hash");
        }
        // `self_hash` itself is NOT part of its own input.
        let mut p = base.clone();
        p.self_hash = "not the real hash".into();
        assert_eq!(compute_self_hash(&p), h);
    }

    /// Every `.al` file under the two committed fixture corpora.
    fn fixture_al_files() -> Vec<std::path::PathBuf> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
        let mut out: Vec<std::path::PathBuf> = ["r0-corpus", "fixtures"]
            .iter()
            .flat_map(|d| walkdir::WalkDir::new(root.join(d)).into_iter().flatten())
            .filter(|e| e.file_type().is_file())
            .map(walkdir::DirEntry::into_path)
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("al")))
            .collect();
        out.sort();
        out
    }

    #[test]
    fn every_decl_origin_kind_is_a_named_grammar_kind() {
        // Lower a real fixture corpus and assert both Origins on every routine decl
        // survive try_from_raw. If this ever fails, the encoder's None arm is live
        // and that app silently stops being packable -- which is safe, but we want
        // to KNOW, not discover it as a mystery cache-miss rate.
        let files = fixture_al_files();
        assert!(
            !files.is_empty(),
            "no .al fixtures found -- the corpus walk is broken, not the corpus"
        );
        let mut checked = 0usize;
        for path in files {
            let file = al_syntax::parse(&std::fs::read_to_string(&path).unwrap());
            for obj in &file.objects {
                for r in &obj.routines {
                    assert!(
                        RawKind::try_from_raw(r.origin.kind_text).is_some(),
                        "{}: routine {} origin kind {:?} is not a named grammar kind",
                        path.display(),
                        r.name,
                        r.origin.kind_text
                    );
                    assert!(
                        RawKind::try_from_raw(r.name_origin.kind_text).is_some(),
                        "{}: routine {} name_origin kind {:?} is not a named grammar kind",
                        path.display(),
                        r.name,
                        r.name_origin.kind_text
                    );
                    checked += 1;
                }
            }
        }
        assert!(
            checked > 0,
            "the corpus produced no routines -- the test proved nothing"
        );
        // Printed so the count is an observed number in the run log, not a
        // claim: `cargo test -- --nocapture`.
        println!("every_decl_origin_kind_is_a_named_grammar_kind: checked {checked} routines");
    }
}
