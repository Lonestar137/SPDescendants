//! The unversioned property reader: how a cooked object's property values are laid out once the
//! types are known.
//!
//! Cooked UE5 objects serialize an "unversioned header" — a fragment list that says which schema
//! slots are present and which of their values are zero (absent from the stream) — followed by the
//! present values in schema order. A [`Schema`] supplies each struct's properties; [`read_struct`]
//! walks one against a [`Reader`].
//!
//! The wire rules here mirror the research parser (`usmap-rows-partial.py`) that was validated to
//! exact export boundaries: bool property values are one byte, enum values use their underlying
//! scalar, names are `FMappedName`, and a schema that knows more slots than the wire is accepted as
//! a prefix.

use std::collections::HashMap;

use anyhow::{Result, bail};

/// A forward cursor over a byte slice with the primitives cooked objects use.
pub struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }

    /// The current offset.
    pub fn offset(&self) -> usize {
        self.at
    }

    /// Bytes left after the cursor.
    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.at)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.at + n > self.data.len() {
            bail!("read past end: {} + {} > {}", self.at, n, self.data.len());
        }
        let s = &self.data[self.at..self.at + n];
        self.at += n;
        Ok(s)
    }

    /// Advance past `n` bytes without interpreting them.
    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n)?;
        Ok(())
    }

    /// A view of `len` bytes starting at `start`, without moving the cursor.
    pub fn slice(&self, start: usize, len: usize) -> &'a [u8] {
        &self.data[start..start + len]
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    /// An `FString`: a signed 32-bit length (negative means UTF-16, which this reader rejects) then
    /// the bytes including a trailing NUL.
    pub fn fstring(&mut self) -> Result<String> {
        let n = self.i32()?;
        if n == 0 {
            return Ok(String::new());
        }
        if n < 0 {
            bail!("UTF-16 FString length {n} not supported here");
        }
        if n > 1 << 20 {
            bail!("implausible FString length {n}");
        }
        let bytes = self.take(n as usize)?;
        let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        Ok(String::from_utf8_lossy(&bytes[..end]).into_owned())
    }

    /// A serialized `bool` where a native serializer used the archive's 4-byte form.
    pub fn bool32(&mut self) -> Result<bool> {
        match self.i32()? {
            0 => Ok(false),
            1 => Ok(true),
            v => bail!("bool32 {v}"),
        }
    }
}

/// The two name tables an `FMappedName` can index: the package-local batch and the global batch.
#[derive(Clone, Copy)]
pub struct Names<'a> {
    pub local: &'a [String],
    pub global: &'a [String],
}

impl<'a> Names<'a> {
    /// Resolve an `FMappedName` at the cursor: low 30 bits index, top two bits select the table
    /// (non-zero = global), the extra index becomes a `_N` suffix.
    pub fn read(&self, r: &mut Reader) -> Result<String> {
        let raw = r.u32()?;
        let extra = r.u32()?;
        let idx = (raw & 0x3FFF_FFFF) as usize;
        let table = if raw >> 30 != 0 {
            self.global
        } else {
            self.local
        };
        let base = table
            .get(idx)
            .cloned()
            .unwrap_or_else(|| format!("<name#{idx}>"));
        Ok(if extra > 0 {
            format!("{base}_{}", extra - 1)
        } else {
            base
        })
    }
}

/// A property's serialized type. Mirrors the `.usmap` type vocabulary the tables use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropType {
    Bool,
    Byte,
    Int8,
    Int16,
    UInt16,
    Int,
    UInt32,
    Float,
    Double,
    Int64,
    UInt64,
    Name,
    Str,
    Object,
    SoftObject,
    Class,
    Enum(Box<PropType>),
    Struct(String),
    Array(Box<PropType>),
    Set(Box<PropType>),
    Map(Box<PropType>, Box<PropType>),
    Optional(Box<PropType>),
}

/// One named property in a struct's declaration order.
#[derive(Debug, Clone)]
pub struct Property {
    pub name: String,
    pub ty: PropType,
}

/// A struct's own properties and its parent (whose properties follow).
#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    pub parent: Option<String>,
    pub properties: Vec<Property>,
}

/// The struct layouts a reader walks, keyed by name.
#[derive(Default)]
pub struct Schema {
    structs: HashMap<String, StructDef>,
}

impl Schema {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, def: StructDef) {
        self.structs.insert(def.name.clone(), def);
    }

    pub fn get(&self, name: &str) -> Option<&StructDef> {
        self.structs.get(name)
    }

    /// A struct's property chain: its own properties first, then each ancestor's, cycle-guarded.
    pub fn chain(&self, name: &str) -> Vec<&Property> {
        let mut out = Vec::new();
        let mut seen = Vec::new();
        let mut cur = Some(name.to_string());
        while let Some(name) = cur {
            if seen.contains(&name) {
                break;
            }
            match self.structs.get(&name) {
                Some(def) => {
                    out.extend(def.properties.iter());
                    seen.push(name);
                    cur = def.parent.clone();
                }
                None => break,
            }
        }
        out
    }
}

