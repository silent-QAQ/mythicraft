use std::collections::BTreeSet;

use mythicraft_nbt::{NbtTagType, NbtValue};
use thiserror::Error;

pub const BLOCKS_PER_SECTION: usize = 16 * 16 * 16;
pub const LIGHT_BYTES_PER_SECTION: usize = BLOCKS_PER_SECTION / 2;
const MINIMUM_BLOCK_BITS_PER_ENTRY: u8 = 4;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkSection {
    pub y: i8,
    pub block_states: Option<BlockStateSection>,
    pub block_light: Option<Vec<u8>>,
    pub sky_light: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockStateSection {
    pub raw_palette_values: Vec<u32>,
    pub bits_per_entry: u8,
    pub packed_word_count: usize,
    pub palette_indices: Vec<u32>,
}

impl BlockStateSection {
    pub fn is_homogeneous(&self) -> bool {
        self.raw_palette_values.len() == 1
    }

    pub fn raw_palette_value_at(&self, block_index: usize) -> Option<u32> {
        let palette_index = usize::try_from(*self.palette_indices.get(block_index)?).ok()?;
        self.raw_palette_values.get(palette_index).copied()
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum SectionError {
    #[error("chunk sections tag must be a list of compounds")]
    SectionsNotCompoundList,
    #[error("chunk section {section_index} is missing byte Y")]
    MissingSectionY { section_index: usize },
    #[error("chunk contains duplicate section Y={y}")]
    DuplicateSectionY { y: i8 },
    #[error("section Y={y} block_states must be a compound")]
    BlockStatesNotCompound { y: i8 },
    #[error("section Y={y} block_states is missing palette")]
    MissingPalette { y: i8 },
    #[error("section Y={y} palette uses unsupported tag type {tag_type:?}")]
    UnsupportedPaletteType { y: i8, tag_type: NbtTagType },
    #[error("section Y={y} palette entry {entry_index} is not an integer")]
    NonIntegerPaletteEntry { y: i8, entry_index: usize },
    #[error("section Y={y} palette entry {entry_index} has negative value {value}")]
    NegativePaletteValue {
        y: i8,
        entry_index: usize,
        value: i64,
    },
    #[error("section Y={y} palette entry {entry_index} value {value} exceeds u32")]
    PaletteValueTooLarge {
        y: i8,
        entry_index: usize,
        value: i64,
    },
    #[error("section Y={y} palette must not be empty")]
    EmptyPalette { y: i8 },
    #[error("section Y={y} with {palette_length} palette entries is missing packed data")]
    MissingPackedData { y: i8, palette_length: usize },
    #[error("section Y={y} packed data must be a long array")]
    PackedDataNotLongArray { y: i8 },
    #[error(
        "section Y={y} packed data has {actual_words} words, expected {expected_words} for {bits_per_entry} bits per entry"
    )]
    PackedWordCountMismatch {
        y: i8,
        bits_per_entry: u8,
        actual_words: usize,
        expected_words: usize,
    },
    #[error(
        "section Y={y} block {block_index} references palette index {palette_index}, palette length is {palette_length}"
    )]
    PaletteIndexOutOfBounds {
        y: i8,
        block_index: usize,
        palette_index: u32,
        palette_length: usize,
    },
    #[error("section Y={y} {tag_name} must be a byte array")]
    LightNotByteArray { y: i8, tag_name: &'static str },
    #[error("section Y={y} {tag_name} has {actual_bytes} bytes, expected {expected_bytes}")]
    LightLengthMismatch {
        y: i8,
        tag_name: &'static str,
        actual_bytes: usize,
        expected_bytes: usize,
    },
}

pub fn parse_chunk_sections(root: &NbtValue) -> Result<Vec<ChunkSection>, SectionError> {
    let Some(sections) = root.compound_entry("sections") else {
        return Ok(Vec::new());
    };
    let NbtValue::List {
        element_type: NbtTagType::Compound,
        values,
    } = sections
    else {
        return Err(SectionError::SectionsNotCompoundList);
    };

    let mut parsed = Vec::with_capacity(values.len());
    let mut seen_y = BTreeSet::new();
    for (section_index, section) in values.iter().enumerate() {
        let NbtValue::Compound(_) = section else {
            return Err(SectionError::SectionsNotCompoundList);
        };
        let y = match section.compound_entry("Y") {
            Some(NbtValue::Byte(y)) => *y,
            _ => return Err(SectionError::MissingSectionY { section_index }),
        };
        if !seen_y.insert(y) {
            return Err(SectionError::DuplicateSectionY { y });
        }
        let block_states = match section.compound_entry("block_states") {
            Some(block_states) => Some(parse_block_states(y, block_states)?),
            None => None,
        };
        let block_light = parse_light_array(section, y, "BlockLight")?;
        let sky_light = parse_light_array(section, y, "SkyLight")?;
        parsed.push(ChunkSection {
            y,
            block_states,
            block_light,
            sky_light,
        });
    }
    parsed.sort_unstable_by_key(|section| section.y);
    Ok(parsed)
}

