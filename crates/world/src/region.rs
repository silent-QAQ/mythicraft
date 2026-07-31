use std::ops::Range;

use thiserror::Error;

pub const REGION_SECTOR_BYTES: usize = 4096;
pub const REGION_LOCATION_COUNT: usize = 1024;
pub const REGION_HEADER_BYTES: usize = REGION_SECTOR_BYTES * 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SectorRange {
    pub start: u32,
    pub end_exclusive: u32,
}

impl SectorRange {
    pub fn byte_range(self) -> Range<u64> {
        u64::from(self.start) * REGION_SECTOR_BYTES as u64
            ..u64::from(self.end_exclusive) * REGION_SECTOR_BYTES as u64
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChunkLocation {
    pub sector_range: SectorRange,
}

impl ChunkLocation {
    pub fn sector_count(self) -> u32 {
        self.sector_range.end_exclusive - self.sector_range.start
    }

    pub fn byte_range(self) -> Range<u64> {
        self.sector_range.byte_range()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegionHeader {
    locations: [Option<ChunkLocation>; REGION_LOCATION_COUNT],
    timestamps: [u32; REGION_LOCATION_COUNT],
}

impl RegionHeader {
    pub fn parse(region_file: &[u8]) -> Result<Self, RegionError> {
        if region_file.len() < REGION_HEADER_BYTES {
            return Err(RegionError::TruncatedHeader {
                actual_bytes: region_file.len(),
                required_bytes: REGION_HEADER_BYTES,
            });
        }

        let mut locations = [None; REGION_LOCATION_COUNT];
        for (index, location) in locations.iter_mut().enumerate() {
            let entry_offset = index * 4;
            let raw = u32::from_be_bytes([
                region_file[entry_offset],
                region_file[entry_offset + 1],
                region_file[entry_offset + 2],
                region_file[entry_offset + 3],
            ]);
            let sector_offset = raw >> 8;
            let sector_count = raw & 0xff;

            *location = parse_location(index, sector_offset, sector_count, region_file.len())?;
        }

        validate_no_overlaps(&locations)?;

        let mut timestamps = [0; REGION_LOCATION_COUNT];
        for (index, timestamp) in timestamps.iter_mut().enumerate() {
            let entry_offset = REGION_SECTOR_BYTES + index * 4;
            *timestamp = u32::from_be_bytes([
                region_file[entry_offset],
                region_file[entry_offset + 1],
                region_file[entry_offset + 2],
                region_file[entry_offset + 3],
            ]);
        }

        Ok(Self {
            locations,
            timestamps,
        })
    }

    pub fn location(&self, local_x: u8, local_z: u8) -> Option<ChunkLocation> {
        chunk_index(local_x, local_z).and_then(|index| self.locations[index])
    }

    pub fn timestamp(&self, local_x: u8, local_z: u8) -> Option<u32> {
        chunk_index(local_x, local_z).map(|index| self.timestamps[index])
    }

    pub fn summary(&self) -> RegionSummary {
        let chunks = self
            .locations
            .iter()
            .enumerate()
            .filter_map(|(index, location)| {
                location.map(|location| ChunkSectorSummary {
                    index: index as u16,
                    local_x: (index % 32) as u8,
                    local_z: (index / 32) as u8,
                    sector_range: location.sector_range,
                    timestamp: self.timestamps[index],
                })
            })
            .collect::<Vec<_>>();
        let timestamp_min = chunks.iter().map(|chunk| chunk.timestamp).min();
        let timestamp_max = chunks.iter().map(|chunk| chunk.timestamp).max();
        let allocated_sector_count = chunks
            .iter()
            .map(|chunk| u64::from(chunk.sector_range.end_exclusive - chunk.sector_range.start))
            .sum();

        RegionSummary {
            present_chunk_count: chunks.len(),
            allocated_sector_count,
            timestamp_min,
            timestamp_max,
            chunks,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkSectorSummary {
    pub index: u16,
    pub local_x: u8,
    pub local_z: u8,
    pub sector_range: SectorRange,
    pub timestamp: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegionSummary {
    pub present_chunk_count: usize,
    pub allocated_sector_count: u64,
    pub timestamp_min: Option<u32>,
    pub timestamp_max: Option<u32>,
    pub chunks: Vec<ChunkSectorSummary>,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RegionError {
    #[error(
        "region header is truncated: got {actual_bytes} bytes, need at least {required_bytes}"
    )]
    TruncatedHeader {
        actual_bytes: usize,
        required_bytes: usize,
    },
    #[error(
        "chunk location {index} has inconsistent empty fields: sector offset {sector_offset}, sector count {sector_count}"
    )]
    InconsistentEmptyLocation {
        index: usize,
        sector_offset: u32,
        sector_count: u32,
    },
    #[error("chunk location {index} points into the region header at sector {sector_offset}")]
    LocationPointsIntoHeader { index: usize, sector_offset: u32 },
    #[error(
        "chunk location {index} sector range overflows: start {sector_offset}, count {sector_count}"
    )]
    SectorRangeOverflow {
        index: usize,
        sector_offset: u32,
        sector_count: u32,
    },
    #[error(
        "chunk location {index} is outside the region file: bytes {start_byte}..{end_byte}, file length {file_bytes}"
    )]
    LocationOutOfBounds {
        index: usize,
        start_byte: u64,
        end_byte: u64,
        file_bytes: u64,
    },
    #[error(
        "chunk locations {first_index} and {second_index} overlap in sectors {overlap_start}..{overlap_end}"
    )]
    OverlappingLocations {
        first_index: usize,
        second_index: usize,
        overlap_start: u32,
        overlap_end: u32,
    },
}