/// A parsed property value. Enough variants for the cooked objects this crate reads.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Declared in the schema but zeroed on the wire (the header's zero mask).
    Zero,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f32),
    Double(f64),
    Text(String),
    Name(String),
    /// A cooked object reference: a package index (negative = import).
    Object(i32),
    Struct(Vec<(String, Value)>),
    Array(Vec<Value>),
    Map(Vec<(Value, Value)>),
    Optional(Box<Option<Value>>),
}

/// A custom (native) struct serializer: reads the wire form a plain schema cannot describe.
pub type NativeFn = fn(&mut Reader, &Names) -> Result<Value>;

/// Custom (native) struct serializers, looked up before the schema. Keyed by struct name.
#[derive(Default)]
pub struct Natives {
    fns: HashMap<String, NativeFn>,
}

impl Natives {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: &str, f: NativeFn) {
        self.fns.insert(name.to_string(), f);
    }

    fn get(&self, name: &str) -> Option<NativeFn> {
        self.fns.get(name).copied()
    }
}

/// Everything [`read_value`] needs: the schema, the name tables, and any native serializers.
pub struct Ctx<'a> {
    pub schema: &'a Schema,
    pub names: Names<'a>,
    pub natives: &'a Natives,
}

/// An unversioned header: the fragment list and the per-stored-value zero mask.
pub struct UniHeader {
    /// `(skip, value_num)` per fragment.
    pub fragments: Vec<(usize, usize)>,
    /// One flag per stored slot that declares a value: `true` means the value is zero on the wire.
    pub is_zero: Vec<bool>,
    /// Header size in bytes.
    pub header_len: usize,
}

/// Read an `FUnversionedHeader`: fragments until the `IsLast` bit, then the zero mask.
pub fn read_unversioned_header(r: &mut Reader) -> Result<UniHeader> {
    let start = r.offset();
    let mut fragments = Vec::new();
    let mut zero_bits = 0usize;
    loop {
        let raw = r.u16()?;
        let skip = (raw & 0x7f) as usize;
        let has_zero = raw & 0x80 != 0;
        let is_last = raw & 0x100 != 0;
        let value_num = (raw >> 9) as usize;
        if has_zero {
            zero_bits += value_num;
        }
        fragments.push((skip, value_num));
        if is_last {
            break;
        }
        if fragments.len() > 4096 {
            bail!("unversioned header implausibly long");
        }
    }
    let mask_bytes = match zero_bits {
        0 => 0,
        1..=8 => 1,
        9..=16 => 2,
        n => 4 * n.div_ceil(32),
    };
    let mut is_zero = Vec::with_capacity(zero_bits);
    let mask = r.take(mask_bytes)?;
    for k in 0..zero_bits {
        is_zero.push(mask[k / 8] & (1 << (k % 8)) != 0);
    }
    Ok(UniHeader {
        fragments,
        is_zero,
        header_len: r.offset() - start,
    })
}

/// The number of schema slots a header describes (skips plus value counts).
pub fn header_slots(h: &UniHeader) -> usize {
    h.fragments.iter().map(|(s, v)| s + v).sum()
}

/// Read one property value of the given type.
pub fn read_value(r: &mut Reader, ty: &PropType, ctx: &Ctx) -> Result<Value> {
    Ok(match ty {
        PropType::Bool => Value::Bool(r.u8()? != 0),
        PropType::Byte | PropType::Int8 => Value::UInt(u64::from(r.u8()?)),
        PropType::Int16 | PropType::UInt16 => Value::UInt(u64::from(r.u16()?)),
        PropType::Int => Value::Int(i64::from(r.i32()?)),
        PropType::UInt32 => Value::UInt(u64::from(r.u32()?)),
        PropType::Float => Value::Float(r.f32()?),
        PropType::Double => Value::Double(r.f64()?),
        PropType::Int64 => Value::Int(r.i64()?),
        PropType::UInt64 => Value::UInt(r.u64()?),
        PropType::Name => Value::Name(ctx.names.read(r)?),
        PropType::Str => Value::Text(r.fstring()?),
        PropType::Object => Value::Object(r.i32()?),
        PropType::SoftObject | PropType::Class => {
            let a = r.u32()?;
            let b = r.u32()?;
            let sub = r.fstring()?;
            Value::Struct(vec![
                ("package".into(), Value::UInt(u64::from(a & 0x3FFF_FFFF))),
                ("asset".into(), Value::UInt(u64::from(b & 0x3FFF_FFFF))),
                ("sub".into(), Value::Text(sub)),
            ])
        }
        PropType::Enum(inner) => read_value(r, inner, ctx)?,
        PropType::Struct(name) => match ctx.natives.get(name) {
            Some(f) => f(r, &ctx.names)?,
            None => read_struct(r, name, ctx)?,
        },
        PropType::Array(inner) | PropType::Set(inner) => {
            let n = r.i32()?;
            if !(0..=2_000_000).contains(&n) {
                bail!("bad array/set length {n}");
            }
            let mut out = Vec::with_capacity(n as usize);
            for _ in 0..n {
                out.push(read_value(r, inner, ctx)?);
            }
            Value::Array(out)
        }
        PropType::Map(k, v) => {
            let n = r.i32()?;
            if !(0..=2_000_000).contains(&n) {
                bail!("bad map length {n}");
            }
            let mut out = Vec::with_capacity(n as usize);
            for _ in 0..n {
                out.push((read_value(r, k, ctx)?, read_value(r, v, ctx)?));
            }
            Value::Map(out)
        }
        PropType::Optional(inner) => {
            let present = r.u8()?;
            if present != 0 {
                Value::Optional(Box::new(Some(read_value(r, inner, ctx)?)))
            } else {
                Value::Optional(Box::new(None))
            }
        }
    })
}

