//! Hand-rolled CBOR encoder — byte-compatible with `cbor-x` 1.6.4 encoding
//! `Encoder({ useRecords:false, mapsAsObjects:true, pack:false }).encode(obj)`.
//!
//! al-sem serializes the `CapabilitySnapshot` to CBOR via cbor-x for the
//! `.cbor` / `.cbor.gz` snapshot goldens (Stage cli-b). No Rust CBOR crate
//! reproduces cbor-x's NON-CANONICAL choices (always-map-16 header, insertion
//! key order, undefined slots kept), so this module implements the exact byte
//! rules, proven against cbor-x by the `cbor_oracles` tests below.
//!
//! ## Exact cbor-x byte rules (proven against cbor-x 1.6.4)
//!
//! - object → `0xb9` + `(count as u16).to_be_bytes()` ALWAYS (the non-canonical
//!   always-map-16 header, even for 0/1 entries), then each entry: key as a
//!   text-string, value. Keys in INSERTION order; keys are text strings.
//! - integer → smallest CBOR int (major 0 for >=0, major 1 for <0 encoding -1-n).
//! - non-integer number → ALWAYS `0xfb` + `f64::to_be_bytes()` (no float32/16).
//! - string → major 3 (`0x60|len` / `0x78` / `0x79` / `0x7a`) + UTF-8 bytes.
//! - array → major 4 (`0x80|len` / `0x98` / `0x99` / `0x9a`) + elements.
//! - bool → `0xf5`/`0xf4`, null → `0xf6`, undefined → `0xf7` (KEY KEPT — emit the
//!   `f7` slot, the same as cbor-x with `mapsAsObjects:true`).
//! - no tags.

use indexmap::IndexMap;

/// An insertion-ordered CBOR value model. Objects MUST preserve insertion order
/// (never a sorted/Hash map) — CBOR byte parity depends on the key order, which
/// mirrors al-sem's `composeSnapshot` literal key order.
#[derive(Debug, Clone)]
pub enum CborValue {
    /// A JSON/JS `null` → `0xf6`.
    Null,
    /// A JS `undefined` → `0xf7`. The KEY is still emitted (cbor-x keeps the slot
    /// under `mapsAsObjects:true`).
    Undefined,
    Bool(bool),
    /// An integer (encoded as the smallest CBOR int).
    Int(i64),
    /// A non-integer number — ALWAYS encoded as `0xfb` + f64 BE bytes.
    Float(f64),
    Text(String),
    Array(Vec<CborValue>),
    /// An object — insertion-ordered key/value pairs. Keys are text strings.
    Map(IndexMap<String, CborValue>),
}

impl CborValue {
    /// Convenience: an empty insertion-ordered map.
    pub fn map() -> CborValue {
        CborValue::Map(IndexMap::new())
    }
}

/// Encode a `CborValue` to its cbor-x-compatible byte form.
pub fn encode(value: &CborValue) -> Vec<u8> {
    let mut out = Vec::new();
    encode_into(value, &mut out);
    out
}

fn encode_into(value: &CborValue, out: &mut Vec<u8>) {
    match value {
        CborValue::Null => out.push(0xf6),
        CborValue::Undefined => out.push(0xf7),
        CborValue::Bool(true) => out.push(0xf5),
        CborValue::Bool(false) => out.push(0xf4),
        CborValue::Int(n) => encode_int(*n, out),
        CborValue::Float(f) => {
            out.push(0xfb);
            out.extend_from_slice(&f.to_be_bytes());
        }
        CborValue::Text(s) => encode_text(s, out),
        CborValue::Array(items) => {
            encode_array_header(items.len(), out);
            for item in items {
                encode_into(item, out);
            }
        }
        CborValue::Map(entries) => {
            // ALWAYS the non-canonical map-16 header: 0xb9 + (count as u16) BE,
            // even for 0/1 entries (proven against cbor-x).
            out.push(0xb9);
            let count = entries.len() as u16;
            out.extend_from_slice(&count.to_be_bytes());
            for (k, v) in entries {
                // Keys are ALWAYS text strings (smallest-length text header).
                encode_text(k, out);
                encode_into(v, out);
            }
        }
    }
}

/// Encode an integer with the smallest CBOR major-0/major-1 header.
fn encode_int(n: i64, out: &mut Vec<u8>) {
    if n >= 0 {
        encode_uint_header(0x00, n as u64, out);
    } else {
        // major 1 encodes -1-n (i.e. the magnitude minus one).
        let mag = (-(n + 1)) as u64;
        encode_uint_header(0x20, mag, out);
    }
}

/// Emit `major | argument` using the smallest CBOR additional-info encoding.
/// `major` is the major-type bits already shifted into the high 3 bits (e.g.
/// `0x00` for unsigned, `0x20` for negative, `0x60` for text, `0x80` for array).
fn encode_uint_header(major: u8, n: u64, out: &mut Vec<u8>) {
    if n <= 23 {
        out.push(major | (n as u8));
    } else if n <= 0xff {
        out.push(major | 0x18);
        out.push(n as u8);
    } else if n <= 0xffff {
        out.push(major | 0x19);
        out.extend_from_slice(&(n as u16).to_be_bytes());
    } else if n <= 0xffff_ffff {
        out.push(major | 0x1a);
        out.extend_from_slice(&(n as u32).to_be_bytes());
    } else {
        out.push(major | 0x1b);
        out.extend_from_slice(&n.to_be_bytes());
    }
}

