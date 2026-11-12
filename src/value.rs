//! Attribute values in `.lsf` files.
//!
//! The document keeps every value as the bytes it was read with. This module turns those bytes
//! into a typed [`Value`] for display and editing, and back. Encoding a decoded value always
//! gives the original bytes; a value that can't meet that is refused by [`decode`] and stays
//! raw.

use anyhow::{bail, ensure, Result};
use std::fmt;

/// The type tag stored with each attribute (the low six bits of its type-and-length field).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum DataType {
    None = 0,
    Byte = 1,
    Short = 2,
    UShort = 3,
    Int = 4,
    UInt = 5,
    Float = 6,
    Double = 7,
    IVec2 = 8,
    IVec3 = 9,
    IVec4 = 10,
    Vec2 = 11,
    Vec3 = 12,
    Vec4 = 13,
    Mat2 = 14,
    Mat3 = 15,
    Mat3x4 = 16,
    Mat4x3 = 17,
    Mat4 = 18,
    Bool = 19,
    String = 20,
    Path = 21,
    FixedString = 22,
    LSString = 23,
    ULongLong = 24,
    ScratchBuffer = 25,
    Long = 26,
    Int8 = 27,
    TranslatedString = 28,
    WString = 29,
    LSWString = 30,
    Uuid = 31,
    Int64 = 32,
    TranslatedFSString = 33,
}

impl DataType {
    pub fn from_id(id: u8) -> Option<DataType> {
        use DataType::*;
        Some(match id {
            0 => None,
            1 => Byte,
            2 => Short,
            3 => UShort,
            4 => Int,
            5 => UInt,
            6 => Float,
            7 => Double,
            8 => IVec2,
            9 => IVec3,
            10 => IVec4,
            11 => Vec2,
            12 => Vec3,
            13 => Vec4,
            14 => Mat2,
            15 => Mat3,
            16 => Mat3x4,
            17 => Mat4x3,
            18 => Mat4,
            19 => Bool,
            20 => String,
            21 => Path,
            22 => FixedString,
            23 => LSString,
            24 => ULongLong,
            25 => ScratchBuffer,
            26 => Long,
            27 => Int8,
            28 => TranslatedString,
            29 => WString,
            30 => LSWString,
            31 => Uuid,
            32 => Int64,
            33 => TranslatedFSString,
            _ => return Option::None,
        })
    }

    pub fn id(self) -> u8 {
        self as u8
    }

    pub fn name(self) -> &'static str {
        use DataType::*;
        match self {
            None => "None",
            Byte => "Byte",
            Short => "Short",
            UShort => "UShort",
            Int => "Int",
            UInt => "UInt",
            Float => "Float",
            Double => "Double",
            IVec2 => "IVec2",
            IVec3 => "IVec3",
            IVec4 => "IVec4",
            Vec2 => "Vec2",
            Vec3 => "Vec3",
            Vec4 => "Vec4",
            Mat2 => "Mat2",
            Mat3 => "Mat3",
            Mat3x4 => "Mat3x4",
            Mat4x3 => "Mat4x3",
            Mat4 => "Mat4",
            Bool => "Bool",
            String => "String",
            Path => "Path",
            FixedString => "FixedString",
            LSString => "LSString",
            ULongLong => "ULongLong",
            ScratchBuffer => "ScratchBuffer",
            Long => "Long",
            Int8 => "Int8",
            TranslatedString => "TranslatedString",
            WString => "WString",
            LSWString => "LSWString",
            Uuid => "UUID",
            Int64 => "Int64",
            TranslatedFSString => "TranslatedFSString",
        }
    }

    fn is_text(self) -> bool {
        use DataType::*;
        matches!(
            self,
            String | Path | FixedString | LSString | WString | LSWString
        )
    }

    /// Number of 32-bit components for the vector and matrix types.
    fn components(self) -> Option<usize> {
        use DataType::*;
        Some(match self {
            IVec2 | Vec2 => 2,
            IVec3 | Vec3 => 3,
            IVec4 | Vec4 | Mat2 => 4,
            Mat3 => 9,
            Mat3x4 | Mat4x3 => 12,
            Mat4 => 16,
            _ => return Option::None,
        })
    }
}