fn parse_location(
    index: usize,
    sector_offset: u32,
    sector_count: u32,
    file_bytes: usize,
) -> Result<Option<ChunkLocation>, RegionError> {
    match (sector_offset, sector_count) {
        (0, 0) => return Ok(None),
        (0, _) | (_, 0) => {
            return Err(RegionError::InconsistentEmptyLocation {
                index,
                sector_offset,
                sector_count,
            });
        }
        _ => {}
    }

    if sector_offset < 2 {
        return Err(RegionError::LocationPointsIntoHeader {
            index,
            sector_offset,
        });
    }

    let end_exclusive =
        sector_offset
            .checked_add(sector_count)
            .ok_or(RegionError::SectorRangeOverflow {
                index,
                sector_offset,
                sector_count,
            })?;
    let sector_range = SectorRange {
        start: sector_offset,
        end_exclusive,
    };
    let byte_range = sector_range.byte_range();
    let file_bytes = file_bytes as u64;
    if byte_range.end > file_bytes {
        return Err(RegionError::LocationOutOfBounds {
            index,
            start_byte: byte_range.start,
            end_byte: byte_range.end,
            file_bytes,
        });
    }

    Ok(Some(ChunkLocation { sector_range }))
}

fn validate_no_overlaps(
    locations: &[Option<ChunkLocation>; REGION_LOCATION_COUNT],
) -> Result<(), RegionError> {
    let mut allocated = locations
        .iter()
        .enumerate()
        .filter_map(|(index, location)| location.map(|location| (index, location.sector_range)))
        .collect::<Vec<_>>();
    allocated.sort_unstable_by_key(|(index, range)| (range.start, range.end_exclusive, *index));

    for pair in allocated.windows(2) {
        let (first_index, first) = pair[0];
        let (second_index, second) = pair[1];
        if second.start < first.end_exclusive {
            return Err(RegionError::OverlappingLocations {
                first_index,
                second_index,
                overlap_start: second.start,
                overlap_end: first.end_exclusive.min(second.end_exclusive),
            });
        }
    }

    Ok(())
}

fn chunk_index(local_x: u8, local_z: u8) -> Option<usize> {
    if local_x >= 32 || local_z >= 32 {
        return None;
    }
    Some(usize::from(local_z) * 32 + usize::from(local_x))
}

