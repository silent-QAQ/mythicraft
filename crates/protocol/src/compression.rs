use std::io::{Read, Write};

use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};

use crate::frame::{decode_packet_body, encode_packet_body};
use crate::{
    encode_varint, try_decode_varint, DecodeStatus, PacketFrame, ProtocolError, MAX_PACKET_BYTES,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompressionMode {
    Disabled,
    Enabled { threshold: usize },
}

pub fn decode_compressed_frame(
    input: &[u8],
    mode: CompressionMode,
) -> Result<DecodeStatus<PacketFrame>, ProtocolError> {
    if mode == CompressionMode::Disabled {
        return crate::decode_frame(input);
    }
    let CompressionMode::Enabled { threshold } = mode else {
        return crate::decode_frame(input);
    };
    validate_threshold(threshold)?;

    let Some(packet_length) = try_decode_varint(input)? else {
        return Ok(DecodeStatus::Incomplete);
    };
    if packet_length.value < 0 {
        return Err(ProtocolError::NegativePacketLength(packet_length.value));
    }
    let packet_length_value = packet_length.value as usize;
    if packet_length_value > MAX_PACKET_BYTES {
        return Err(ProtocolError::PacketTooLarge {
            actual: packet_length_value,
        });
    }

    let total_length = packet_length
        .consumed
        .checked_add(packet_length_value)
        .ok_or(ProtocolError::PacketTooLarge {
            actual: packet_length_value,
        })?;
    if input.len() < total_length {
        return Ok(DecodeStatus::Incomplete);
    }

    let compressed_body = &input[packet_length.consumed..total_length];
    let uncompressed_length = crate::decode_varint(compressed_body)?;
    if uncompressed_length.value < 0 {
        return Err(ProtocolError::NegativeUncompressedLength(
            uncompressed_length.value,
        ));
    }
    let packet_data = &compressed_body[uncompressed_length.consumed..];

    let body = if uncompressed_length.value == 0 {
        if packet_data.len() >= threshold {
            return Err(ProtocolError::UncompressedAboveThreshold {
                actual: packet_data.len(),
                threshold,
            });
        }
        packet_data.to_vec()
    } else {
        let declared = uncompressed_length.value as usize;
        if declared > MAX_PACKET_BYTES {
            return Err(ProtocolError::PacketTooLarge { actual: declared });
        }
        if declared < threshold {
            return Err(ProtocolError::CompressedBelowThreshold {
                declared,
                threshold,
            });
        }
        decompress_exact(packet_data, declared)?
    };

    Ok(DecodeStatus::Complete {
        value: decode_packet_body(&body)?,
        consumed: total_length,
    })
}

pub fn encode_compressed_frame(
    frame: &PacketFrame,
    mode: CompressionMode,
) -> Result<Vec<u8>, ProtocolError> {
    if mode == CompressionMode::Disabled {
        return crate::encode_frame(frame);
    }
    let CompressionMode::Enabled { threshold } = mode else {
        return crate::encode_frame(frame);
    };
    validate_threshold(threshold)?;

    let packet_body = encode_packet_body(frame)?;
    let mut compression_body = Vec::new();
    if packet_body.len() >= threshold {
        encode_varint(packet_body.len() as i32, &mut compression_body);
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(&packet_body)
            .map_err(|error| ProtocolError::CompressionFailure(error.to_string()))?;
        let compressed = encoder
            .finish()
            .map_err(|error| ProtocolError::CompressionFailure(error.to_string()))?;
        compression_body.extend_from_slice(&compressed);
    } else {
        encode_varint(0, &mut compression_body);
        compression_body.extend_from_slice(&packet_body);
    }
    if compression_body.len() > MAX_PACKET_BYTES {
        return Err(ProtocolError::PacketTooLarge {
            actual: compression_body.len(),
        });
    }

    let mut encoded = Vec::with_capacity(compression_body.len().saturating_add(5));
    encode_varint(compression_body.len() as i32, &mut encoded);
    encoded.extend_from_slice(&compression_body);
    Ok(encoded)
}

fn validate_threshold(threshold: usize) -> Result<(), ProtocolError> {
    if threshold > MAX_PACKET_BYTES {
        return Err(ProtocolError::InvalidCompressionThreshold(threshold));
    }
    Ok(())
}

fn decompress_exact(input: &[u8], declared: usize) -> Result<Vec<u8>, ProtocolError> {
    let read_limit = u64::try_from(declared)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut decoder = ZlibDecoder::new(input);
    let mut output = Vec::with_capacity(declared);
    decoder
        .by_ref()
        .take(read_limit)
        .read_to_end(&mut output)
        .map_err(|error| ProtocolError::CompressionFailure(error.to_string()))?;
    if output.len() != declared {
        return Err(ProtocolError::DecompressedLengthMismatch {
            declared,
            actual: output.len(),
        });
    }
    let consumed = usize::try_from(decoder.total_in()).unwrap_or(usize::MAX);
    if consumed != input.len() {
        return Err(ProtocolError::CompressedTrailingBytes {
            consumed,
            actual: input.len(),
        });
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::{decode_compressed_frame, encode_compressed_frame, CompressionMode};
    use crate::{encode_varint, DecodeStatus, PacketFrame, ProtocolError};

    #[test]
    fn round_trips_compressed_packet() {
        let frame = PacketFrame {
            packet_id: 7,
            payload: vec![42; 512],
        };
        let mode = CompressionMode::Enabled { threshold: 128 };
        let encoded = encode_compressed_frame(&frame, mode).unwrap_or_default();

        assert_eq!(
            decode_compressed_frame(&encoded, mode),
            Ok(DecodeStatus::Complete {
                value: frame,
                consumed: encoded.len(),
            })
        );
    }

    #[test]
    fn round_trips_uncompressed_packet_below_threshold() {
        let frame = PacketFrame {
            packet_id: 7,
            payload: vec![42; 8],
        };
        let mode = CompressionMode::Enabled { threshold: 128 };
        let encoded = encode_compressed_frame(&frame, mode).unwrap_or_default();

        assert_eq!(
            decode_compressed_frame(&encoded, mode),
            Ok(DecodeStatus::Complete {
                value: frame,
                consumed: encoded.len(),
            })
        );
    }

    #[test]
    fn rejects_declared_decompressed_length_mismatch() {
        let frame = PacketFrame {
            packet_id: 7,
            payload: vec![42; 512],
        };
        let mode = CompressionMode::Enabled { threshold: 128 };
        let encoded = encode_compressed_frame(&frame, mode).unwrap_or_default();
        let outer = crate::decode_varint(&encoded).unwrap_or(crate::DecodedVarInt {
            value: 0,
            consumed: 0,
        });
        let body = &encoded[outer.consumed..];
        let data_length = crate::decode_varint(body).unwrap_or(crate::DecodedVarInt {
            value: 0,
            consumed: 0,
        });
        let mut malformed_body = Vec::new();
        encode_varint(data_length.value + 1, &mut malformed_body);
        malformed_body.extend_from_slice(&body[data_length.consumed..]);
        let mut malformed = Vec::new();
        encode_varint(malformed_body.len() as i32, &mut malformed);
        malformed.extend_from_slice(&malformed_body);

        assert!(matches!(
            decode_compressed_frame(&malformed, mode),
            Err(ProtocolError::DecompressedLengthMismatch { .. })
        ));
    }

    #[test]
    fn rejects_uncompressed_packet_at_threshold() {
        let mode = CompressionMode::Enabled { threshold: 2 };
        let mut body = Vec::new();
        encode_varint(0, &mut body);
        body.extend_from_slice(&[1, 2]);
        let mut encoded = Vec::new();
        encode_varint(body.len() as i32, &mut encoded);
        encoded.extend_from_slice(&body);

        assert_eq!(
            decode_compressed_frame(&encoded, mode),
            Err(ProtocolError::UncompressedAboveThreshold {
                actual: 2,
                threshold: 2,
            })
        );
    }

    #[test]
    fn rejects_trailing_compressed_bytes() {
        let frame = PacketFrame {
            packet_id: 7,
            payload: vec![42; 512],
        };
        let mode = CompressionMode::Enabled { threshold: 128 };
        let encoded = encode_compressed_frame(&frame, mode).unwrap_or_default();
        let outer = crate::decode_varint(&encoded).unwrap_or(crate::DecodedVarInt {
            value: 0,
            consumed: 0,
        });
        let mut body = encoded[outer.consumed..].to_vec();
        body.extend_from_slice(&[0xaa, 0xbb]);
        let mut malformed = Vec::new();
        encode_varint(body.len() as i32, &mut malformed);
        malformed.extend_from_slice(&body);

        assert!(matches!(
            decode_compressed_frame(&malformed, mode),
            Err(ProtocolError::CompressedTrailingBytes { .. })
        ));
    }
}
