use std::str;

use thiserror::Error;

pub const DEFAULT_MAX_NBT_DEPTH: usize = 512;
pub const DEFAULT_MAX_NBT_BYTES: usize = 64 * 1024 * 1024;
pub const DEFAULT_MAX_NBT_COLLECTION_LENGTH: usize = 1024 * 1024;
pub const DEFAULT_MAX_NBT_STRING_BYTES: usize = u16::MAX as usize;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NbtLimits {
    pub max_depth: usize,
    pub max_bytes: usize,
    pub max_collection_length: usize,
    pub max_string_bytes: usize,
}

impl Default for NbtLimits {
    fn default() -> Self {
        Self {
            max_depth: DEFAULT_MAX_NBT_DEPTH,
            max_bytes: DEFAULT_MAX_NBT_BYTES,
            max_collection_length: DEFAULT_MAX_NBT_COLLECTION_LENGTH,
            max_string_bytes: DEFAULT_MAX_NBT_STRING_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum NbtTagType {
    End = 0,
    Byte = 1,
    Short = 2,
    Int = 3,
    Long = 4,
    Float = 5,
    Double = 6,
    ByteArray = 7,
    String = 8,
    List = 9,
    Compound = 10,
    IntArray = 11,
    LongArray = 12,
}

impl NbtTagType {
    fn from_id(id: u8, offset: usize) -> Result<Self, NbtError> {
        match id {
            0 => Ok(Self::End),
            1 => Ok(Self::Byte),
            2 => Ok(Self::Short),
            3 => Ok(Self::Int),
            4 => Ok(Self::Long),
            5 => Ok(Self::Float),
            6 => Ok(Self::Double),
            7 => Ok(Self::ByteArray),
            8 => Ok(Self::String),
            9 => Ok(Self::List),
            10 => Ok(Self::Compound),
            11 => Ok(Self::IntArray),
            12 => Ok(Self::LongArray),
            _ => Err(NbtError::InvalidTagType { offset, id }),
        }
    }

    fn minimum_payload_bytes(self) -> usize {
        match self {
            Self::End => 0,
            Self::Byte => 1,
            Self::Short => 2,
            Self::Int | Self::Float => 4,
            Self::Long | Self::Double => 8,
            Self::ByteArray | Self::List | Self::IntArray | Self::LongArray => 4,
            Self::String => 2,
            Self::Compound => 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NamedTag {
    pub name: String,
    pub value: NbtValue,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NbtValue {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    List {
        element_type: NbtTagType,
        values: Vec<NbtValue>,
    },
    Compound(Vec<NamedTag>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl NbtValue {
    pub fn compound_entry(&self, name: &str) -> Option<&NbtValue> {
        let Self::Compound(entries) = self else {
            return None;
        };
        entries
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| &entry.value)
    }

    pub fn as_i32(&self) -> Option<i32> {
        match self {
            Self::Int(value) => Some(*value),
            _ => None,
        }
    }
}

pub fn parse_named_root(input: &[u8], limits: NbtLimits) -> Result<NamedTag, NbtError> {
    if input.len() > limits.max_bytes {
        return Err(NbtError::InputTooLarge {
            actual_bytes: input.len(),
            max_bytes: limits.max_bytes,
        });
    }

    let mut reader = Reader::new(input, limits);
    let type_offset = reader.position();
    let tag_type = NbtTagType::from_id(reader.read_u8()?, type_offset)?;
    if tag_type == NbtTagType::End {
        return Err(NbtError::RootEndTag);
    }
    let name = reader.read_string()?;
    let value = reader.read_payload(tag_type, 0)?;
    if reader.remaining() != 0 {
        return Err(NbtError::TrailingData {
            offset: reader.position(),
            trailing_bytes: reader.remaining(),
        });
    }
    Ok(NamedTag { name, value })
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum NbtError {
    #[error("NBT input has {actual_bytes} bytes, limit is {max_bytes}")]
    InputTooLarge {
        actual_bytes: usize,
        max_bytes: usize,
    },
    #[error("NBT input ended at byte {offset}: need {needed_bytes}, have {remaining_bytes}")]
    UnexpectedEof {
        offset: usize,
        needed_bytes: usize,
        remaining_bytes: usize,
    },
    #[error("unknown NBT tag id {id} at byte {offset}")]
    InvalidTagType { offset: usize, id: u8 },
    #[error("NBT root cannot be TAG_End")]
    RootEndTag,
    #[error("NBT nesting depth {depth} exceeds limit {max_depth}")]
    DepthLimitExceeded { depth: usize, max_depth: usize },
    #[error("{tag_type:?} has negative collection length {length} at byte {offset}")]
    NegativeLength {
        offset: usize,
        tag_type: NbtTagType,
        length: i32,
    },
    #[error("{tag_type:?} collection length {length} exceeds limit {max_length}")]
    CollectionTooLarge {
        tag_type: NbtTagType,
        length: usize,
        max_length: usize,
    },
    #[error("NBT string length {length} exceeds limit {max_length}")]
    StringTooLarge { length: usize, max_length: usize },
    #[error("NBT string at byte {offset} is not valid UTF-8")]
    InvalidUtf8 { offset: usize },
    #[error("non-empty NBT list cannot use TAG_End as its element type")]
    NonEmptyEndList,
    #[error("NBT collection byte size overflows platform limits")]
    CollectionByteSizeOverflow,
    #[error("NBT document has {trailing_bytes} trailing bytes at offset {offset}")]
    TrailingData {
        offset: usize,
        trailing_bytes: usize,
    },
}

struct Reader<'a> {
    input: &'a [u8],
    offset: usize,
    limits: NbtLimits,
}

impl<'a> Reader<'a> {
    fn new(input: &'a [u8], limits: NbtLimits) -> Self {
        Self {
            input,
            offset: 0,
            limits,
        }
    }

    fn position(&self) -> usize {
        self.offset
    }

    fn remaining(&self) -> usize {
        self.input.len() - self.offset
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], NbtError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(NbtError::CollectionByteSizeOverflow)?;
        if end > self.input.len() {
            return Err(NbtError::UnexpectedEof {
                offset: self.offset,
                needed_bytes: length,
                remaining_bytes: self.remaining(),
            });
        }
        let bytes = &self.input[self.offset..end];
        self.offset = end;
        Ok(bytes)
    }

    fn read_u8(&mut self) -> Result<u8, NbtError> {
        Ok(self.take(1)?[0])
    }

    fn read_i8(&mut self) -> Result<i8, NbtError> {
        Ok(i8::from_be_bytes([self.read_u8()?]))
    }

    fn read_u16(&mut self) -> Result<u16, NbtError> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn read_i16(&mut self) -> Result<i16, NbtError> {
        let bytes = self.take(2)?;
        Ok(i16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn read_i32(&mut self) -> Result<i32, NbtError> {
        let bytes = self.take(4)?;
        Ok(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_i64(&mut self) -> Result<i64, NbtError> {
        let bytes = self.take(8)?;
        Ok(i64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn read_string(&mut self) -> Result<String, NbtError> {
        let length = usize::from(self.read_u16()?);
        if length > self.limits.max_string_bytes {
            return Err(NbtError::StringTooLarge {
                length,
                max_length: self.limits.max_string_bytes,
            });
        }
        let offset = self.position();
        let bytes = self.take(length)?;
        let value = str::from_utf8(bytes).map_err(|_| NbtError::InvalidUtf8 { offset })?;
        Ok(value.to_owned())
    }

    fn read_payload(&mut self, tag_type: NbtTagType, depth: usize) -> Result<NbtValue, NbtError> {
        if depth > self.limits.max_depth {
            return Err(NbtError::DepthLimitExceeded {
                depth,
                max_depth: self.limits.max_depth,
            });
        }

        match tag_type {
            NbtTagType::End => Err(NbtError::RootEndTag),
            NbtTagType::Byte => Ok(NbtValue::Byte(self.read_i8()?)),
            NbtTagType::Short => Ok(NbtValue::Short(self.read_i16()?)),
            NbtTagType::Int => Ok(NbtValue::Int(self.read_i32()?)),
            NbtTagType::Long => Ok(NbtValue::Long(self.read_i64()?)),
            NbtTagType::Float => {
                let bits = u32::from_be_bytes(self.read_i32()?.to_be_bytes());
                Ok(NbtValue::Float(f32::from_bits(bits)))
            }
            NbtTagType::Double => {
                let bits = u64::from_be_bytes(self.read_i64()?.to_be_bytes());
                Ok(NbtValue::Double(f64::from_bits(bits)))
            }
            NbtTagType::ByteArray => self.read_byte_array(),
            NbtTagType::String => Ok(NbtValue::String(self.read_string()?)),
            NbtTagType::List => self.read_list(depth),
            NbtTagType::Compound => self.read_compound(depth),
            NbtTagType::IntArray => self.read_int_array(),
            NbtTagType::LongArray => self.read_long_array(),
        }
    }

    fn read_byte_array(&mut self) -> Result<NbtValue, NbtError> {
        let length = self.read_collection_length(NbtTagType::ByteArray)?;
        let bytes = self.take(length)?;
        Ok(NbtValue::ByteArray(
            bytes
                .iter()
                .map(|byte| i8::from_be_bytes([*byte]))
                .collect(),
        ))
    }

    fn read_list(&mut self, depth: usize) -> Result<NbtValue, NbtError> {
        let type_offset = self.position();
        let element_type = NbtTagType::from_id(self.read_u8()?, type_offset)?;
        let length = self.read_collection_length(NbtTagType::List)?;
        if element_type == NbtTagType::End && length != 0 {
            return Err(NbtError::NonEmptyEndList);
        }
        self.ensure_minimum_collection_bytes(element_type, length)?;

        let mut values = Vec::with_capacity(length);
        for _ in 0..length {
            values.push(self.read_payload(element_type, depth + 1)?);
        }
        Ok(NbtValue::List {
            element_type,
            values,
        })
    }

    fn read_compound(&mut self, depth: usize) -> Result<NbtValue, NbtError> {
        let mut entries = Vec::new();
        loop {
            let type_offset = self.position();
            let tag_type = NbtTagType::from_id(self.read_u8()?, type_offset)?;
            if tag_type == NbtTagType::End {
                return Ok(NbtValue::Compound(entries));
            }
            if entries.len() >= self.limits.max_collection_length {
                return Err(NbtError::CollectionTooLarge {
                    tag_type: NbtTagType::Compound,
                    length: entries.len() + 1,
                    max_length: self.limits.max_collection_length,
                });
            }
            let name = self.read_string()?;
            let value = self.read_payload(tag_type, depth + 1)?;
            entries.push(NamedTag { name, value });
        }
    }

    fn read_int_array(&mut self) -> Result<NbtValue, NbtError> {
        let length = self.read_collection_length(NbtTagType::IntArray)?;
        self.ensure_fixed_collection_bytes(length, 4)?;
        let mut values = Vec::with_capacity(length);
        for _ in 0..length {
            values.push(self.read_i32()?);
        }
        Ok(NbtValue::IntArray(values))
    }

    fn read_long_array(&mut self) -> Result<NbtValue, NbtError> {
        let length = self.read_collection_length(NbtTagType::LongArray)?;
        self.ensure_fixed_collection_bytes(length, 8)?;
        let mut values = Vec::with_capacity(length);
        for _ in 0..length {
            values.push(self.read_i64()?);
        }
        Ok(NbtValue::LongArray(values))
    }

    fn read_collection_length(&mut self, tag_type: NbtTagType) -> Result<usize, NbtError> {
        let offset = self.position();
        let signed_length = self.read_i32()?;
        if signed_length < 0 {
            return Err(NbtError::NegativeLength {
                offset,
                tag_type,
                length: signed_length,
            });
        }
        let length = signed_length as usize;
        if length > self.limits.max_collection_length {
            return Err(NbtError::CollectionTooLarge {
                tag_type,
                length,
                max_length: self.limits.max_collection_length,
            });
        }
        Ok(length)
    }

    fn ensure_minimum_collection_bytes(
        &self,
        element_type: NbtTagType,
        length: usize,
    ) -> Result<(), NbtError> {
        self.ensure_fixed_collection_bytes(length, element_type.minimum_payload_bytes())
    }

    fn ensure_fixed_collection_bytes(
        &self,
        length: usize,
        element_bytes: usize,
    ) -> Result<(), NbtError> {
        let required = length
            .checked_mul(element_bytes)
            .ok_or(NbtError::CollectionByteSizeOverflow)?;
        if required > self.remaining() {
            return Err(NbtError::UnexpectedEof {
                offset: self.position(),
                needed_bytes: required,
                remaining_bytes: self.remaining(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_named_root, NbtError, NbtLimits, NbtTagType, NbtValue};

    #[test]
    fn parses_named_compound_and_all_core_collection_shapes() {
        let bytes = [
            10, 0, 0, // root compound
            3, 0, 11, b'D', b'a', b't', b'a', b'V', b'e', b'r', b's', b'i', b'o', b'n', 0, 0, 0x13,
            0x27, // int 4903
            8, 0, 4, b'n', b'a', b'm', b'e', 0, 5, b'w', b'o', b'r', b'l', b'd', 9, 0, 4, b'l',
            b'i', b's', b't', 1, 0, 0, 0, 3, 1, 2, 3, 11, 0, 4, b'i', b'n', b't', b's', 0, 0, 0, 2,
            0, 0, 0, 7, 0, 0, 0, 9, 0,
        ];

        let root = parse_named_root(&bytes, NbtLimits::default()).expect("parse synthetic NBT");
        assert_eq!(root.name, "");
        assert_eq!(
            root.value
                .compound_entry("DataVersion")
                .and_then(NbtValue::as_i32),
            Some(4903)
        );
        assert_eq!(
            root.value.compound_entry("list"),
            Some(&NbtValue::List {
                element_type: NbtTagType::Byte,
                values: vec![NbtValue::Byte(1), NbtValue::Byte(2), NbtValue::Byte(3)],
            })
        );
    }

    #[test]
    fn rejects_unknown_tag_and_trailing_data() {
        assert_eq!(
            parse_named_root(&[99], NbtLimits::default()),
            Err(NbtError::InvalidTagType { offset: 0, id: 99 })
        );
        assert_eq!(
            parse_named_root(&[1, 0, 0, 7, 9], NbtLimits::default()),
            Err(NbtError::TrailingData {
                offset: 4,
                trailing_bytes: 1,
            })
        );
    }

    #[test]
    fn rejects_negative_and_oversized_collections_before_allocation() {
        let negative_list = [9, 0, 0, 1, 0xff, 0xff, 0xff, 0xff];
        assert_eq!(
            parse_named_root(&negative_list, NbtLimits::default()),
            Err(NbtError::NegativeLength {
                offset: 4,
                tag_type: NbtTagType::List,
                length: -1,
            })
        );

        let oversized_array = [7, 0, 0, 0, 0, 0, 3];
        let limits = NbtLimits {
            max_collection_length: 2,
            ..NbtLimits::default()
        };
        assert_eq!(
            parse_named_root(&oversized_array, limits),
            Err(NbtError::CollectionTooLarge {
                tag_type: NbtTagType::ByteArray,
                length: 3,
                max_length: 2,
            })
        );
    }

    #[test]
    fn rejects_non_empty_end_list_and_invalid_utf8() {
        assert_eq!(
            parse_named_root(&[9, 0, 0, 0, 0, 0, 0, 1], NbtLimits::default()),
            Err(NbtError::NonEmptyEndList)
        );
        assert_eq!(
            parse_named_root(&[8, 0, 0, 0, 1, 0xff], NbtLimits::default()),
            Err(NbtError::InvalidUtf8 { offset: 5 })
        );
    }

    #[test]
    fn enforces_depth_and_total_byte_limits() {
        let nested_compounds = [10, 0, 0, 10, 0, 1, b'a', 10, 0, 1, b'b', 0, 0, 0];
        let depth_limits = NbtLimits {
            max_depth: 1,
            ..NbtLimits::default()
        };
        assert_eq!(
            parse_named_root(&nested_compounds, depth_limits),
            Err(NbtError::DepthLimitExceeded {
                depth: 2,
                max_depth: 1,
            })
        );

        let byte_limits = NbtLimits {
            max_bytes: 3,
            ..NbtLimits::default()
        };
        assert_eq!(
            parse_named_root(&[1, 0, 0, 7], byte_limits),
            Err(NbtError::InputTooLarge {
                actual_bytes: 4,
                max_bytes: 3,
            })
        );
    }
}