fn parse_light_array(
    section: &NbtValue,
    y: i8,
    tag_name: &'static str,
) -> Result<Option<Vec<u8>>, SectionError> {
    let Some(value) = section.compound_entry(tag_name) else {
        return Ok(None);
    };
    let NbtValue::ByteArray(bytes) = value else {
        return Err(SectionError::LightNotByteArray { y, tag_name });
    };
    if bytes.len() != LIGHT_BYTES_PER_SECTION {
        return Err(SectionError::LightLengthMismatch {
            y,
            tag_name,
            actual_bytes: bytes.len(),
            expected_bytes: LIGHT_BYTES_PER_SECTION,
        });
    }
    Ok(Some(
        bytes
            .iter()
            .map(|byte| u8::from_be_bytes(byte.to_be_bytes()))
            .collect(),
    ))
}

fn parse_block_states(y: i8, value: &NbtValue) -> Result<BlockStateSection, SectionError> {
    let NbtValue::Compound(_) = value else {
        return Err(SectionError::BlockStatesNotCompound { y });
    };
    let palette_value = value
        .compound_entry("palette")
        .ok_or(SectionError::MissingPalette { y })?;
    let raw_palette_values = parse_palette(y, palette_value)?;
    if raw_palette_values.is_empty() {
        return Err(SectionError::EmptyPalette { y });
    }

    let bits_per_entry = bits_for_palette(raw_palette_values.len());
    if raw_palette_values.len() == 1 {
        if let Some(data) = value.compound_entry("data") {
            let NbtValue::LongArray(words) = data else {
                return Err(SectionError::PackedDataNotLongArray { y });
            };
            if !words.is_empty() {
                return Err(SectionError::PackedWordCountMismatch {
                    y,
                    bits_per_entry,
                    actual_words: words.len(),
                    expected_words: 0,
                });
            }
        }
        return Ok(BlockStateSection {
            raw_palette_values,
            bits_per_entry: 0,
            packed_word_count: 0,
            palette_indices: vec![0; BLOCKS_PER_SECTION],
        });
    }

    let packed = value
        .compound_entry("data")
        .ok_or(SectionError::MissingPackedData {
            y,
            palette_length: raw_palette_values.len(),
        })?;
    let NbtValue::LongArray(words) = packed else {
        return Err(SectionError::PackedDataNotLongArray { y });
    };
    let values_per_word = 64 / usize::from(bits_per_entry);
    let expected_words = BLOCKS_PER_SECTION.div_ceil(values_per_word);
    if words.len() != expected_words {
        return Err(SectionError::PackedWordCountMismatch {
            y,
            bits_per_entry,
            actual_words: words.len(),
            expected_words,
        });
    }

    let mask = (1_u64 << bits_per_entry) - 1;
    let mut palette_indices = Vec::with_capacity(BLOCKS_PER_SECTION);
    for block_index in 0..BLOCKS_PER_SECTION {
        let word_index = block_index / values_per_word;
        let index_in_word = block_index % values_per_word;
        let palette_index = ((words[word_index] as u64
            >> (index_in_word * usize::from(bits_per_entry)))
            & mask) as u32;
        if usize::try_from(palette_index).map_or(true, |index| index >= raw_palette_values.len()) {
            return Err(SectionError::PaletteIndexOutOfBounds {
                y,
                block_index,
                palette_index,
                palette_length: raw_palette_values.len(),
            });
        }
        palette_indices.push(palette_index);
    }

    Ok(BlockStateSection {
        raw_palette_values,
        bits_per_entry,
        packed_word_count: words.len(),
        palette_indices,
    })
}

fn parse_palette(y: i8, value: &NbtValue) -> Result<Vec<u32>, SectionError> {
    match value {
        NbtValue::ByteArray(values) => values
            .iter()
            .enumerate()
            .map(|(index, value)| convert_palette_value(y, index, i64::from(*value)))
            .collect(),
        NbtValue::IntArray(values) => values
            .iter()
            .enumerate()
            .map(|(index, value)| convert_palette_value(y, index, i64::from(*value)))
            .collect(),
        NbtValue::LongArray(values) => values
            .iter()
            .enumerate()
            .map(|(index, value)| convert_palette_value(y, index, *value))
            .collect(),
        NbtValue::List { values, .. } => values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let integer = match value {
                    NbtValue::Byte(value) => i64::from(*value),
                    NbtValue::Short(value) => i64::from(*value),
                    NbtValue::Int(value) => i64::from(*value),
                    NbtValue::Long(value) => *value,
                    _ => {
                        return Err(SectionError::NonIntegerPaletteEntry {
                            y,
                            entry_index: index,
                        });
                    }
                };
                convert_palette_value(y, index, integer)
            })
            .collect(),
        other => Err(SectionError::UnsupportedPaletteType {
            y,
            tag_type: tag_type_of(other),
        }),
    }
}