/// Read a struct: its unversioned header, then the present values in schema order. A schema that
/// knows more slots than the wire supplies is accepted as a prefix (the mapping may be newer).
pub fn read_struct(r: &mut Reader, name: &str, ctx: &Ctx) -> Result<Value> {
    let props = ctx.schema.chain(name);
    let header = read_unversioned_header(r)?;
    let slots = header_slots(&header);
    if slots > props.len() {
        bail!("{name}: wire slots {slots} > mapped {}", props.len());
    }
    let mut out = Vec::with_capacity(slots);
    let mut idx = 0usize;
    let mut mi = 0usize;
    for (skip, value_num) in &header.fragments {
        idx += skip;
        for _ in 0..*value_num {
            let p = &props[idx];
            idx += 1;
            let zero = header.is_zero.get(mi).copied().unwrap_or(false);
            mi += 1;
            if zero {
                out.push((p.name.clone(), Value::Zero));
            } else {
                out.push((p.name.clone(), read_value(r, &p.ty, ctx)?));
            }
        }
    }
    Ok(Value::Struct(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Schema {
        let mut s = Schema::new();
        s.insert(StructDef {
            name: "Base".into(),
            parent: None,
            properties: vec![Property {
                name: "Id".into(),
                ty: PropType::Int,
            }],
        });
        s.insert(StructDef {
            name: "Thing".into(),
            parent: Some("Base".into()),
            properties: vec![
                Property {
                    name: "Count".into(),
                    ty: PropType::Int,
                },
                Property {
                    name: "Label".into(),
                    ty: PropType::Name,
                },
                Property {
                    name: "Tint".into(),
                    ty: PropType::Struct("Color".into()),
                },
            ],
        });
        s
    }

    fn color_native(r: &mut Reader, _names: &Names) -> Result<Value> {
        Ok(Value::Struct(vec![
            ("r".into(), Value::Float(r.f32()?)),
            ("g".into(), Value::Float(r.f32()?)),
            ("b".into(), Value::Float(r.f32()?)),
            ("a".into(), Value::Float(r.f32()?)),
        ]))
    }

    #[test]
    fn chain_is_own_then_parent() {
        let s = schema();
        let names: Vec<&str> = s.chain("Thing").iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Count", "Label", "Tint", "Id"]);
    }

    #[test]
    fn reads_a_struct_with_a_native_member_and_a_zero_skip() {
        // One fragment: skip 0, three stored values, has-zero, is-last.
        let raw: u16 = 0x80 | (3 << 9) | 0x100;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&raw.to_le_bytes());
        bytes.push(0b001); // zero mask: Count (bit 0) is zero; Label and Tint are present
        bytes.extend_from_slice(&5u32.to_le_bytes()); // Label: local name index 5
        bytes.extend_from_slice(&0u32.to_le_bytes()); // Label: extra
        for v in [0.25f32, 0.5, 0.75, 1.0] {
            bytes.extend_from_slice(&v.to_le_bytes()); // Tint: native Color, 4 floats
        }

        let s = schema();
        let mut natives = Natives::new();
        natives.insert("Color", color_native);
        let local = vec!["<0>".to_string(), "unused".to_string()]; // index 5 is out of range
        let ctx = Ctx {
            schema: &s,
            names: Names {
                local: &local,
                global: &[],
            },
            natives: &natives,
        };
        let mut r = Reader::new(&bytes);
        let v = read_struct(&mut r, "Thing", &ctx).unwrap();
        let Value::Struct(fields) = v else {
            panic!("expected a struct")
        };
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0], ("Count".to_string(), Value::Zero));
        assert_eq!(fields[1].0, "Label");
        assert_eq!(fields[1].1, Value::Name("<name#5>".into()));
        assert_eq!(fields[2].0, "Tint");
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn rejects_a_schema_smaller_than_the_wire() {
        let mut s = Schema::new();
        s.insert(StructDef {
            name: "Tiny".into(),
            parent: None,
            properties: vec![],
        });
        // one fragment: skip 0, 1 value, no zero, last
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&((1u16 << 9) | 0x100).to_le_bytes());
        bytes.extend_from_slice(&[0u8; 4]);
        let natives = Natives::new();
        let ctx = Ctx {
            schema: &s,
            names: Names {
                local: &[],
                global: &[],
            },
            natives: &natives,
        };
        let mut r = Reader::new(&bytes);
        assert!(read_struct(&mut r, "Tiny", &ctx).is_err());
    }
}
