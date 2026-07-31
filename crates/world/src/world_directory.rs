use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use mythicraft_nbt::{parse_named_root, NbtError, NbtLimits, NbtValue};
use thiserror::Error;

use crate::{
    compression::{decode_gzip_limited, GzipDecodeError},
    inspect_region, ChunkNbtSchema, ChunkReadLimits, RegionInspectionSummary, UnknownTagSummary,
};

pub const DEFAULT_MAX_LEVEL_DAT_COMPRESSED_BYTES: u64 = 16 * 1024 * 1024;
pub const DEFAULT_MAX_REGION_FILE_BYTES: u64 = 1024 * 1024 * 1024;
pub const DEFAULT_MAX_REGION_FILES: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorldInspectionLimits {
    pub max_level_dat_compressed_bytes: u64,
    pub max_region_file_bytes: u64,
    pub max_region_files: usize,
    pub chunk: ChunkReadLimits,
    pub nbt: NbtLimits,
}

impl Default for WorldInspectionLimits {
    fn default() -> Self {
        Self {
            max_level_dat_compressed_bytes: DEFAULT_MAX_LEVEL_DAT_COMPRESSED_BYTES,
            max_region_file_bytes: DEFAULT_MAX_REGION_FILE_BYTES,
            max_region_files: DEFAULT_MAX_REGION_FILES,
            chunk: ChunkReadLimits::default(),
            nbt: NbtLimits::default(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LevelDatSummary {
    pub data_version: i32,
    pub supported: bool,
    pub compressed_bytes: usize,
    pub decompressed_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorldInspectionSummary {
    pub level_dat: LevelDatSummary,
    pub regions: Vec<WorldRegionInspection>,
    pub issues: Vec<WorldFileIssue>,
    pub coordinate_bounds: Option<ChunkCoordinateBounds>,
    pub region_count: usize,
    pub present_chunk_count: usize,
    pub inspected_chunk_count: usize,
    pub data_versions: Vec<i32>,
    pub unknown_top_level_tags: Vec<UnknownTagSummary>,
    pub total_region_file_bytes: u64,
    pub total_decompressed_chunk_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorldRegionInspection {
    pub relative_path: String,
    pub region_x: i32,
    pub region_z: i32,
    pub file_bytes: u64,
    pub summary: RegionInspectionSummary,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChunkCoordinateBounds {
    pub minimum_x: i64,
    pub maximum_x: i64,
    pub minimum_z: i64,
    pub maximum_z: i64,
}

impl ChunkCoordinateBounds {
    fn include(&mut self, x: i64, z: i64) {
        self.minimum_x = self.minimum_x.min(x);
        self.maximum_x = self.maximum_x.max(x);
        self.minimum_z = self.minimum_z.min(z);
        self.maximum_z = self.maximum_z.max(z);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorldFileIssue {
    pub relative_path: String,
    pub kind: WorldFileIssueKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorldFileIssueKind {
    InvalidRegionFileName,
    SymlinkNotAllowed,
    FileTooLarge { actual_bytes: u64, max_bytes: u64 },
    MetadataFailed { message: String },
    ReadFailed { message: String },
    RegionInspectionFailed { message: String },
}

#[derive(Debug, Error)]
pub enum WorldInspectionError {
    #[error("failed to inspect {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("level.dat is a symlink: {path}")]
    LevelDatSymlink { path: PathBuf },
    #[error("level.dat has {actual_bytes} compressed bytes, limit is {max_bytes}")]
    LevelDatCompressedTooLarge { actual_bytes: u64, max_bytes: u64 },
    #[error("failed to decompress level.dat: {message}")]
    LevelDatDecompression { message: String },
    #[error("level.dat NBT is invalid: {0}")]
    LevelDatNbt(#[from] NbtError),
    #[error("level.dat root must be a compound")]
    LevelDatRootNotCompound,
    #[error("level.dat is missing the Data compound")]
    MissingDataCompound,
    #[error("level.dat Data compound is missing integer DataVersion")]
    MissingDataVersion,
    #[error("world has {actual_files} region files, limit is {max_files}")]
    TooManyRegionFiles {
        actual_files: usize,
        max_files: usize,
    },
    #[error("DataVersion range is invalid: {minimum}..={maximum}")]
    InvalidDataVersionRange { minimum: i32, maximum: i32 },
}

pub fn inspect_world_directory(
    world_root: &Path,
    schema: &ChunkNbtSchema,
    limits: WorldInspectionLimits,
) -> Result<WorldInspectionSummary, WorldInspectionError> {
    if schema.data_version.minimum > schema.data_version.maximum {
        return Err(WorldInspectionError::InvalidDataVersionRange {
            minimum: schema.data_version.minimum,
            maximum: schema.data_version.maximum,
        });
    }

    let level_dat = inspect_level_dat(world_root, schema, limits)?;
    let region_root = world_root.join("region");
    let mut region_paths = read_region_paths(&region_root)?;
    if region_paths.len() > limits.max_region_files {
        return Err(WorldInspectionError::TooManyRegionFiles {
            actual_files: region_paths.len(),
            max_files: limits.max_region_files,
        });
    }
    region_paths.sort_by(|left, right| left.file_name().cmp(&right.file_name()));

    let mut regions = Vec::new();
    let mut issues = Vec::new();
    let mut coordinate_bounds: Option<ChunkCoordinateBounds> = None;
    let mut present_chunk_count = 0_usize;
    let mut inspected_chunk_count = 0_usize;
    let mut data_versions = BTreeSet::from([level_dat.data_version]);
    let mut unknown_tags = BTreeMap::<String, usize>::new();
    let mut total_region_file_bytes = 0_u64;
    let mut total_decompressed_chunk_bytes = 0_u64;

    for path in region_paths {
        let relative_path = relative_region_path(&path);
        let Some((region_x, region_z)) = parse_region_coordinates(&path) else {
            issues.push(WorldFileIssue {
                relative_path,
                kind: WorldFileIssueKind::InvalidRegionFileName,
            });
            continue;
        };
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                issues.push(WorldFileIssue {
                    relative_path,
                    kind: WorldFileIssueKind::MetadataFailed {
                        message: error.to_string(),
                    },
                });
                continue;
            }
        };
        if metadata.file_type().is_symlink() {
            issues.push(WorldFileIssue {
                relative_path,
                kind: WorldFileIssueKind::SymlinkNotAllowed,
            });
            continue;
        }
        if metadata.len() > limits.max_region_file_bytes {
            issues.push(WorldFileIssue {
                relative_path,
                kind: WorldFileIssueKind::FileTooLarge {
                    actual_bytes: metadata.len(),
                    max_bytes: limits.max_region_file_bytes,
                },
            });
            continue;
        }
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                issues.push(WorldFileIssue {
                    relative_path,
                    kind: WorldFileIssueKind::ReadFailed {
                        message: error.to_string(),
                    },
                });
                continue;
            }
        };
        let summary = match inspect_region(&bytes, schema, limits.chunk, limits.nbt) {
            Ok(summary) => summary,
            Err(error) => {
                issues.push(WorldFileIssue {
                    relative_path,
                    kind: WorldFileIssueKind::RegionInspectionFailed {
                        message: error.to_string(),
                    },
                });
                continue;
            }
        };

        total_region_file_bytes = total_region_file_bytes.saturating_add(metadata.len());
        total_decompressed_chunk_bytes =
            total_decompressed_chunk_bytes.saturating_add(summary.total_decompressed_bytes);
        present_chunk_count =
            present_chunk_count.saturating_add(summary.region.present_chunk_count);
        inspected_chunk_count = inspected_chunk_count.saturating_add(summary.chunks.len());
        data_versions.extend(summary.data_versions.iter().copied());
        for unknown in &summary.unknown_top_level_tags {
            *unknown_tags.entry(unknown.name.clone()).or_default() += unknown.occurrences;
        }
        for chunk in &summary.region.chunks {
            let x = i64::from(region_x) * 32 + i64::from(chunk.local_x);
            let z = i64::from(region_z) * 32 + i64::from(chunk.local_z);
            match &mut coordinate_bounds {
                Some(bounds) => bounds.include(x, z),
                None => {
                    coordinate_bounds = Some(ChunkCoordinateBounds {
                        minimum_x: x,
                        maximum_x: x,
                        minimum_z: z,
                        maximum_z: z,
                    });
                }
            }
        }
        regions.push(WorldRegionInspection {
            relative_path,
            region_x,
            region_z,
            file_bytes: metadata.len(),
            summary,
        });
    }

    Ok(WorldInspectionSummary {
        level_dat,
        region_count: regions.len(),
        regions,
        issues,
        coordinate_bounds,
        present_chunk_count,
        inspected_chunk_count,
        data_versions: data_versions.into_iter().collect(),
        unknown_top_level_tags: unknown_tags
            .into_iter()
            .map(|(name, occurrences)| UnknownTagSummary { name, occurrences })
            .collect(),
        total_region_file_bytes,
        total_decompressed_chunk_bytes,
    })
}

fn inspect_level_dat(
    world_root: &Path,
    schema: &ChunkNbtSchema,
    limits: WorldInspectionLimits,
) -> Result<LevelDatSummary, WorldInspectionError> {
    let path = world_root.join("level.dat");
    let metadata = fs::symlink_metadata(&path).map_err(|source| WorldInspectionError::Io {
        path: path.clone(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(WorldInspectionError::LevelDatSymlink { path });
    }
    if metadata.len() > limits.max_level_dat_compressed_bytes {
        return Err(WorldInspectionError::LevelDatCompressedTooLarge {
            actual_bytes: metadata.len(),
            max_bytes: limits.max_level_dat_compressed_bytes,
        });
    }
    let compressed = fs::read(&path).map_err(|source| WorldInspectionError::Io {
        path: path.clone(),
        source,
    })?;
    let decompressed =
        decode_gzip_limited(&compressed, limits.nbt.max_bytes).map_err(map_level_dat_gzip_error)?;
    let root = parse_named_root(&decompressed, limits.nbt)?;
    let NbtValue::Compound(root_entries) = root.value else {
        return Err(WorldInspectionError::LevelDatRootNotCompound);
    };
    let data = root_entries
        .iter()
        .find(|entry| entry.name == "Data")
        .map(|entry| &entry.value)
        .ok_or(WorldInspectionError::MissingDataCompound)?;
    let data_version = data
        .compound_entry("DataVersion")
        .and_then(NbtValue::as_i32)
        .ok_or(WorldInspectionError::MissingDataVersion)?;

    Ok(LevelDatSummary {
        data_version,
        supported: data_version >= schema.data_version.minimum
            && data_version <= schema.data_version.maximum,
        compressed_bytes: compressed.len(),
        decompressed_bytes: decompressed.len(),
    })
}

fn map_level_dat_gzip_error(error: GzipDecodeError) -> WorldInspectionError {
    let message = match error {
        GzipDecodeError::Decode(message) => message,
        GzipDecodeError::TooLarge { max_bytes } => {
            format!("decompressed data exceeds {max_bytes} bytes")
        }
        GzipDecodeError::TrailingData { trailing_bytes } => {
            format!("gzip stream has {trailing_bytes} trailing bytes")
        }
    };
    WorldInspectionError::LevelDatDecompression { message }
}

fn read_region_paths(region_root: &Path) -> Result<Vec<PathBuf>, WorldInspectionError> {
    let entries = match fs::read_dir(region_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(WorldInspectionError::Io {
                path: region_root.to_path_buf(),
                source,
            });
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| WorldInspectionError::Io {
            path: region_root.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) == Some("mca") {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn parse_region_coordinates(path: &Path) -> Option<(i32, i32)> {
    let name = path.file_name()?.to_str()?;
    let body = name.strip_prefix("r.")?.strip_suffix(".mca")?;
    let mut parts = body.split('.');
    let x = parts.next()?.parse().ok()?;
    let z = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((x, z))
}

fn relative_region_path(path: &Path) -> String {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    format!("region/{file_name}")
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Write, path::PathBuf};

    use flate2::{write::GzEncoder, Compression};
    use mythicraft_api::DataVersionRange;

    use super::{
        inspect_world_directory, ChunkCoordinateBounds, WorldFileIssueKind, WorldInspectionLimits,
    };
    use crate::{ChunkNbtSchema, REGION_SECTOR_BYTES};

    #[test]
    fn scans_regions_in_stable_order_and_aggregates_coordinates() {
        let root = temp_world("stable");
        write_level_dat(&root, 4903);
        let region_root = root.join("region");
        fs::create_dir_all(&region_root).expect("create region directory");
        fs::write(
            region_root.join("r.1.0.mca"),
            region_with_chunk(0, 2, &chunk_nbt(4903, "future")),
        )
        .expect("write positive region");
        fs::write(
            region_root.join("r.-1.0.mca"),
            region_with_chunk(31, 2, &chunk_nbt(4903, "sections")),
        )
        .expect("write negative region");

        let schema = schema();
        let summary = inspect_world_directory(&root, &schema, WorldInspectionLimits::default())
            .expect("inspect synthetic world");
        assert_eq!(summary.region_count, 2);
        assert_eq!(summary.present_chunk_count, 2);
        assert_eq!(summary.inspected_chunk_count, 2);
        assert_eq!(summary.regions[0].relative_path, "region/r.-1.0.mca");
        assert_eq!(summary.regions[1].relative_path, "region/r.1.0.mca");
        assert_eq!(
            summary.coordinate_bounds,
            Some(ChunkCoordinateBounds {
                minimum_x: -1,
                maximum_x: 32,
                minimum_z: 0,
                maximum_z: 0,
            })
        );
        assert_eq!(summary.unknown_top_level_tags[0].name, "future");
        assert_eq!(summary.data_versions, vec![4903]);
        fs::remove_dir_all(root).expect("cleanup synthetic world");
    }

    #[test]
    fn records_corrupt_region_and_continues() {
        let root = temp_world("corrupt");
        write_level_dat(&root, 4903);
        let region_root = root.join("region");
        fs::create_dir_all(&region_root).expect("create region directory");
        fs::write(region_root.join("r.0.0.mca"), [0_u8; 39]).expect("write corrupt region");
        fs::write(region_root.join("broken.mca"), [0_u8; 8192])
            .expect("write malformed region name");

        let summary = inspect_world_directory(&root, &schema(), WorldInspectionLimits::default())
            .expect("inspect world despite corrupt regions");
        assert_eq!(summary.region_count, 0);
        assert_eq!(summary.issues.len(), 2);
        assert!(summary
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, WorldFileIssueKind::InvalidRegionFileName)));
        assert!(summary.issues.iter().any(|issue| matches!(
            issue.kind,
            WorldFileIssueKind::RegionInspectionFailed { .. }
        )));
        fs::remove_dir_all(root).expect("cleanup synthetic world");
    }

    #[test]
    fn reports_unsupported_level_data_version_without_accepting_it() {
        let root = temp_world("unsupported");
        write_level_dat(&root, i32::MAX);
        let summary = inspect_world_directory(&root, &schema(), WorldInspectionLimits::default())
            .expect("inspect unsupported version metadata");
        assert_eq!(summary.level_dat.data_version, i32::MAX);
        assert!(!summary.level_dat.supported);
        assert_eq!(summary.data_versions, vec![i32::MAX]);
        fs::remove_dir_all(root).expect("cleanup synthetic world");
    }

    fn schema() -> ChunkNbtSchema {
        ChunkNbtSchema::new(
            DataVersionRange {
                minimum: 4903,
                maximum: 4903,
            },
            ["sections"],
        )
    }

    fn temp_world(label: &str) -> PathBuf {
        let unique = format!(
            "mythicraft-world-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time after epoch")
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }

    fn write_level_dat(root: &PathBuf, data_version: i32) {
        fs::create_dir_all(root).expect("create world root");
        let mut nbt = vec![10, 0, 0, 10, 0, 4, b'D', b'a', b't', b'a'];
        push_named_int(&mut nbt, "DataVersion", data_version);
        nbt.extend_from_slice(&[0, 0]);
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&nbt).expect("compress level.dat");
        fs::write(
            root.join("level.dat"),
            encoder.finish().expect("finish level.dat compression"),
        )
        .expect("write level.dat");
    }

    fn chunk_nbt(data_version: i32, extra_tag: &str) -> Vec<u8> {
        let mut nbt = vec![10, 0, 0];
        push_named_int(&mut nbt, "DataVersion", data_version);
        push_named_int(&mut nbt, extra_tag, 1);
        nbt.push(0);
        nbt
    }

    fn push_named_int(bytes: &mut Vec<u8>, name: &str, value: i32) {
        bytes.push(3);
        let name_length = u16::try_from(name.len()).expect("synthetic name length");
        bytes.extend_from_slice(&name_length.to_be_bytes());
        bytes.extend_from_slice(name.as_bytes());
        bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn region_with_chunk(index: usize, sector: u32, nbt: &[u8]) -> Vec<u8> {
        let mut region = vec![0; (sector as usize + 1) * REGION_SECTOR_BYTES];
        let location = (sector << 8) | 1;
        let header_offset = index * 4;
        region[header_offset..header_offset + 4].copy_from_slice(&location.to_be_bytes());
        let chunk_offset = sector as usize * REGION_SECTOR_BYTES;
        let declared_length = u32::try_from(nbt.len() + 1).expect("synthetic chunk length");
        region[chunk_offset..chunk_offset + 4].copy_from_slice(&declared_length.to_be_bytes());
        region[chunk_offset + 4] = 3;
        region[chunk_offset + 5..chunk_offset + 5 + nbt.len()].copy_from_slice(nbt);
        region
    }
}
