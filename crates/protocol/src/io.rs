use crate::{decode_varint, encode_varint, ProtocolError};
use uuid::Uuid;

pub(crate) struct PacketCursor<'a> {
    input: &'a [u8],
    position: usize,
}

impl<'a> PacketCursor<'a> {
    pub(crate) const fn new(input: &'a [u8]) -> Self {
        Self { input, position: 0 }
    }

    pub(crate) fn read_varint(&mut self) -> Result<i32, ProtocolError> {
        let decoded = decode_varint(&self.input[self.position..])?;
        self.position += decoded.consumed;
        Ok(decoded.value)
    }

    pub(crate) fn read_u16(&mut self, field: &'static str) -> Result<u16, ProtocolError> {
        let bytes = self.read_exact(2, field)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    pub(crate) fn read_i64(&mut self, field: &'static str) -> Result<i64, ProtocolError> {
        let bytes = self.read_exact(8, field)?;
        Ok(i64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    pub(crate) fn read_bool(&mut self, field: &'static str) -> Result<bool, ProtocolError> {
        match self.read_exact(1, field)?[0] {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(ProtocolError::InvalidBoolean(value)),
        }
    }

    pub(crate) fn read_uuid(&mut self, field: &'static str) -> Result<Uuid, ProtocolError> {
        let bytes = self.read_exact(16, field)?;
        let mut uuid_bytes = [0_u8; 16];
        uuid_bytes.copy_from_slice(bytes);
        Ok(Uuid::from_bytes(uuid_bytes))
    }

    pub(crate) fn read_byte_array(
        &mut self,
        maximum: usize,
        field: &'static str,
    ) -> Result<Vec<u8>, ProtocolError> {
        let length = self.read_varint()?;
        if length < 0 {
            return Err(ProtocolError::NegativeByteArrayLength(length));
        }
        let length = length as usize;
        if length > maximum {
            return Err(ProtocolError::ByteArrayTooLong {
                actual: length,
                maximum,
            });
        }
        Ok(self.read_exact(length, field)?.to_vec())
    }

    pub(crate) fn read_string(&mut self, maximum: usize) -> Result<String, ProtocolError> {
        let length = self.read_varint()?;
        if length < 0 {
            return Err(ProtocolError::NegativeStringLength(length));
        }
        let length = length as usize;
        if length > maximum {
            return Err(ProtocolError::StringTooLong {
                actual: length,
                maximum,
            });
        }
        let bytes = self.read_exact(length, "string")?;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| ProtocolError::InvalidStringUtf8)
    }

    pub(crate) fn finish(self) -> Result<(), ProtocolError> {
        let remaining = self.input.len().saturating_sub(self.position);
        if remaining != 0 {
            return Err(ProtocolError::TrailingPacketData { remaining });
        }
        Ok(())
    }

    pub(crate) fn read_exact(
        &mut self,
        length: usize,
        field: &'static str,
    ) -> Result<&'a [u8], ProtocolError> {
        let remaining = self.input.len().saturating_sub(self.position);
        if remaining < length {
            return Err(ProtocolError::TruncatedField {
                field,
                needed: length,
                remaining,
            });
        }
        let start = self.position;
        self.position += length;
        Ok(&self.input[start..self.position])
    }
}

pub(crate) fn write_string(
    value: &str,
    maximum: usize,
    output: &mut Vec<u8>,
) -> Result<(), ProtocolError> {
    if value.len() > maximum {
        return Err(ProtocolError::StringTooLong {
            actual: value.len(),
            maximum,
        });
    }
    encode_varint(value.len() as i32, output);
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

pub(crate) fn write_byte_array(
    value: &[u8],
    maximum: usize,
    output: &mut Vec<u8>,
) -> Result<(), ProtocolError> {
    if value.len() > maximum {
        return Err(ProtocolError::ByteArrayTooLong {
            actual: value.len(),
            maximum,
        });
    }
    encode_varint(value.len() as i32, output);
    output.extend_from_slice(value);
    Ok(())
}

pub(crate) fn write_bool(value: bool, output: &mut Vec<u8>) {
    output.push(u8::from(value));
}

pub(crate) fn write_uuid(value: Uuid, output: &mut Vec<u8>) {
    output.extend_from_slice(value.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::{write_bool, write_byte_array, write_string, write_uuid, PacketCursor};
    use crate::ProtocolError;
    use uuid::Uuid;

    #[test]
    fn string_round_trip_supports_utf8() {
        let mut encoded = Vec::new();
        assert_eq!(write_string("Mythic 世界", 64, &mut encoded), Ok(()));
        let mut cursor = PacketCursor::new(&encoded);

        assert_eq!(cursor.read_string(64), Ok("Mythic 世界".to_owned()));
        assert_eq!(cursor.finish(), Ok(()));
    }

    #[test]
    fn string_limit_is_checked_before_body_read() {
        assert_eq!(
            PacketCursor::new(&[10]).read_string(5),
            Err(ProtocolError::StringTooLong {
                actual: 10,
                maximum: 5,
            })
        );
    }

    #[test]
    fn truncated_fixed_width_field_is_rejected() {
        assert_eq!(
            PacketCursor::new(&[0, 1, 2]).read_i64("ping payload"),
            Err(ProtocolError::TruncatedField {
                field: "ping payload",
                needed: 8,
                remaining: 3,
            })
        );
    }

    #[test]
    fn uuid_bool_and_byte_array_round_trip() {
        let uuid = Uuid::from_u128(0x1234_5678_90ab_cdef_0123_4567_89ab_cdef);
        let mut encoded = Vec::new();
        write_uuid(uuid, &mut encoded);
        write_bool(true, &mut encoded);
        assert_eq!(write_byte_array(&[1, 2, 3], 8, &mut encoded), Ok(()));
        let mut cursor = PacketCursor::new(&encoded);

        assert_eq!(cursor.read_uuid("uuid"), Ok(uuid));
        assert_eq!(cursor.read_bool("flag"), Ok(true));
        assert_eq!(cursor.read_byte_array(8, "bytes"), Ok(vec![1, 2, 3]));
        assert_eq!(cursor.finish(), Ok(()));
    }
}
