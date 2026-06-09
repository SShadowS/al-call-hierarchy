//! A serde `Serializer` that materializes any `Serialize` value into an
//! insertion-ordered [`CborValue`] tree. This lets the consumed-core typed
//! derivers (whose custom `Serialize` impls already emit the correct per-fact
//! FIELD ORDER) feed the full-snapshot CBOR tree WITHOUT routing through
//! `serde_json::Value` (which would alphabetize map keys — `preserve_order` is OFF
//! for this target — and scramble the cbor-x key order).
//!
//! Map keys are kept in serialization (insertion) order via `IndexMap`. Integers
//! land as `CborValue::Int`; non-integer floats as `CborValue::Float`; `None`/unit
//! as `CborValue::Null`. The snapshot's `skip_serializing_if = "Option::is_none"`
//! fields simply never call `serialize_entry`, so they don't appear — matching
//! al-sem's JS objects where an absent optional is an absent key.

use indexmap::IndexMap;
use serde::ser::{
    Serialize, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
    SerializeTupleStruct, SerializeTupleVariant, Serializer,
};

use crate::engine::gate::cbor::CborValue;

/// Materialize any `Serialize` value into an insertion-ordered `CborValue` tree.
pub fn to_cbor_value<T: ?Sized + Serialize>(value: &T) -> CborValue {
    value.serialize(CborSerializer).unwrap_or(CborValue::Null)
}

/// The serde error type — the serialization is total (no fallible IO), but serde
/// requires an `Error: ser::Error`.
#[derive(Debug)]
pub struct CborSerError(String);

impl std::fmt::Display for CborSerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for CborSerError {}
impl serde::ser::Error for CborSerError {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        CborSerError(msg.to_string())
    }
}

type R = Result<CborValue, CborSerError>;

struct CborSerializer;

impl Serializer for CborSerializer {
    type Ok = CborValue;
    type Error = CborSerError;
    type SerializeSeq = SeqSer;
    type SerializeTuple = SeqSer;
    type SerializeTupleStruct = SeqSer;
    type SerializeTupleVariant = SeqSer;
    type SerializeMap = MapSer;
    type SerializeStruct = StructSer;
    type SerializeStructVariant = StructSer;