fn convert_palette_value(y: i8, entry_index: usize, value: i64) -> Result<u32, SectionError> {
    if value < 0 {
        return Err(SectionError::NegativePaletteValue {
            y,
            entry_index,
            value,
        });
    }
    u32::try_from(value).map_err(|_| SectionError::PaletteValueTooLarge {
        y,
        entry_index,
        value,
    })
}

fn bits_for_palette(palette_length: usize) -> u8 {
    let needed = usize::BITS - (palette_length - 1).leading_zeros();
    u8::try_from(needed)
        .unwrap_or(u8::MAX)
        .max(MINIMUM_BLOCK_BITS_PER_ENTRY)
}

fn tag_type_of(value: &NbtValue) -> NbtTagType {
    match value {
        NbtValue::Byte(_) => NbtTagType::Byte,
        NbtValue::Short(_) => NbtTagType::Short,
        NbtValue::Int(_) => NbtTagType::Int,
        NbtValue::Long(_) => NbtTagType::Long,
        NbtValue::Float(_) => NbtTagType::Float,
        NbtValue::Double(_) => NbtTagType::Double,
        NbtValue::ByteArray(_) => NbtTagType::ByteArray,
        NbtValue::String(_) => NbtTagType::String,
        NbtValue::List { .. } => NbtTagType::List,
        NbtValue::Compound(_) => NbtTagType::Compound,
        NbtValue::IntArray(_) => NbtTagType::IntArray,
        NbtValue::LongArray(_) => NbtTagType::LongArray,
    }
}

#[cfg(test)]
mod tests {
    use mythicraft_nbt::{NamedTag, NbtTagType, NbtValue};

    use super::{parse_chunk_sections, SectionError, BLOCKS_PER_SECTION, LIGHT_BYTES_PER_SECTION};

    #[test]
    fn parses_homogeneous_numeric_palette_without_registry_mapping() {
        let root = root_with_sections(vec![section(
            -4,
            NbtValue::Compound(vec![NamedTag {
                name: "palette".into(),
                value: NbtValue::List {
                    element_type: NbtTagType::Int,
                    values: vec![NbtValue::Int(1234)],
                },
            }]),
        )]);
        let sections = parse_chunk_sections(&root).expect("parse homogeneous section");
        let block_states = sections[0]
            .block_states
            .as_ref()
            .expect("block states present");
        assert_eq!(block_states.raw_palette_values, vec![1234]);
        assert_eq!(block_states.palette_indices.len(), BLOCKS_PER_SECTION);
        assert_eq!(block_states.raw_palette_value_at(4095), Some(1234));
        assert!(block_states.is_homogeneous());
    }

    #[test]
    fn decodes_non_crossing_packed_palette_indices() {
        let mut indices = vec![0_u32; BLOCKS_PER_SECTION];
        indices[0] = 1;
        indices[15] = 2;
        indices[16] = 1;
        indices[4095] = 2;
        let words = pack_indices(&indices, 4);
        let root = root_with_sections(vec![section(
            0,
            NbtValue::Compound(vec![
                NamedTag {
                    name: "palette".into(),
                    value: NbtValue::IntArray(vec![10, 20, 30]),
                },
                NamedTag {
                    name: "data".into(),
                    value: NbtValue::LongArray(words),
                },
            ]),
        )]);
        let sections = parse_chunk_sections(&root).expect("parse packed section");
        let states = sections[0].block_states.as_ref().expect("states present");
        assert_eq!(states.raw_palette_value_at(0), Some(20));
        assert_eq!(states.raw_palette_value_at(15), Some(30));
        assert_eq!(states.raw_palette_value_at(16), Some(20));
        assert_eq!(states.raw_palette_value_at(4095), Some(30));
    }