/// The name for a type tag, including tags this editor doesn't know.
pub fn type_name(id: u8) -> String {
    match DataType::from_id(id) {
        Some(t) => t.name().to_owned(),
        None => format!("Type{id}"),
    }
}

/// A 16-byte identifier, kept in the byte order the file uses.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Uuid(pub [u8; 16]);

impl Uuid {
    /// The usual text form. The file stores the first three groups little-endian and the rest
    /// with each pair of bytes swapped.
    fn text_order(&self) -> [u8; 16] {
        let b = &self.0;
        [
            b[3], b[2], b[1], b[0], b[5], b[4], b[7], b[6], b[9], b[8], b[11], b[10], b[13], b[12],
            b[15], b[14],
        ]
    }

    /// Parses `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`.
    pub fn parse(text: &str) -> Option<Uuid> {
        let bytes = text.as_bytes();
        if bytes.len() != 36 {
            return None;
        }
        let mut t = [0u8; 16];
        let mut n = 0;
        let mut i = 0;
        while i < 36 {
            if matches!(i, 8 | 13 | 18 | 23) {
                if bytes[i] != b'-' {
                    return None;
                }
                i += 1;
                continue;
            }
            let hi = (bytes[i] as char).to_digit(16)?;
            let lo = (bytes[i + 1] as char).to_digit(16)?;
            t[n] = (hi * 16 + lo) as u8;
            n += 1;
            i += 2;
        }
        // The mapping between file order and text order is its own inverse.
        Some(Uuid(Uuid(t).text_order()))
    }

    pub fn is_nil(&self) -> bool {
        self.0 == [0u8; 16]
    }
}

impl fmt::Display for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let t = self.text_order();
        for (i, byte) in t.iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) {
                f.write_str("-")?;
            }
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// A decoded attribute value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    None,
    Byte(u8),
    Short(i16),
    UShort(u16),
    Int(i32),
    UInt(u32),
    Float(f32),
    Double(f64),
    /// IVec2, IVec3, IVec4.
    Ints(Vec<i32>),
    /// Vec2 to Vec4 and the matrix types, row by row.
    Floats(Vec<f32>),
    Bool(bool),
    /// String, Path, FixedString, LSString, WString and LSWString.
    Text(String),
    ULongLong(u64),
    /// Long and Int64.
    Int64(i64),
    Int8(i8),
    /// ScratchBuffer, and TranslatedFSString, which is kept as it is.
    Bytes(Vec<u8>),
    /// A reference to a localized string: a version and the string's handle.
    Translated {
        version: u16,
        handle: String,
    },
    Uuid(Uuid),
}

impl Value {
    /// The value as an integer, for the integer types and Bool.
    pub fn as_i64(&self) -> Option<i64> {
        Some(match self {
            Value::Byte(v) => *v as i64,
            Value::Short(v) => *v as i64,
            Value::UShort(v) => *v as i64,
            Value::Int(v) => *v as i64,
            Value::UInt(v) => *v as i64,
            Value::Int8(v) => *v as i64,
            Value::Int64(v) => *v,
            Value::ULongLong(v) => i64::try_from(*v).ok()?,
            Value::Bool(v) => *v as i64,
            _ => return None,
        })
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(v) => Some(*v as f64),
            Value::Double(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            Value::Translated { handle, .. } => Some(handle),
            _ => None,
        }
    }

    pub fn as_uuid(&self) -> Option<Uuid> {
        match self {
            Value::Uuid(u) => Some(*u),
            _ => None,
        }
    }
}

fn exact<const N: usize>(bytes: &[u8]) -> Result<[u8; N]> {
    match <[u8; N]>::try_from(bytes) {
        Ok(array) => Ok(array),
        Err(_) => bail!("expected {N} bytes, found {}", bytes.len()),
    }
}

/// Text stored as UTF-8 with one terminating NUL and none inside.
fn decode_text(bytes: &[u8]) -> Result<String> {
    let Some((&0, body)) = bytes.split_last() else {
        bail!("text without a terminating NUL");
    };
    ensure!(!body.contains(&0), "text with a NUL inside");
    Ok(std::str::from_utf8(body)?.to_owned())
}

fn encode_text(text: &str) -> Result<Vec<u8>> {
    ensure!(!text.contains('\0'), "text can't contain NUL");
    let mut out = Vec::with_capacity(text.len() + 1);
    out.extend_from_slice(text.as_bytes());
    out.push(0);
    Ok(out)
}

