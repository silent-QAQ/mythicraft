use crate::{
    decode_compressed_frame, CompressionMode, DecodeStatus, PacketFrame, ProtocolError,
    MAX_STREAM_BUFFER_BYTES,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameDecoder {
    buffer: Vec<u8>,
    compression: CompressionMode,
}

impl FrameDecoder {
    pub const fn new(compression: CompressionMode) -> Self {
        Self {
            buffer: Vec::new(),
            compression,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<(), ProtocolError> {
        let new_length = self.buffer.len().checked_add(bytes.len()).ok_or(
            ProtocolError::StreamBufferTooLarge {
                actual: usize::MAX,
                maximum: MAX_STREAM_BUFFER_BYTES,
            },
        )?;
        if new_length > MAX_STREAM_BUFFER_BYTES {
            return Err(ProtocolError::StreamBufferTooLarge {
                actual: new_length,
                maximum: MAX_STREAM_BUFFER_BYTES,
            });
        }
        self.buffer.extend_from_slice(bytes);
        Ok(())
    }

    pub fn next_frame(&mut self) -> Result<Option<PacketFrame>, ProtocolError> {
        match decode_compressed_frame(&self.buffer, self.compression)? {
            DecodeStatus::Incomplete => Ok(None),
            DecodeStatus::Complete { value, consumed } => {
                self.buffer.drain(..consumed);
                Ok(Some(value))
            }
        }
    }

    pub fn set_compression(&mut self, compression: CompressionMode) -> Result<(), ProtocolError> {
        if !self.buffer.is_empty() {
            return Err(ProtocolError::CompressionModeChangeWithBufferedData {
                buffered: self.buffer.len(),
            });
        }
        self.compression = compression;
        Ok(())
    }

    pub fn buffered_len(&self) -> usize {
        self.buffer.len()
    }
}

#[cfg(test)]
mod tests {
    use super::FrameDecoder;
    use crate::{encode_frame, CompressionMode, PacketFrame, ProtocolError};

    #[test]
    fn reconstructs_fragmented_frame() {
        let frame = PacketFrame {
            packet_id: 7,
            payload: vec![1, 2, 3, 4],
        };
        let encoded = encode_frame(&frame).unwrap_or_default();
        let mut decoder = FrameDecoder::new(CompressionMode::Disabled);

        for byte in &encoded[..encoded.len() - 1] {
            assert_eq!(decoder.push(&[*byte]), Ok(()));
            assert_eq!(decoder.next_frame(), Ok(None));
        }
        assert_eq!(decoder.push(&encoded[encoded.len() - 1..]), Ok(()));
        assert_eq!(decoder.next_frame(), Ok(Some(frame)));
        assert_eq!(decoder.buffered_len(), 0);
    }

    #[test]
    fn decodes_coalesced_frames_in_order() {
        let first = PacketFrame {
            packet_id: 1,
            payload: vec![10],
        };
        let second = PacketFrame {
            packet_id: 2,
            payload: vec![20],
        };
        let mut encoded = encode_frame(&first).unwrap_or_default();
        encoded.extend_from_slice(&encode_frame(&second).unwrap_or_default());
        let mut decoder = FrameDecoder::new(CompressionMode::Disabled);

        assert_eq!(decoder.push(&encoded), Ok(()));
        assert_eq!(decoder.next_frame(), Ok(Some(first)));
        assert_eq!(decoder.next_frame(), Ok(Some(second)));
        assert_eq!(decoder.next_frame(), Ok(None));
    }

    #[test]
    fn compression_mode_change_requires_empty_buffer() {
        let mut decoder = FrameDecoder::new(CompressionMode::Disabled);
        assert_eq!(decoder.push(&[0x80]), Ok(()));

        assert_eq!(
            decoder.set_compression(CompressionMode::Enabled { threshold: 256 }),
            Err(ProtocolError::CompressionModeChangeWithBufferedData { buffered: 1 })
        );
    }
}