    #[test]
    fn rejects_truncated_words_and_palette_index_overflow() {
        let truncated = root_with_sections(vec![section(
            1,
            NbtValue::Compound(vec![
                NamedTag {
                    name: "palette".into(),
                    value: NbtValue::IntArray(vec![1, 2]),
                },
                NamedTag {
                    name: "data".into(),
                    value: NbtValue::LongArray(vec![0]),
                },
            ]),
        )]);
        assert_eq!(
            parse_chunk_sections(&truncated),
            Err(SectionError::PackedWordCountMismatch {
                y: 1,
                bits_per_entry: 4,
                actual_words: 1,
                expected_words: 256,
            })
        );

        let mut words = vec![0_i64; 256];
        words[0] = 15;
        let out_of_bounds = root_with_sections(vec![section(
            2,
            NbtValue::Compound(vec![
                NamedTag {
                    name: "palette".into(),
                    value: NbtValue::IntArray(vec![1, 2]),
                },
                NamedTag {
                    name: "data".into(),
                    value: NbtValue::LongArray(words),
                },
            ]),
        )]);
        assert_eq!(
            parse_chunk_sections(&out_of_bounds),
            Err(SectionError::PaletteIndexOutOfBounds {
                y: 2,
                block_index: 0,
                palette_index: 15,
                palette_length: 2,
            })
        );
    }

    #[test]
    fn rejects_duplicate_y_and_negative_palette_values() {
        let duplicate = root_with_sections(vec![
            section(3, homogeneous_states(1)),
            section(3, homogeneous_states(2)),
        ]);
        assert_eq!(
            parse_chunk_sections(&duplicate),
            Err(SectionError::DuplicateSectionY { y: 3 })
        );

        let negative = root_with_sections(vec![section(
            4,
            NbtValue::Compound(vec![NamedTag {
                name: "palette".into(),
                value: NbtValue::IntArray(vec![-1]),
            }]),
        )]);
        assert_eq!(
            parse_chunk_sections(&negative),
            Err(SectionError::NegativePaletteValue {
                y: 4,
                entry_index: 0,
                value: -1,
            })
        );
    }

    #[test]
    fn validates_fixed_light_array_capacity() {
        let mut section_entries = match section(5, homogeneous_states(1)) {
            NbtValue::Compound(entries) => entries,
            _ => unreachable!("test helper always returns compound"),
        };
        section_entries.push(NamedTag {
            name: "BlockLight".into(),
            value: NbtValue::ByteArray(vec![-1; LIGHT_BYTES_PER_SECTION]),
        });
        section_entries.push(NamedTag {
            name: "SkyLight".into(),
            value: NbtValue::ByteArray(vec![0; LIGHT_BYTES_PER_SECTION]),
        });
        let root = root_with_sections(vec![NbtValue::Compound(section_entries)]);
        let sections = parse_chunk_sections(&root).expect("parse fixed light arrays");
        assert_eq!(sections[0].block_light.as_ref().map(Vec::len), Some(2048));
        assert_eq!(
            sections[0].block_light.as_ref().map(|light| light[0]),
            Some(255)
        );
        assert_eq!(sections[0].sky_light.as_ref().map(Vec::len), Some(2048));

        let mut invalid_entries = match section(6, homogeneous_states(1)) {
            NbtValue::Compound(entries) => entries,
            _ => unreachable!("test helper always returns compound"),
        };
        invalid_entries.push(NamedTag {
            name: "BlockLight".into(),
            value: NbtValue::ByteArray(vec![0; LIGHT_BYTES_PER_SECTION - 1]),
        });
        let invalid = root_with_sections(vec![NbtValue::Compound(invalid_entries)]);
        assert_eq!(
            parse_chunk_sections(&invalid),
            Err(SectionError::LightLengthMismatch {
                y: 6,
                tag_name: "BlockLight",
                actual_bytes: LIGHT_BYTES_PER_SECTION - 1,
                expected_bytes: LIGHT_BYTES_PER_SECTION,
            })
        );
    }

    fn root_with_sections(sections: Vec<NbtValue>) -> NbtValue {
        NbtValue::Compound(vec![NamedTag {
            name: "sections".into(),
            value: NbtValue::List {
                element_type: NbtTagType::Compound,
                values: sections,
            },
        }])
    }

    fn section(y: i8, block_states: NbtValue) -> NbtValue {
        NbtValue::Compound(vec![
            NamedTag {
                name: "Y".into(),
                value: NbtValue::Byte(y),
            },
            NamedTag {
                name: "block_states".into(),
                value: block_states,
            },
        ])
    }

    fn homogeneous_states(value: i32) -> NbtValue {
        NbtValue::Compound(vec![NamedTag {
            name: "palette".into(),
            value: NbtValue::IntArray(vec![value]),
        }])
    }

    fn pack_indices(indices: &[u32], bits_per_entry: usize) -> Vec<i64> {
        let values_per_word = 64 / bits_per_entry;
        let mut words = vec![0_u64; indices.len().div_ceil(values_per_word)];
        for (index, value) in indices.iter().enumerate() {
            let word = index / values_per_word;
            let offset = index % values_per_word * bits_per_entry;
            words[word] |= u64::from(*value) << offset;
        }
        words.into_iter().map(|word| word as i64).collect()
    }
}