/// Decodes the bytes of an attribute of type `type_id`.
pub fn decode(type_id: u8, bytes: &[u8]) -> Result<Value> {
    let Some(data_type) = DataType::from_id(type_id) else {
        bail!("unknown value type {type_id}");
    };
    use DataType as T;
    Ok(match data_type {
        T::None => {
            ensure!(bytes.is_empty(), "a None value with data");
            Value::None
        }
        T::Byte => Value::Byte(exact::<1>(bytes)?[0]),
        T::Int8 => Value::Int8(exact::<1>(bytes)?[0] as i8),
        // The game writes a few Bool attributes with a byte other than 0 or 1 (HotbarLocked
        // in meta.lsf has been seen holding 30). Those come back as Byte so they survive.
        T::Bool => match exact::<1>(bytes)?[0] {
            0 => Value::Bool(false),
            1 => Value::Bool(true),
            other => Value::Byte(other),
        },
        T::Short => Value::Short(i16::from_le_bytes(exact(bytes)?)),
        T::UShort => Value::UShort(u16::from_le_bytes(exact(bytes)?)),
        T::Int => Value::Int(i32::from_le_bytes(exact(bytes)?)),
        T::UInt => Value::UInt(u32::from_le_bytes(exact(bytes)?)),
        T::Float => Value::Float(f32::from_le_bytes(exact(bytes)?)),
        T::Double => Value::Double(f64::from_le_bytes(exact(bytes)?)),
        T::ULongLong => Value::ULongLong(u64::from_le_bytes(exact(bytes)?)),
        T::Long | T::Int64 => Value::Int64(i64::from_le_bytes(exact(bytes)?)),
        T::Uuid => Value::Uuid(Uuid(exact(bytes)?)),
        T::IVec2 | T::IVec3 | T::IVec4 => {
            let n = data_type.components().unwrap_or(0);
            ensure!(
                bytes.len() == n * 4,
                "expected {} bytes, found {}",
                n * 4,
                bytes.len()
            );
            Value::Ints(
                bytes
                    .chunks_exact(4)
                    .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect(),
            )
        }
        T::Vec2 | T::Vec3 | T::Vec4 | T::Mat2 | T::Mat3 | T::Mat3x4 | T::Mat4x3 | T::Mat4 => {
            let n = data_type.components().unwrap_or(0);
            ensure!(
                bytes.len() == n * 4,
                "expected {} bytes, found {}",
                n * 4,
                bytes.len()
            );
            Value::Floats(
                bytes
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect(),
            )
        }
        T::String | T::Path | T::FixedString | T::LSString | T::WString | T::LSWString => {
            Value::Text(decode_text(bytes)?)
        }
        T::TranslatedString => {
            ensure!(
                bytes.len() >= 6,
                "a translated string of {} bytes",
                bytes.len()
            );
            let version = u16::from_le_bytes([bytes[0], bytes[1]]);
            let len = i32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]);
            ensure!(
                len >= 0 && len as usize == bytes.len() - 6,
                "a translated string whose length field doesn't match"
            );
            Value::Translated {
                version,
                handle: decode_text(&bytes[6..])?,
            }
        }
        T::ScratchBuffer | T::TranslatedFSString => Value::Bytes(bytes.to_vec()),
    })
}