fn encode_text(s: &str, out: &mut Vec<u8>) {
    let bytes = s.as_bytes();
    encode_uint_header(0x60, bytes.len() as u64, out);
    out.extend_from_slice(bytes);
}

fn encode_array_header(len: usize, out: &mut Vec<u8>) {
    encode_uint_header(0x80, len as u64, out);
}

#[cfg(test)]
mod cbor_oracles {
    //! Hex-diff oracles for each encoding rule. The expected bytes are the
    //! VERBATIM output of cbor-x 1.6.4
    //! `Encoder({useRecords:false,mapsAsObjects:true,pack:false}).encode(x)`,
    //! captured by `bun -e 'import{Encoder}from"cbor-x";...'` and pinned here.
    use super::*;

    fn m(pairs: &[(&str, CborValue)]) -> CborValue {
        let mut map = IndexMap::new();
        for (k, v) in pairs {
            map.insert((*k).to_string(), v.clone());
        }
        CborValue::Map(map)
    }

    fn hex(bytes: &[u8]) -> String {
        bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn empty_object_is_always_map16() {
        // cbor-x: {} → b9 00 00
        assert_eq!(hex(&encode(&CborValue::map())), "b9 00 00");
    }

    #[test]
    fn single_key_int() {
        // cbor-x: {a:1} → b9 00 01 61 61 01
        let v = m(&[("a", CborValue::Int(1))]);
        assert_eq!(hex(&encode(&v)), "b9 00 01 61 61 01");
    }

    #[test]
    fn bools_and_null() {
        // cbor-x: {a:true,b:false,c:null} → b9 00 03 61 61 f5 61 62 f4 61 63 f6
        let v = m(&[
            ("a", CborValue::Bool(true)),
            ("b", CborValue::Bool(false)),
            ("c", CborValue::Null),
        ]);
        assert_eq!(hex(&encode(&v)), "b9 00 03 61 61 f5 61 62 f4 61 63 f6");
    }

    #[test]
    fn undefined_value_keeps_key() {
        // cbor-x: {a:undefined,b:1} → b9 00 02 61 61 f7 61 62 01
        let v = m(&[("a", CborValue::Undefined), ("b", CborValue::Int(1))]);
        assert_eq!(hex(&encode(&v)), "b9 00 02 61 61 f7 61 62 01");
    }

    #[test]
    fn nested_array_with_big_int() {
        // cbor-x: {x:[1,2,300]} → b9 00 01 61 78 83 01 02 19 01 2c
        let v = m(&[(
            "x",
            CborValue::Array(vec![
                CborValue::Int(1),
                CborValue::Int(2),
                CborValue::Int(300),
            ]),
        )]);
        assert_eq!(hex(&encode(&v)), "b9 00 01 61 78 83 01 02 19 01 2c");
    }

    #[test]
    fn integer_boundaries() {
        // cbor-x: {a:23,b:24,c:255,d:256,e:65535,f:65536,g:-1,h:-100,i:-300} →
        // b9 00 09 61 61 17 61 62 18 18 61 63 18 ff 61 64 19 01 00 61 65 19 ff ff
        // 61 66 1a 00 01 00 00 61 67 20 61 68 38 63 61 69 39 01 2b
        let v = m(&[
            ("a", CborValue::Int(23)),
            ("b", CborValue::Int(24)),
            ("c", CborValue::Int(255)),
            ("d", CborValue::Int(256)),
            ("e", CborValue::Int(65535)),
            ("f", CborValue::Int(65536)),
            ("g", CborValue::Int(-1)),
            ("h", CborValue::Int(-100)),
            ("i", CborValue::Int(-300)),
        ]);
        assert_eq!(
            hex(&encode(&v)),
            "b9 00 09 61 61 17 61 62 18 18 61 63 18 ff 61 64 19 01 00 \
             61 65 19 ff ff 61 66 1a 00 01 00 00 61 67 20 61 68 38 63 61 69 39 01 2b"
                .replace("             ", "")
        );
    }

    #[test]
    fn float_is_always_f64() {
        // cbor-x: {a:1.5} → b9 00 01 61 61 fb 3f f8 00 00 00 00 00 00
        let v = m(&[("a", CborValue::Float(1.5))]);
        assert_eq!(
            hex(&encode(&v)),
            "b9 00 01 61 61 fb 3f f8 00 00 00 00 00 00"
        );
    }

    #[test]
    fn string_value() {
        // cbor-x: {k:"hello"} → b9 00 01 61 6b 65 68 65 6c 6c 6f
        let v = m(&[("k", CborValue::Text("hello".to_string()))]);
        assert_eq!(hex(&encode(&v)), "b9 00 01 61 6b 65 68 65 6c 6c 6f");
    }

    #[test]
    fn map_header_for_24_keys() {
        // cbor-x: 24-key object header → b9 00 18 …
        let mut map = IndexMap::new();
        for i in 0..24 {
            map.insert(format!("k{i}"), CborValue::Int(i));
        }
        let bytes = encode(&CborValue::Map(map));
        assert_eq!(hex(&bytes[..3]), "b9 00 18");
    }
}
