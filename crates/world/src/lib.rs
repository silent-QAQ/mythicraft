mod chunk;
mod compression;
mod heightmap;
mod inspection;
mod region;
mod section;
mod world_directory;

pub use chunk::{
    ChunkCompression, ChunkData, ChunkError, ChunkReadLimits, RegionFile,
    DEFAULT_MAX_CHUNK_COMPRESSED_BYTES, DEFAULT_MAX_CHUNK_DECOMPRESSED_BYTES,
};
pub use heightmap::{
    parse_heightmaps, Heightmap, HeightmapError, HEIGHTMAP_BITS_PER_ENTRY, HEIGHTMAP_COLUMNS,
    HEIGHTMAP_PACKED_WORDS,
};
pub use inspection::{
    inspect_region, ChunkInspection, ChunkInspectionIssue, ChunkInspectionIssueKind,
    ChunkNbtSchema, HeightmapInspectionSummary, RegionInspectionError, RegionInspectionSummary,
    SectionInspectionSummary, UnknownTagSummary,
};
pub use region::{
    ChunkLocation, ChunkSectorSummary, RegionError, RegionHeader, RegionSummary, SectorRange,
    REGION_HEADER_BYTES, REGION_LOCATION_COUNT, REGION_SECTOR_BYTES,
};
pub use section::{
    parse_chunk_sections, BlockStateSection, ChunkSection, SectionError, BLOCKS_PER_SECTION,
    LIGHT_BYTES_PER_SECTION,
};
pub use world_directory::{
    inspect_world_directory, ChunkCoordinateBounds, LevelDatSummary, WorldFileIssue,
    WorldFileIssueKind, WorldInspectionError, WorldInspectionLimits, WorldInspectionSummary,
    WorldRegionInspection, DEFAULT_MAX_LEVEL_DAT_COMPRESSED_BYTES, DEFAULT_MAX_REGION_FILES,
    DEFAULT_MAX_REGION_FILE_BYTES,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChunkPosition {
    pub x: i32,
    pub z: i32,
}