#[cfg(test)]
mod tests {
    use super::{RegionError, RegionHeader, SectorRange, REGION_HEADER_BYTES, REGION_SECTOR_BYTES};

    #[test]
    fn rejects_truncated_fixture() {
        let fixture = include_bytes!("../../../fixtures/world/corrupt/truncated-region.bin");
        assert_eq!(
            RegionHeader::parse(fixture),
            Err(RegionError::TruncatedHeader {
                actual_bytes: fixture.len(),
                required_bytes: REGION_HEADER_BYTES,
            })
        );
    }

    #[test]
    fn parses_locations_timestamps_and_stable_summary() {
        let mut region = vec![0; REGION_SECTOR_BYTES * 5];
        set_location(&mut region, 0, 2, 1);
        set_location(&mut region, 33, 3, 2);
        set_timestamp(&mut region, 0, 200);
        set_timestamp(&mut region, 33, 100);

        let header = RegionHeader::parse(&region).expect("synthetic header should parse");
        assert_eq!(
            header.location(0, 0).map(|location| location.sector_range),
            Some(SectorRange {
                start: 2,
                end_exclusive: 3,
            })
        );
        assert_eq!(header.timestamp(1, 1), Some(100));
        assert_eq!(header.location(32, 0), None);

        let summary = header.summary();
        assert_eq!(summary.present_chunk_count, 2);
        assert_eq!(summary.allocated_sector_count, 3);
        assert_eq!(summary.timestamp_min, Some(100));
        assert_eq!(summary.timestamp_max, Some(200));
        assert_eq!(summary.chunks[0].index, 0);
        assert_eq!(summary.chunks[1].index, 33);
        assert_eq!(summary, header.summary());
    }

    #[test]
    fn rejects_inconsistent_empty_location() {
        let mut region = vec![0; REGION_HEADER_BYTES];
        set_location(&mut region, 7, 2, 0);
        assert_eq!(
            RegionHeader::parse(&region),
            Err(RegionError::InconsistentEmptyLocation {
                index: 7,
                sector_offset: 2,
                sector_count: 0,
            })
        );
    }

    #[test]
    fn rejects_location_inside_header() {
        let mut region = vec![0; REGION_HEADER_BYTES];
        set_location(&mut region, 3, 1, 1);
        assert_eq!(
            RegionHeader::parse(&region),
            Err(RegionError::LocationPointsIntoHeader {
                index: 3,
                sector_offset: 1,
            })
        );
    }

    #[test]
    fn rejects_location_past_file_end() {
        let mut region = vec![0; REGION_SECTOR_BYTES * 3];
        set_location(&mut region, 5, 2, 2);
        assert_eq!(
            RegionHeader::parse(&region),
            Err(RegionError::LocationOutOfBounds {
                index: 5,
                start_byte: (REGION_SECTOR_BYTES * 2) as u64,
                end_byte: (REGION_SECTOR_BYTES * 4) as u64,
                file_bytes: (REGION_SECTOR_BYTES * 3) as u64,
            })
        );
    }

    #[test]
    fn rejects_overlapping_chunk_sectors() {
        let mut region = vec![0; REGION_SECTOR_BYTES * 6];
        set_location(&mut region, 8, 2, 3);
        set_location(&mut region, 9, 4, 2);
        assert_eq!(
            RegionHeader::parse(&region),
            Err(RegionError::OverlappingLocations {
                first_index: 8,
                second_index: 9,
                overlap_start: 4,
                overlap_end: 5,
            })
        );
    }

    fn set_location(region: &mut [u8], index: usize, sector_offset: u32, sector_count: u8) {
        let raw = (sector_offset << 8) | u32::from(sector_count);
        let entry_offset = index * 4;
        region[entry_offset..entry_offset + 4].copy_from_slice(&raw.to_be_bytes());
    }

    fn set_timestamp(region: &mut [u8], index: usize, timestamp: u32) {
        let entry_offset = REGION_SECTOR_BYTES + index * 4;
        region[entry_offset..entry_offset + 4].copy_from_slice(&timestamp.to_be_bytes());
    }
}
