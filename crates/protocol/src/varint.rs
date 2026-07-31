use crate::{ProtocolError, MAX_VARINT_BYTES};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodedVarInt {
    pub value: i32,
    pub consumed: usize,
}

pub fn decode_varint(input: &[u8]) -> Result<DecodedVarInt, ProtocolError> {
    try_decode_varint(input)?.ok_or(ProtocolError::TruncatedVarInt)
}

pub fn encode_varint(value: i32, output: &mut Vec<u8>) {
    let mut remaining = value as u32;
    loop {
        if remaining & !0x7f == 0 {
            output.push(remaining as u8);
            return;
        }
        output.push(((remaining & 0x7f) | 0x80) as u8);
        remaining >>= 7;
    }
}

pub(crate) fn try_decode_varint(input: &[u8]) -> Result<Option<DecodedVarInt>, ProtocolError> {
    let mut value = 0_u32;

    for index in 0..MAX_VARINT_BYTES {
        let Some(&byte) = input.get(index) else {
            return Ok(None);
        };
        if index == MAX_VARINT_BYTES - 1 && byte & 0xf0 != 0 {
            return Err(ProtocolError::VarIntTooLong);
        }

        value |= u32::from(byte & 0x7f) << (index * 7);
        if byte & 0x80 == 0 {
            let decoded = value as i32;
            let consumed = index + 1;
            if encoded_len(decoded) != consumed {
                return Err(ProtocolError::NonCanonicalVarInt);
            }
            return Ok(Some(DecodedVarInt {
                value: decoded,
                consumed,
            }));
        }
    }

    Err(ProtocolError::VarIntTooLong)
}

fn encoded_len(value: i32) -> usize {
    let mut remaining = value as u32;
    let mut length = 1;
    while remaining & !0x7f != 0 {
        length += 1;
        remaining >>= 7;
    }
    length
}

#[cfg(test)]
mod tests {
    use super::{decode_varint, encode_varint};
    use crate::ProtocolError;

    #[test]
    fn round_trips_i32_values() {
        for value in [0, 1, 127, 128, 255, 2_097_151, i32::MAX, -1, i32::MIN] {
            let mut encoded = Vec::new();
            encode_varint(value, &mut encoded);
            let decoded = decode_varint(&encoded);

            assert_eq!(
                decoded.map(|item| (item.value, item.consumed)),
                Ok((value, encoded.len()))
            );
        }
    }

    #[test]
    fn rejects_truncated_fixture() {
        let fixture = include_str!("../../../fixtures/protocol/truncated-varint.hex");
        let bytes = hex::decode(fixture.trim());

        assert_eq!(
            bytes.as_deref().map(decode_varint),
            Ok(Err(ProtocolError::TruncatedVarInt))
        );
    }

    #[test]
    fn rejects_too_long_varint() {
        assert_eq!(
            decode_varint(&[0xff, 0xff, 0xff, 0xff, 0x80]),
            Err(ProtocolError::VarIntTooLong)
        );
    }

    #[test]
    fn rejects_non_canonical_varint() {
        assert_eq!(
            decode_varint(&[0x80, 0x00]),
            Err(ProtocolError::NonCanonicalVarInt)
        );
    }
}