/// Encodes `value` as the bytes of an attribute of type `type_id`. The value must be the kind
/// that [`decode`] returns for that type.
pub fn encode(type_id: u8, value: &Value) -> Result<Vec<u8>> {
    let Some(data_type) = DataType::from_id(type_id) else {
        bail!("unknown value type {type_id}");
    };
    use DataType as T;
    let mismatch = || anyhow::anyhow!("a {} attribute can't hold {value:?}", data_type.name());
    Ok(match (data_type, value) {
        (T::None, Value::None) => Vec::new(),
        (T::Byte, Value::Byte(v)) => vec![*v],
        (T::Int8, Value::Int8(v)) => vec![*v as u8],
        (T::Bool, Value::Bool(v)) => vec![*v as u8],
        (T::Bool, Value::Byte(v)) if *v > 1 => vec![*v],
        (T::Short, Value::Short(v)) => v.to_le_bytes().to_vec(),
        (T::UShort, Value::UShort(v)) => v.to_le_bytes().to_vec(),
        (T::Int, Value::Int(v)) => v.to_le_bytes().to_vec(),
        (T::UInt, Value::UInt(v)) => v.to_le_bytes().to_vec(),
        (T::Float, Value::Float(v)) => v.to_le_bytes().to_vec(),
        (T::Double, Value::Double(v)) => v.to_le_bytes().to_vec(),
        (T::ULongLong, Value::ULongLong(v)) => v.to_le_bytes().to_vec(),
        (T::Long | T::Int64, Value::Int64(v)) => v.to_le_bytes().to_vec(),
        (T::Uuid, Value::Uuid(v)) => v.0.to_vec(),
        (T::IVec2 | T::IVec3 | T::IVec4, Value::Ints(v)) => {
            ensure!(Some(v.len()) == data_type.components(), mismatch());
            v.iter().flat_map(|c| c.to_le_bytes()).collect()
        }
        (
            T::Vec2 | T::Vec3 | T::Vec4 | T::Mat2 | T::Mat3 | T::Mat3x4 | T::Mat4x3 | T::Mat4,
            Value::Floats(v),
        ) => {
            ensure!(Some(v.len()) == data_type.components(), mismatch());
            v.iter().flat_map(|c| c.to_le_bytes()).collect()
        }
        (t, Value::Text(s)) if t.is_text() => encode_text(s)?,
        (T::TranslatedString, Value::Translated { version, handle }) => {
            let text = encode_text(handle)?;
            let mut out = Vec::with_capacity(6 + text.len());
            out.extend_from_slice(&version.to_le_bytes());
            out.extend_from_slice(&(text.len() as i32).to_le_bytes());
            out.extend_from_slice(&text);
            out
        }
        (T::ScratchBuffer | T::TranslatedFSString, Value::Bytes(v)) => v.clone(),
        _ => return Err(mismatch()),
    })
}

fn write_list<T: fmt::Display>(f: &mut fmt::Formatter<'_>, items: &[T]) -> fmt::Result {
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            f.write_str(" ")?;
        }
        write!(f, "{item}")?;
    }
    Ok(())
}

