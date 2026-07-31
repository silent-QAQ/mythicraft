use std::collections::BTreeSet;

use mythicraft_nbt::NbtValue;
use thiserror::Error;

pub const HEIGHTMAP_COLUMNS: usize = 16 * 16;
pub const HEIGHTMAP_BITS_PER_ENTRY: usize = 9;
const HEIGHTMAP_VALUES_PER_WORD: usize = 64 / HEIGHTMAP_BITS_PER_ENTRY;
pub const HEIGHTMAP_PACKED_WORDS: usize = HEIGHTMAP_COLUMNS.div_ceil(HEIGHTMAP_VALUES_PER_WORD);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Heightmap {
    pub name: String,
    pub packed_word_count: usize,
    pub values: Vec<u16>,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum HeightmapError {
    #[error("Heightmaps must be a compound")]
    HeightmapsNotCompound,
    #[error("heightmap {name} is duplicated")]
    DuplicateHeightmap { name: String },
    #[error("heightmap {name} must be a long array")]
    HeightmapNotLongArray { name: String },
    #[error("heightmap {name} has {actual_words} packed words, expected {expected_words}")]
    PackedWordCountMismatch {
        name: String,
        actual_words: usize,
        expected_words: usize,
    },
}

pub fn parse_heightmaps(root: &NbtValue) -> Result<Vec<Heightmap>, HeightmapError> {
    let Some(value) = root.compound_entry("Heightmaps") else {
        return Ok(Vec::new());
    };
    let NbtValue::Compound(entries) = value else {
        return Err(HeightmapError::HeightmapsNotCompound);
    };
    let mut seen = BTreeSet::new();
    let mut heightmaps = Vec::with_capacity(entries.len());
    for entry in entries {
        if !seen.insert(entry.name.clone()) {
            return Err(HeightmapError::DuplicateHeightmap {
                name: entry.name.clone(),
            });
        }
        let NbtValue::LongArray(words) = &entry.value else {
            return Err(HeightmapError::HeightmapNotLongArray {
                name: entry.name.clone(),
            });
        };
        if words.len() != HEIGHTMAP_PACKED_WORDS {
            return Err(HeightmapError::PackedWordCountMismatch {
                name: entry.name.clone(),
                actual_words: words.len(),
                expected_words: HEIGHTMAP_PACKED_WORDS,
            });
        }
        let mut values = Vec::with_capacity(HEIGHTMAP_COLUMNS);
        let mask = (1_u64 << HEIGHTMAP_BITS_PER_ENTRY) - 1;
        for column in 0..HEIGHTMAP_COLUMNS {
            let word_index = column / HEIGHTMAP_VALUES_PER_WORD;
            let index_in_word = column % HEIGHTMAP_VALUES_PER_WORD;
            let value = ((words[word_index] as u64 >> (index_in_word * HEIGHTMAP_BITS_PER_ENTRY))
                & mask) as u16;
            values.push(value);
        }
        heightmaps.push(Heightmap {
            name: entry.name.clone(),
            packed_word_count: words.len(),
            values,
        });
    }
    heightmaps.sort_unstable_by(|left, right| left.name.cmp(&right.name));
    Ok(heightmaps)
}

#[cfg(test)]
mod tests {
    use mythicraft_nbt::{NamedTag, NbtValue};

    use super::{
        parse_heightmaps, HeightmapError, HEIGHTMAP_BITS_PER_ENTRY, HEIGHTMAP_COLUMNS,
        HEIGHTMAP_PACKED_WORDS,
    };

    #[test]
    fn decodes_stable_heightmap_columns() {
        let mut values = vec![0_u16; HEIGHTMAP_COLUMNS];
        values[0] = 1;
        values[6] = 511;
        values[7] = 200;
        values[255] = 384;
        let root = heightmap_root("WORLD_SURFACE", pack(&values));
        let heightmaps = parse_heightmaps(&root).expect("parse heightmap");
        assert_eq!(heightmaps[0].name, "WORLD_SURFACE");
        assert_eq!(heightmaps[0].packed_word_count, HEIGHTMAP_PACKED_WORDS);
        assert_eq!(heightmaps[0].values, values);
    }

    #[test]
    fn rejects_wrong_word_count_and_tag_type() {
        let truncated = heightmap_root("MOTION_BLOCKING", vec![0; HEIGHTMAP_PACKED_WORDS - 1]);
        assert_eq!(
            parse_heightmaps(&truncated),
            Err(HeightmapError::PackedWordCountMismatch {
                name: "MOTION_BLOCKING".into(),
                actual_words: HEIGHTMAP_PACKED_WORDS - 1,
                expected_words: HEIGHTMAP_PACKED_WORDS,
            })
        );

        let wrong_type = NbtValue::Compound(vec![NamedTag {
            name: "Heightmaps".into(),
            value: NbtValue::Compound(vec![NamedTag {
                name: "WORLD_SURFACE".into(),
                value: NbtValue::IntArray(vec![0; HEIGHTMAP_PACKED_WORDS]),
            }]),
        }]);
        assert_eq!(
            parse_heightmaps(&wrong_type),
            Err(HeightmapError::HeightmapNotLongArray {
                name: "WORLD_SURFACE".into(),
            })
        );
    }

    fn heightmap_root(name: &str, words: Vec<i64>) -> NbtValue {
        NbtValue::Compound(vec![NamedTag {
            name: "Heightmaps".into(),
            value: NbtValue::Compound(vec![NamedTag {
                name: name.into(),
                value: NbtValue::LongArray(words),
            }]),
        }])
    }

    fn pack(values: &[u16]) -> Vec<i64> {
        let values_per_word = 64 / HEIGHTMAP_BITS_PER_ENTRY;
        let mut words = vec![0_u64; HEIGHTMAP_PACKED_WORDS];
        for (index, value) in values.iter().enumerate() {
            let word = index / values_per_word;
            let offset = index % values_per_word * HEIGHTMAP_BITS_PER_ENTRY;
            words[word] |= u64::from(*value) << offset;
        }
        words.into_iter().map(|word| word as i64).collect()
    }
}
