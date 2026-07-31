use std::collections::{BTreeMap, BTreeSet};

use mythicraft_api::DataVersionRange;
use mythicraft_nbt::{parse_named_root, NbtError, NbtLimits, NbtValue};
use thiserror::Error;

use crate::{
    parse_chunk_sections, parse_heightmaps, ChunkCompression, ChunkError, ChunkReadLimits,
    HeightmapError, RegionError, RegionFile, RegionSummary, SectionError,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkNbtSchema {
    pub data_version: DataVersionRange,
    pub known_top_level_tags: BTreeSet<String>,
}

impl ChunkNbtSchema {
    pub fn new(
        data_version: DataVersionRange,
        known_top_level_tags: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            data_version,
            known_top_level_tags: known_top_level_tags.into_iter().map(Into::into).collect(),
        }
    }

    fn supports_data_version(&self, data_version: i32) -> bool {
        data_version >= self.data_version.minimum && data_version <= self.data_version.maximum
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegionInspectionSummary {
    pub region: RegionSummary,
    pub chunks: Vec<ChunkInspection>,
    pub issues: Vec<ChunkInspectionIssue>,
    pub data_versions: Vec<i32>,
    pub unknown_top_level_tags: Vec<UnknownTagSummary>,
    pub total_decompressed_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkInspection {
    pub index: u16,
    pub local_x: u8,
    pub local_z: u8,
    pub timestamp: u32,
    pub compression: ChunkCompression,
    pub declared_length: u32,
    pub compressed_bytes: usize,
    pub decompressed_bytes: usize,
    pub root_name: String,
    pub data_version: Option<i32>,
    pub top_level_tag_count: usize,
    pub sections: Vec<SectionInspectionSummary>,
    pub heightmaps: Vec<HeightmapInspectionSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SectionInspectionSummary {
    pub y: i8,
    pub has_block_states: bool,
    pub palette_entry_count: usize,
    pub bits_per_entry: u8,
    pub packed_word_count: usize,
    pub decoded_block_count: usize,
    pub homogeneous: bool,
    pub block_light_bytes: usize,
    pub sky_light_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeightmapInspectionSummary {
    pub name: String,
    pub packed_word_count: usize,
    pub decoded_column_count: usize,
    pub minimum_value: u16,
    pub maximum_value: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnknownTagSummary {
    pub name: String,
    pub occurrences: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkInspectionIssue {
    pub index: u16,
    pub local_x: u8,
    pub local_z: u8,
    pub kind: ChunkInspectionIssueKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChunkInspectionIssueKind {
    ChunkRead(ChunkError),
    Nbt(NbtError),
    Sections(SectionError),
    Heightmaps(HeightmapError),
    RootNotCompound,
    MissingDataVersion,
    InvalidDataVersionType,
    UnsupportedDataVersion {
        actual: i32,
        minimum: i32,
        maximum: i32,
    },
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RegionInspectionError {
    #[error(transparent)]
    Region(#[from] RegionError),
    #[error("DataVersion range is invalid: {minimum}..={maximum}")]
    InvalidDataVersionRange { minimum: i32, maximum: i32 },
}

pub fn inspect_region(
    region_bytes: &[u8],
    schema: &ChunkNbtSchema,
    chunk_limits: ChunkReadLimits,
    nbt_limits: NbtLimits,
) -> Result<RegionInspectionSummary, RegionInspectionError> {
    if schema.data_version.minimum > schema.data_version.maximum {
        return Err(RegionInspectionError::InvalidDataVersionRange {
            minimum: schema.data_version.minimum,
            maximum: schema.data_version.maximum,
        });
    }

    let region_file = RegionFile::parse(region_bytes)?;
    let region = region_file.header().summary();
    let mut chunks = Vec::with_capacity(region.present_chunk_count);
    let mut issues = Vec::new();
    let mut data_versions = BTreeSet::new();
    let mut unknown_tags = BTreeMap::<String, usize>::new();
    let mut total_decompressed_bytes = 0_u64;

    for chunk_summary in &region.chunks {
        let chunk = match region_file.chunk(chunk_summary.local_x, chunk_summary.local_z) {
            Ok(Some(chunk)) => chunk,
            Ok(None) => continue,
            Err(error) => {
                issues.push(issue(
                    chunk_summary,
                    ChunkInspectionIssueKind::ChunkRead(error),
                ));
                continue;
            }
        };
        let decompressed = match chunk.decompress(chunk_limits) {
            Ok(decompressed) => decompressed,
            Err(error) => {
                issues.push(issue(
                    chunk_summary,
                    ChunkInspectionIssueKind::ChunkRead(error),
                ));
                continue;
            }
        };
        let root = match parse_named_root(&decompressed, nbt_limits) {
            Ok(root) => root,
            Err(error) => {
                issues.push(issue(chunk_summary, ChunkInspectionIssueKind::Nbt(error)));
                continue;
            }
        };

        let NbtValue::Compound(entries) = &root.value else {
            issues.push(issue(
                chunk_summary,
                ChunkInspectionIssueKind::RootNotCompound,
            ));
            continue;
        };

        let data_version_value = entries
            .iter()
            .find(|entry| entry.name == "DataVersion")
            .map(|entry| &entry.value);
        let data_version = match data_version_value {
            Some(NbtValue::Int(data_version)) => {
                data_versions.insert(*data_version);
                if !schema.supports_data_version(*data_version) {
                    issues.push(issue(
                        chunk_summary,
                        ChunkInspectionIssueKind::UnsupportedDataVersion {
                            actual: *data_version,
                            minimum: schema.data_version.minimum,
                            maximum: schema.data_version.maximum,
                        },
                    ));
                }
                Some(*data_version)
            }
            Some(_) => {
                issues.push(issue(
                    chunk_summary,
                    ChunkInspectionIssueKind::InvalidDataVersionType,
                ));
                None
            }
            None => {
                issues.push(issue(
                    chunk_summary,
                    ChunkInspectionIssueKind::MissingDataVersion,
                ));
                None
            }
        };

        for entry in entries {
            if entry.name != "DataVersion"
                && !schema.known_top_level_tags.contains(entry.name.as_str())
            {
                *unknown_tags.entry(entry.name.clone()).or_default() += 1;
            }
        }

        let sections = match parse_chunk_sections(&root.value) {
            Ok(sections) => sections
                .into_iter()
                .map(|section| {
                    let Some(block_states) = section.block_states else {
                        return SectionInspectionSummary {
                            y: section.y,
                            has_block_states: false,
                            palette_entry_count: 0,
                            bits_per_entry: 0,
                            packed_word_count: 0,
                            decoded_block_count: 0,
                            homogeneous: false,
                            block_light_bytes: section.block_light.as_ref().map_or(0, Vec::len),
                            sky_light_bytes: section.sky_light.as_ref().map_or(0, Vec::len),
                        };
                    };
                    SectionInspectionSummary {
                        y: section.y,
                        has_block_states: true,
                        palette_entry_count: block_states.raw_palette_values.len(),
                        bits_per_entry: block_states.bits_per_entry,
                        packed_word_count: block_states.packed_word_count,
                        decoded_block_count: block_states.palette_indices.len(),
                        homogeneous: block_states.is_homogeneous(),
                        block_light_bytes: section.block_light.as_ref().map_or(0, Vec::len),
                        sky_light_bytes: section.sky_light.as_ref().map_or(0, Vec::len),
                    }
                })
                .collect(),
            Err(error) => {
                issues.push(issue(
                    chunk_summary,
                    ChunkInspectionIssueKind::Sections(error),
                ));
                Vec::new()
            }
        };
        let heightmaps = match parse_heightmaps(&root.value) {
            Ok(heightmaps) => heightmaps
                .into_iter()
                .map(|heightmap| {
                    let minimum_value = heightmap.values.iter().copied().min().unwrap_or(0);
                    let maximum_value = heightmap.values.iter().copied().max().unwrap_or(0);
                    HeightmapInspectionSummary {
                        name: heightmap.name,
                        packed_word_count: heightmap.packed_word_count,
                        decoded_column_count: heightmap.values.len(),
                        minimum_value,
                        maximum_value,
                    }
                })
                .collect(),
            Err(error) => {
                issues.push(issue(
                    chunk_summary,
                    ChunkInspectionIssueKind::Heightmaps(error),
                ));
                Vec::new()
            }
        };

        total_decompressed_bytes = total_decompressed_bytes
            .saturating_add(u64::try_from(decompressed.len()).unwrap_or(u64::MAX));
        chunks.push(ChunkInspection {
            index: chunk_summary.index,
            local_x: chunk_summary.local_x,
            local_z: chunk_summary.local_z,
            timestamp: chunk_summary.timestamp,
            compression: chunk.compression,
            declared_length: chunk.declared_length,
            compressed_bytes: chunk.compressed_payload.len(),
            decompressed_bytes: decompressed.len(),
            root_name: root.name,
            data_version,
            top_level_tag_count: entries.len(),
            sections,
            heightmaps,
        });
    }

    Ok(RegionInspectionSummary {
        region,
        chunks,
        issues,
        data_versions: data_versions.into_iter().collect(),
        unknown_top_level_tags: unknown_tags
            .into_iter()
            .map(|(name, occurrences)| UnknownTagSummary { name, occurrences })
            .collect(),
        total_decompressed_bytes,
    })
}

fn issue(
    chunk: &crate::ChunkSectorSummary,
    kind: ChunkInspectionIssueKind,
) -> ChunkInspectionIssue {
    ChunkInspectionIssue {
        index: chunk.index,
        local_x: chunk.local_x,
        local_z: chunk.local_z,
        kind,
    }
}

#[cfg(test)]
mod tests {
    use mythicraft_api::DataVersionRange;
    use mythicraft_nbt::NbtLimits;

    use super::{
        inspect_region, ChunkInspectionIssueKind, ChunkNbtSchema, RegionInspectionError,
        UnknownTagSummary,
    };
    use crate::{ChunkReadLimits, REGION_SECTOR_BYTES};

    #[test]
    fn produces_stable_summary_with_unknown_tags_and_versions() {
        let first = compound_chunk(4903, &[("sections", 1), ("future_tag", 2)]);
        let second = compound_chunk(i32::MAX, &[("future_tag", 3)]);
        let region = region_with_chunks(&[(0, 2, &first), (33, 3, &second)]);
        let schema = ChunkNbtSchema::new(
            DataVersionRange {
                minimum: 4903,
                maximum: 4903,
            },
            ["sections"],
        );

        let summary = inspect_region(
            &region,
            &schema,
            ChunkReadLimits::default(),
            NbtLimits::default(),
        )
        .expect("inspect synthetic region");

        assert_eq!(summary.region.present_chunk_count, 2);
        assert_eq!(summary.chunks.len(), 2);
        assert_eq!(summary.chunks[0].index, 0);
        assert_eq!(summary.chunks[1].index, 33);
        assert_eq!(summary.data_versions, vec![4903, i32::MAX]);
        assert_eq!(
            summary.unknown_top_level_tags,
            vec![UnknownTagSummary {
                name: "future_tag".into(),
                occurrences: 2,
            }]
        );
        assert!(summary.issues.iter().any(|issue| matches!(
            issue.kind,
            ChunkInspectionIssueKind::UnsupportedDataVersion {
                actual: i32::MAX,
                minimum: 4903,
                maximum: 4903,
            }
        )));
        assert_eq!(
            summary,
            inspect_region(
                &region,
                &schema,
                ChunkReadLimits::default(),
                NbtLimits::default(),
            )
            .expect("repeat synthetic inspection")
        );
    }

    #[test]
    fn records_chunk_coordinates_for_corrupt_nbt() {
        let region = region_with_chunks(&[(7, 2, &[99])]);
        let schema = ChunkNbtSchema::new(
            DataVersionRange {
                minimum: 4903,
                maximum: 4903,
            },
            std::iter::empty::<String>(),
        );

        let summary = inspect_region(
            &region,
            &schema,
            ChunkReadLimits::default(),
            NbtLimits::default(),
        )
        .expect("header remains inspectable");
        assert_eq!(summary.chunks.len(), 0);
        assert_eq!(summary.issues.len(), 1);
        assert_eq!(summary.issues[0].index, 7);
        assert_eq!(summary.issues[0].local_x, 7);
        assert_eq!(summary.issues[0].local_z, 0);
        assert!(matches!(
            summary.issues[0].kind,
            ChunkInspectionIssueKind::Nbt(_)
        ));
    }

    #[test]
    fn inspects_section_palette_from_raw_chunk_nbt() {
        let chunk = chunk_with_homogeneous_section(4903, -4, 1234);
        let region = region_with_chunks(&[(0, 2, &chunk)]);
        let schema = ChunkNbtSchema::new(
            DataVersionRange {
                minimum: 4903,
                maximum: 4903,
            },
            ["sections"],
        );
        let summary = inspect_region(
            &region,
            &schema,
            ChunkReadLimits::default(),
            NbtLimits::default(),
        )
        .expect("inspect section chunk");

        assert_eq!(summary.issues, Vec::new());
        assert_eq!(summary.chunks[0].sections.len(), 1);
        assert_eq!(summary.chunks[0].sections[0].y, -4);
        assert_eq!(summary.chunks[0].sections[0].palette_entry_count, 1);
        assert_eq!(summary.chunks[0].sections[0].decoded_block_count, 4096);
        assert!(summary.chunks[0].sections[0].homogeneous);
    }

    #[test]
    fn rejects_invalid_schema_range_before_parsing() {
        let schema = ChunkNbtSchema::new(
            DataVersionRange {
                minimum: 10,
                maximum: 9,
            },
            std::iter::empty::<String>(),
        );
        assert_eq!(
            inspect_region(
                &[],
                &schema,
                ChunkReadLimits::default(),
                NbtLimits::default(),
            ),
            Err(RegionInspectionError::InvalidDataVersionRange {
                minimum: 10,
                maximum: 9,
            })
        );
    }

    fn compound_chunk(data_version: i32, fields: &[(&str, i32)]) -> Vec<u8> {
        let mut bytes = vec![10, 0, 0];
        push_named_int(&mut bytes, "DataVersion", data_version);
        for (name, value) in fields {
            push_named_int(&mut bytes, name, *value);
        }
        bytes.push(0);
        bytes
    }

    fn chunk_with_homogeneous_section(
        data_version: i32,
        section_y: i8,
        palette_value: i32,
    ) -> Vec<u8> {
        let mut bytes = vec![10, 0, 0];
        push_named_int(&mut bytes, "DataVersion", data_version);
        bytes.push(9);
        push_name(&mut bytes, "sections");
        bytes.push(10);
        bytes.extend_from_slice(&1_i32.to_be_bytes());
        bytes.push(1);
        push_name(&mut bytes, "Y");
        bytes.push(section_y as u8);
        bytes.push(10);
        push_name(&mut bytes, "block_states");
        bytes.push(11);
        push_name(&mut bytes, "palette");
        bytes.extend_from_slice(&1_i32.to_be_bytes());
        bytes.extend_from_slice(&palette_value.to_be_bytes());
        bytes.push(0);
        bytes.push(0);
        bytes.push(0);
        bytes
    }

    fn push_named_int(bytes: &mut Vec<u8>, name: &str, value: i32) {
        bytes.push(3);
        push_name(bytes, name);
        bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn push_name(bytes: &mut Vec<u8>, name: &str) {
        let name_length = u16::try_from(name.len()).expect("synthetic tag name length");
        bytes.extend_from_slice(&name_length.to_be_bytes());
        bytes.extend_from_slice(name.as_bytes());
    }

    fn region_with_chunks(chunks: &[(usize, u32, &[u8])]) -> Vec<u8> {
        let sector_count = chunks
            .iter()
            .map(|(_, sector, _)| *sector as usize + 1)
            .max()
            .unwrap_or(2);
        let mut region = vec![0; sector_count * REGION_SECTOR_BYTES];
        for (index, sector, nbt) in chunks {
            let location = (*sector << 8) | 1;
            let header_offset = *index * 4;
            region[header_offset..header_offset + 4].copy_from_slice(&location.to_be_bytes());
            let chunk_offset = *sector as usize * REGION_SECTOR_BYTES;
            let declared_length = u32::try_from(nbt.len() + 1).expect("synthetic chunk length");
            region[chunk_offset..chunk_offset + 4].copy_from_slice(&declared_length.to_be_bytes());
            region[chunk_offset + 4] = 3;
            region[chunk_offset + 5..chunk_offset + 5 + nbt.len()].copy_from_slice(nbt);
        }
        region
    }
}
