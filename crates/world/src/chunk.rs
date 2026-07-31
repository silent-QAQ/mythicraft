use flate2::{Decompress, FlushDecompress, Status};
use thiserror::Error;

use crate::{
    compression::decode_gzip_limited, compression::GzipDecodeError, RegionError, RegionHeader,
};

pub const DEFAULT_MAX_CHUNK_COMPRESSED_BYTES: usize = 1024 * 1024;
pub const DEFAULT_MAX_CHUNK_DECOMPRESSED_BYTES: usize = mythicraft_nbt::DEFAULT_MAX_NBT_BYTES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChunkReadLimits {
    pub max_compressed_bytes: usize,
    pub max_decompressed_bytes: usize,
}

impl Default for ChunkReadLimits {
    fn default() -> Self {
        Self {
            max_compressed_bytes: DEFAULT_MAX_CHUNK_COMPRESSED_BYTES,
            max_decompressed_bytes: DEFAULT_MAX_CHUNK_DECOMPRESSED_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChunkCompression {
    Gzip,
    Zlib,
    Uncompressed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChunkData<'a> {
    pub compression: ChunkCompression,
    pub compressed_payload: &'a [u8],
    pub declared_length: u32,
}

impl ChunkData<'_> {
    pub fn decompress(&self, limits: ChunkReadLimits) -> Result<Vec<u8>, ChunkError> {
        if self.compressed_payload.len() > limits.max_compressed_bytes {
            return Err(ChunkError::CompressedPayloadTooLarge {
                actual_bytes: self.compressed_payload.len(),
                max_bytes: limits.max_compressed_bytes,
            });
        }

        match self.compression {
            ChunkCompression::Gzip => {
                decode_gzip_limited(self.compressed_payload, limits.max_decompressed_bytes)
                    .map_err(map_gzip_error)
            }
            ChunkCompression::Zlib => {
                decode_zlib_limited(self.compressed_payload, limits.max_decompressed_bytes)
            }
            ChunkCompression::Uncompressed => {
                if self.compressed_payload.len() > limits.max_decompressed_bytes {
                    return Err(ChunkError::DecompressedPayloadTooLarge {
                        max_bytes: limits.max_decompressed_bytes,
                    });
                }
                Ok(self.compressed_payload.to_vec())
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct RegionFile<'a> {
    bytes: &'a [u8],
    header: RegionHeader,
}

impl<'a> RegionFile<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, RegionError> {
        let header = RegionHeader::parse(bytes)?;
        Ok(Self { bytes, header })
    }

    pub fn header(&self) -> &RegionHeader {
        &self.header
    }

    pub fn chunk(&self, local_x: u8, local_z: u8) -> Result<Option<ChunkData<'a>>, ChunkError> {
        if local_x >= 32 || local_z >= 32 {
            return Err(ChunkError::InvalidLocalCoordinates { local_x, local_z });
        }
        let Some(location) = self.header.location(local_x, local_z) else {
            return Ok(None);
        };
        let byte_range = location.byte_range();
        let start = usize::try_from(byte_range.start).map_err(|_| ChunkError::OffsetTooLarge {
            byte_offset: byte_range.start,
        })?;
        let end = usize::try_from(byte_range.end).map_err(|_| ChunkError::OffsetTooLarge {
            byte_offset: byte_range.end,
        })?;
        let allocated = &self.bytes[start..end];
        if allocated.len() < 5 {
            return Err(ChunkError::TruncatedEnvelope {
                actual_bytes: allocated.len(),
                required_bytes: 5,
            });
        }

        let declared_length =
            u32::from_be_bytes([allocated[0], allocated[1], allocated[2], allocated[3]]);
        if declared_length == 0 {
            return Err(ChunkError::ZeroDeclaredLength);
        }
        let serialized_bytes = usize::try_from(declared_length)
            .ok()
            .and_then(|length| length.checked_add(4))
            .ok_or(ChunkError::DeclaredLengthOverflow { declared_length })?;
        if serialized_bytes > allocated.len() {
            return Err(ChunkError::DeclaredLengthExceedsAllocation {
                declared_length,
                allocated_bytes: allocated.len(),
            });
        }

        let compression_byte = allocated[4];
        if compression_byte & 0x80 != 0 {
            return Err(ChunkError::ExternalChunkUnsupported {
                compression_id: compression_byte & 0x7f,
            });
        }
        let compression = match compression_byte {
            1 => ChunkCompression::Gzip,
            2 => ChunkCompression::Zlib,
            3 => ChunkCompression::Uncompressed,
            4 => return Err(ChunkError::UnsupportedCompression { compression_id: 4 }),
            127 => {
                return Err(ChunkError::UnsupportedCompression {
                    compression_id: 127,
                })
            }
            compression_id => {
                return Err(ChunkError::UnknownCompression { compression_id });
            }
        };
        let payload_end = serialized_bytes;
        let compressed_payload = &allocated[5..payload_end];

        Ok(Some(ChunkData {
            compression,
            compressed_payload,
            declared_length,
        }))
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ChunkError {
    #[error("local chunk coordinates are outside 0..32: ({local_x}, {local_z})")]
    InvalidLocalCoordinates { local_x: u8, local_z: u8 },
    #[error("chunk byte offset {byte_offset} cannot be represented on this platform")]
    OffsetTooLarge { byte_offset: u64 },
    #[error("chunk envelope is truncated: got {actual_bytes} bytes, need {required_bytes}")]
    TruncatedEnvelope {
        actual_bytes: usize,
        required_bytes: usize,
    },
    #[error("chunk declared length must include at least the compression byte")]
    ZeroDeclaredLength,
    #[error("chunk declared length {declared_length} overflows platform limits")]
    DeclaredLengthOverflow { declared_length: u32 },
    #[error(
        "chunk declared length {declared_length} exceeds its {allocated_bytes}-byte sector allocation"
    )]
    DeclaredLengthExceedsAllocation {
        declared_length: u32,
        allocated_bytes: usize,
    },
    #[error("external chunk stream with compression id {compression_id} is not supported")]
    ExternalChunkUnsupported { compression_id: u8 },
    #[error("chunk compression id {compression_id} is recognized but not supported")]
    UnsupportedCompression { compression_id: u8 },
    #[error("unknown chunk compression id {compression_id}")]
    UnknownCompression { compression_id: u8 },
    #[error("compressed chunk payload has {actual_bytes} bytes, limit is {max_bytes}")]
    CompressedPayloadTooLarge {
        actual_bytes: usize,
        max_bytes: usize,
    },
    #[error("failed to decode {compression:?} chunk data: {message}")]
    DecompressionFailed {
        compression: ChunkCompression,
        message: String,
    },
    #[error("{compression:?} chunk stream has {trailing_bytes} trailing compressed bytes")]
    TrailingCompressedData {
        compression: ChunkCompression,
        trailing_bytes: usize,
    },
    #[error("decompressed chunk payload exceeds {max_bytes} bytes")]
    DecompressedPayloadTooLarge { max_bytes: usize },
}

fn map_gzip_error(error: GzipDecodeError) -> ChunkError {
    match error {
        GzipDecodeError::Decode(message) => ChunkError::DecompressionFailed {
            compression: ChunkCompression::Gzip,
            message,
        },
        GzipDecodeError::TooLarge { max_bytes } => {
            ChunkError::DecompressedPayloadTooLarge { max_bytes }
        }
        GzipDecodeError::TrailingData { trailing_bytes } => ChunkError::TrailingCompressedData {
            compression: ChunkCompression::Gzip,
            trailing_bytes,
        },
    }
}

fn decode_zlib_limited(payload: &[u8], max_bytes: usize) -> Result<Vec<u8>, ChunkError> {
    let mut decoder = Decompress::new(true);
    let mut remaining = payload;
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];

    loop {
        let input_before = decoder.total_in();
        let output_before = decoder.total_out();
        let status = decoder
            .decompress(remaining, &mut buffer, FlushDecompress::Finish)
            .map_err(|error| ChunkError::DecompressionFailed {
                compression: ChunkCompression::Zlib,
                message: error.to_string(),
            })?;
        let consumed = usize::try_from(decoder.total_in() - input_before).map_err(|_| {
            ChunkError::DecompressionFailed {
                compression: ChunkCompression::Zlib,
                message: "compressed input counter exceeds platform limits".into(),
            }
        })?;
        let produced = usize::try_from(decoder.total_out() - output_before).map_err(|_| {
            ChunkError::DecompressionFailed {
                compression: ChunkCompression::Zlib,
                message: "decompressed output counter exceeds platform limits".into(),
            }
        })?;
        output.extend_from_slice(&buffer[..produced]);
        if output.len() > max_bytes {
            return Err(ChunkError::DecompressedPayloadTooLarge { max_bytes });
        }
        remaining = &remaining[consumed..];

        if status == Status::StreamEnd {
            if !remaining.is_empty() {
                return Err(ChunkError::TrailingCompressedData {
                    compression: ChunkCompression::Zlib,
                    trailing_bytes: remaining.len(),
                });
            }
            return Ok(output);
        }
        if consumed == 0 && produced == 0 {
            return Err(ChunkError::DecompressionFailed {
                compression: ChunkCompression::Zlib,
                message: "compressed stream ended before StreamEnd".into(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::{write::GzEncoder, write::ZlibEncoder, Compression};

    use super::{
        ChunkCompression, ChunkError, ChunkReadLimits, RegionFile,
        DEFAULT_MAX_CHUNK_COMPRESSED_BYTES,
    };
    use crate::REGION_SECTOR_BYTES;

    #[test]
    fn reads_and_decompresses_zlib_chunk() {
        let raw = b"synthetic chunk nbt";
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(raw).expect("encode synthetic chunk");
        let compressed = encoder.finish().expect("finish synthetic chunk");
        let region = region_with_chunk(2, &compressed);

        let file = RegionFile::parse(&region).expect("parse synthetic region");
        let chunk = file
            .chunk(0, 0)
            .expect("read synthetic chunk")
            .expect("chunk is present");
        assert_eq!(chunk.compression, ChunkCompression::Zlib);
        assert_eq!(
            chunk
                .decompress(ChunkReadLimits::default())
                .expect("decompress synthetic chunk"),
            raw
        );
        assert!(file.chunk(1, 0).expect("read absent location").is_none());
    }

    #[test]
    fn reads_and_decompresses_gzip_chunk() {
        let raw = b"synthetic gzip chunk nbt";
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(raw).expect("encode synthetic chunk");
        let compressed = encoder.finish().expect("finish synthetic chunk");
        let region = region_with_chunk(1, &compressed);

        let file = RegionFile::parse(&region).expect("parse synthetic region");
        let chunk = file
            .chunk(0, 0)
            .expect("read synthetic chunk")
            .expect("chunk is present");
        assert_eq!(chunk.compression, ChunkCompression::Gzip);
        assert_eq!(
            chunk
                .decompress(ChunkReadLimits::default())
                .expect("decompress synthetic chunk"),
            raw
        );
    }

    #[test]
    fn reads_uncompressed_chunk_and_enforces_output_limit() {
        let region = region_with_chunk(3, b"12345");
        let file = RegionFile::parse(&region).expect("parse synthetic region");
        let chunk = file
            .chunk(0, 0)
            .expect("read synthetic chunk")
            .expect("chunk is present");
        let limits = ChunkReadLimits {
            max_compressed_bytes: DEFAULT_MAX_CHUNK_COMPRESSED_BYTES,
            max_decompressed_bytes: 4,
        };
        assert_eq!(
            chunk.decompress(limits),
            Err(ChunkError::DecompressedPayloadTooLarge { max_bytes: 4 })
        );
    }

    #[test]
    fn rejects_zero_and_oversized_declared_lengths() {
        let mut zero = region_with_chunk(3, b"");
        zero[REGION_SECTOR_BYTES * 2..REGION_SECTOR_BYTES * 2 + 4]
            .copy_from_slice(&0_u32.to_be_bytes());
        let zero_file = RegionFile::parse(&zero).expect("parse zero-length region header");
        assert_eq!(zero_file.chunk(0, 0), Err(ChunkError::ZeroDeclaredLength));

        let mut oversized = region_with_chunk(3, b"data");
        oversized[REGION_SECTOR_BYTES * 2..REGION_SECTOR_BYTES * 2 + 4]
            .copy_from_slice(&(REGION_SECTOR_BYTES as u32).to_be_bytes());
        let oversized_file =
            RegionFile::parse(&oversized).expect("parse oversized-length region header");
        assert_eq!(
            oversized_file.chunk(0, 0),
            Err(ChunkError::DeclaredLengthExceedsAllocation {
                declared_length: REGION_SECTOR_BYTES as u32,
                allocated_bytes: REGION_SECTOR_BYTES,
            })
        );
    }

    #[test]
    fn rejects_external_unknown_and_unsupported_compression() {
        for (compression_id, expected) in [
            (
                0x82,
                ChunkError::ExternalChunkUnsupported { compression_id: 2 },
            ),
            (4, ChunkError::UnsupportedCompression { compression_id: 4 }),
            (9, ChunkError::UnknownCompression { compression_id: 9 }),
        ] {
            let region = region_with_chunk(compression_id, b"data");
            let file = RegionFile::parse(&region).expect("parse synthetic region");
            assert_eq!(file.chunk(0, 0), Err(expected));
        }
    }

    #[test]
    fn rejects_truncated_compressed_stream() {
        let region = region_with_chunk(2, &[0x78]);
        let file = RegionFile::parse(&region).expect("parse synthetic region");
        let chunk = file
            .chunk(0, 0)
            .expect("read synthetic chunk")
            .expect("chunk is present");
        assert!(matches!(
            chunk.decompress(ChunkReadLimits::default()),
            Err(ChunkError::DecompressionFailed {
                compression: ChunkCompression::Zlib,
                ..
            })
        ));
    }

    #[test]
    fn rejects_trailing_compressed_data_and_compressed_input_over_limit() {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(b"chunk").expect("encode synthetic chunk");
        let mut compressed = encoder.finish().expect("finish synthetic chunk");
        compressed.extend_from_slice(b"trailing");
        let region = region_with_chunk(2, &compressed);
        let file = RegionFile::parse(&region).expect("parse synthetic region");
        let chunk = file
            .chunk(0, 0)
            .expect("read synthetic chunk")
            .expect("chunk is present");
        assert_eq!(
            chunk.decompress(ChunkReadLimits::default()),
            Err(ChunkError::TrailingCompressedData {
                compression: ChunkCompression::Zlib,
                trailing_bytes: 8,
            })
        );

        assert_eq!(
            chunk.decompress(ChunkReadLimits {
                max_compressed_bytes: compressed.len() - 1,
                max_decompressed_bytes: 1024,
            }),
            Err(ChunkError::CompressedPayloadTooLarge {
                actual_bytes: compressed.len(),
                max_bytes: compressed.len() - 1,
            })
        );
    }

    fn region_with_chunk(compression_id: u8, payload: &[u8]) -> Vec<u8> {
        let mut region = vec![0; REGION_SECTOR_BYTES * 3];
        region[0..4].copy_from_slice(&((2_u32 << 8) | 1).to_be_bytes());
        let chunk_start = REGION_SECTOR_BYTES * 2;
        let declared_length = u32::try_from(payload.len() + 1).expect("synthetic payload length");
        region[chunk_start..chunk_start + 4].copy_from_slice(&declared_length.to_be_bytes());
        region[chunk_start + 4] = compression_id;
        region[chunk_start + 5..chunk_start + 5 + payload.len()].copy_from_slice(payload);
        region
    }
}