/// Writes bytes as hex, shortened in the middle when long.
pub fn write_hex(f: &mut impl fmt::Write, bytes: &[u8], limit: usize) -> fmt::Result {
    if bytes.len() <= limit {
        for byte in bytes {
            write!(f, "{byte:02x}")?;
        }
    } else {
        for byte in &bytes[..limit] {
            write!(f, "{byte:02x}")?;
        }
        write!(f, "… ({} bytes)", bytes.len())?;
    }
    Ok(())
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::None => f.write_str("none"),
            Value::Byte(v) => write!(f, "{v}"),
            Value::Short(v) => write!(f, "{v}"),
            Value::UShort(v) => write!(f, "{v}"),
            Value::Int(v) => write!(f, "{v}"),
            Value::UInt(v) => write!(f, "{v}"),
            Value::Int8(v) => write!(f, "{v}"),
            Value::Int64(v) => write!(f, "{v}"),
            Value::ULongLong(v) => write!(f, "{v}"),
            Value::Float(v) => write!(f, "{v}"),
            Value::Double(v) => write!(f, "{v}"),
            Value::Bool(v) => write!(f, "{v}"),
            Value::Ints(v) => write_list(f, v),
            Value::Floats(v) => write_list(f, v),
            Value::Text(s) => write!(f, "{s:?}"),
            Value::Translated { version, handle } => write!(f, "{handle:?} v{version}"),
            Value::Uuid(u) => write!(f, "{u}"),
            Value::Bytes(b) => write_hex(f, b, 32),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_text_matches_the_game() {
        // Shadowheart's identifier as stored in a save's Globals.lsf.
        let stored = Uuid::parse("3ed74f06-3c60-42dc-83f6-f034cb47c679").unwrap();
        assert_eq!(
            stored.0,
            [
                0x06, 0x4f, 0xd7, 0x3e, 0x60, 0x3c, 0xdc, 0x42, 0xf6, 0x83, 0x34, 0xf0, 0x47, 0xcb,
                0x79, 0xc6
            ]
        );
        assert_eq!(stored.to_string(), "3ed74f06-3c60-42dc-83f6-f034cb47c679");
        assert!(Uuid::parse("3ed74f06-3c60-42dc-83f6-f034cb47c67").is_none());
        assert!(Uuid::parse("3ed74f06+3c60-42dc-83f6-f034cb47c679").is_none());
        assert!(Uuid([0; 16]).is_nil());
    }

    #[test]
    fn every_type_round_trips() {
        let cases: Vec<(DataType, Vec<u8>)> = vec![
            (DataType::None, vec![]),
            (DataType::Byte, vec![200]),
            (DataType::Int8, vec![0xfe]),
            (DataType::Bool, vec![1]),
            (DataType::Short, (-2i16).to_le_bytes().to_vec()),
            (DataType::UShort, 65000u16.to_le_bytes().to_vec()),
            (DataType::Int, (-7i32).to_le_bytes().to_vec()),
            (DataType::UInt, 4_000_000_000u32.to_le_bytes().to_vec()),
            (DataType::Float, 1.5f32.to_le_bytes().to_vec()),
            (DataType::Double, (-0.25f64).to_le_bytes().to_vec()),
            (DataType::ULongLong, u64::MAX.to_le_bytes().to_vec()),
            (DataType::Int64, i64::MIN.to_le_bytes().to_vec()),
            (DataType::Long, 5i64.to_le_bytes().to_vec()),
            (DataType::Uuid, (0u8..16).collect()),
            (
                DataType::IVec2,
                [1i32, -1].iter().flat_map(|v| v.to_le_bytes()).collect(),
            ),
            (
                DataType::Vec3,
                [1f32, 2.5, -3.0]
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect(),
            ),
            (
                DataType::Mat4,
                (0..16).flat_map(|v| (v as f32).to_le_bytes()).collect(),
            ),
            (DataType::FixedString, b"LOOT_Gold_A\0".to_vec()),
            (DataType::LSString, "Faer\u{fb}n\0".as_bytes().to_vec()),
            (DataType::ScratchBuffer, vec![1, 2, 3]),
            (
                DataType::TranslatedString,
                [&[0u8, 0][..], &4i32.to_le_bytes(), b"h12\0"].concat(),
            ),
        ];
        for (data_type, bytes) in cases {
            let value = decode(data_type.id(), &bytes)
                .unwrap_or_else(|e| panic!("{}: {e}", data_type.name()));
            assert_eq!(
                encode(data_type.id(), &value).unwrap(),
                bytes,
                "{}",
                data_type.name()
            );
        }
    }

    #[test]
    fn refuses_values_it_could_not_write_back() {
        assert!(decode(DataType::Int.id(), &[1, 2, 3]).is_err());
        assert_eq!(decode(DataType::Bool.id(), &[30]).unwrap(), Value::Byte(30));
        assert_eq!(encode(DataType::Bool.id(), &Value::Byte(30)).unwrap(), [30]);
        assert!(encode(DataType::Bool.id(), &Value::Byte(1)).is_err());
        assert!(decode(DataType::LSString.id(), b"no terminator").is_err());
        assert!(decode(DataType::LSString.id(), b"two\0parts\0").is_err());
        assert!(decode(
            DataType::TranslatedString.id(),
            &[0, 0, 9, 0, 0, 0, b'h', 0]
        )
        .is_err());
        assert!(decode(60, &[]).is_err());
        assert!(encode(DataType::Int.id(), &Value::UInt(1)).is_err());
        assert!(encode(DataType::Vec3.id(), &Value::Floats(vec![1.0])).is_err());
        assert!(encode(DataType::LSString.id(), &Value::Text("a\0b".into())).is_err());
    }

    #[test]
    fn display_is_readable() {
        assert_eq!(Value::Int(-3).to_string(), "-3");
        assert_eq!(Value::Text("a \"b\"".into()).to_string(), r#""a \"b\"""#);
        assert_eq!(Value::Floats(vec![1.0, 2.5]).to_string(), "1 2.5");
        assert_eq!(
            Value::Bytes(vec![0xab; 40]).to_string().chars().count(),
            64 + 12
        );
        assert_eq!(type_name(31), "UUID");
        assert_eq!(type_name(50), "Type50");
    }
}