    fn serialize_bool(self, v: bool) -> R {
        Ok(CborValue::Bool(v))
    }
    fn serialize_i8(self, v: i8) -> R {
        Ok(CborValue::Int(v as i64))
    }
    fn serialize_i16(self, v: i16) -> R {
        Ok(CborValue::Int(v as i64))
    }
    fn serialize_i32(self, v: i32) -> R {
        Ok(CborValue::Int(v as i64))
    }
    fn serialize_i64(self, v: i64) -> R {
        Ok(CborValue::Int(v))
    }
    fn serialize_u8(self, v: u8) -> R {
        Ok(CborValue::Int(v as i64))
    }
    fn serialize_u16(self, v: u16) -> R {
        Ok(CborValue::Int(v as i64))
    }
    fn serialize_u32(self, v: u32) -> R {
        Ok(CborValue::Int(v as i64))
    }
    fn serialize_u64(self, v: u64) -> R {
        // The snapshot's usize/u64 fields (candidateCount, counts) are small; clamp
        // is unnecessary in practice. Keep full range via i64 where it fits.
        Ok(CborValue::Int(v as i64))
    }
    fn serialize_f32(self, v: f32) -> R {
        self.serialize_f64(v as f64)
    }
    fn serialize_f64(self, v: f64) -> R {
        if v.is_finite() && v == v.trunc() && v.abs() < 9.007_199_254_740_992e15 {
            // Integer-valued float → CBOR int (cbor-x encodes whole numbers as ints).
            Ok(CborValue::Int(v as i64))
        } else {
            Ok(CborValue::Float(v))
        }
    }
    fn serialize_char(self, v: char) -> R {
        Ok(CborValue::Text(v.to_string()))
    }
    fn serialize_str(self, v: &str) -> R {
        Ok(CborValue::Text(v.to_string()))
    }
    fn serialize_bytes(self, v: &[u8]) -> R {
        // No byte fields in the snapshot; represent as an array of ints (faithful).
        Ok(CborValue::Array(
            v.iter().map(|b| CborValue::Int(*b as i64)).collect(),
        ))
    }
    fn serialize_none(self) -> R {
        Ok(CborValue::Null)
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> R {
        value.serialize(self)
    }
    fn serialize_unit(self) -> R {
        Ok(CborValue::Null)
    }
    fn serialize_unit_struct(self, _name: &'static str) -> R {
        Ok(CborValue::Null)
    }
    fn serialize_unit_variant(self, _name: &'static str, _index: u32, variant: &'static str) -> R {
        Ok(CborValue::Text(variant.to_string()))
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(self, _name: &'static str, value: &T) -> R {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        value: &T,
    ) -> R {
        let mut m = IndexMap::new();
        m.insert(variant.to_string(), to_cbor_value(value));
        Ok(CborValue::Map(m))
    }
    fn serialize_seq(self, _len: Option<usize>) -> Result<SeqSer, CborSerError> {
        Ok(SeqSer { items: Vec::new() })
    }
    fn serialize_tuple(self, _len: usize) -> Result<SeqSer, CborSerError> {
        Ok(SeqSer { items: Vec::new() })
    }
    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<SeqSer, CborSerError> {
        Ok(SeqSer { items: Vec::new() })
    }
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<SeqSer, CborSerError> {
        Ok(SeqSer { items: Vec::new() })
    }
    fn serialize_map(self, _len: Option<usize>) -> Result<MapSer, CborSerError> {
        Ok(MapSer {
            entries: IndexMap::new(),
            next_key: None,
        })
    }
    fn serialize_struct(self, _name: &'static str, _len: usize) -> Result<StructSer, CborSerError> {
        Ok(StructSer {
            entries: IndexMap::new(),
        })
    }
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<StructSer, CborSerError> {
        Ok(StructSer {
            entries: IndexMap::new(),
        })
    }
}

struct SeqSer {
    items: Vec<CborValue>,
}
impl SerializeSeq for SeqSer {
    type Ok = CborValue;
    type Error = CborSerError;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), CborSerError> {
        self.items.push(to_cbor_value(value));
        Ok(())
    }
    fn end(self) -> R {
        Ok(CborValue::Array(self.items))
    }
}
impl SerializeTuple for SeqSer {
    type Ok = CborValue;
    type Error = CborSerError;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), CborSerError> {
        self.items.push(to_cbor_value(value));
        Ok(())
    }
    fn end(self) -> R {
        Ok(CborValue::Array(self.items))
    }
}
impl SerializeTupleStruct for SeqSer {
    type Ok = CborValue;
    type Error = CborSerError;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), CborSerError> {
        self.items.push(to_cbor_value(value));
        Ok(())
    }
    fn end(self) -> R {
        Ok(CborValue::Array(self.items))
    }
}
impl SerializeTupleVariant for SeqSer {
    type Ok = CborValue;
    type Error = CborSerError;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), CborSerError> {
        self.items.push(to_cbor_value(value));
        Ok(())
    }
    fn end(self) -> R {
        Ok(CborValue::Array(self.items))
    }
}

struct MapSer {
    entries: IndexMap<String, CborValue>,
    next_key: Option<String>,
}
impl SerializeMap for MapSer {
    type Ok = CborValue;
    type Error = CborSerError;
    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), CborSerError> {
        // Keys are text strings (the snapshot's maps are all string-keyed). The
        // non-text fallback is unreachable-by-construction; the debug_assert ENFORCES
        // that invariant rather than silently stringifying a non-text key.
        self.next_key = Some(match to_cbor_value(key) {
            CborValue::Text(s) => s,
            other => {
                debug_assert!(
                    false,
                    "to_cbor MapSer: non-text map key encountered ({other:?}); \
                     snapshot maps must be string-keyed"
                );
                format!("{other:?}")
            }
        });
        Ok(())
    }
    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), CborSerError> {
        if let Some(k) = self.next_key.take() {
            self.entries.insert(k, to_cbor_value(value));
        }
        Ok(())
    }
    fn end(self) -> R {
        Ok(CborValue::Map(self.entries))
    }
}

struct StructSer {
    entries: IndexMap<String, CborValue>,
}
impl SerializeStruct for StructSer {
    type Ok = CborValue;
    type Error = CborSerError;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), CborSerError> {
        self.entries.insert(key.to_string(), to_cbor_value(value));
        Ok(())
    }
    fn end(self) -> R {
        Ok(CborValue::Map(self.entries))
    }
}
impl SerializeStructVariant for StructSer {
    type Ok = CborValue;
    type Error = CborSerError;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), CborSerError> {
        self.entries.insert(key.to_string(), to_cbor_value(value));
        Ok(())
    }
    fn end(self) -> R {
        Ok(CborValue::Map(self.entries))
    }
}
