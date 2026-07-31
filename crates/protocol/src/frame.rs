use crate::{decode_varint, encode_varint, try_decode_varint, ProtocolError, MAX_PACKET_BYTES};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PacketFrame {
    pub packet_id: i32,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecodeStatus<T> {
    Incomplete,
    Complete { value: T, consumed: usize },
}

pub fn decode_frame(input: &[u8]) -> Result<DecodeStatus<PacketFrame>, ProtocolError> {
    let Some(length) = try_decode_varint(input)? else {
        return Ok(DecodeStatus::Incomplete);
    };
    if length.value < 0 {
        return Err(ProtocolError::NegativePacketLength(length.value));
    }

    let packet_length = length.value as usize;
    if packet_length > MAX_PACKET_BYTES {
        return Err(ProtocolError::PacketTooLarge {
            actual: packet_length,
        });
    }
    if packet_length == 0 {
        return Err(ProtocolError::EmptyPacket);
    }

    let total_length =
        length
            .consumed
            .checked_add(packet_length)
            .ok_or(ProtocolError::PacketTooLarge {
                actual: packet_length,
            })?;
    if input.len() < total_length {
        return Ok(DecodeStatus::Incomplete);
    }

    Ok(DecodeStatus::Complete {
        value: decode_packet_body(&input[length.consumed..total_length])?,
        consumed: total_length,
    })
}

pub fn encode_frame(frame: &PacketFrame) -> Result<Vec<u8>, ProtocolError> {
    let body = encode_packet_body(frame)?;

    let mut encoded = Vec::with_capacity(body.len().saturating_add(5));
    encode_varint(body.len() as i32, &mut encoded);
    encoded.extend_from_slice(&body);
    Ok(encoded)
}

pub(crate) fn decode_packet_body(body: &[u8]) -> Result<PacketFrame, ProtocolError> {
    let packet_id = decode_varint(body)?;
    if packet_id.value < 0 {
        return Err(ProtocolError::NegativePacketId(packet_id.value));
    }
    Ok(PacketFrame {
        packet_id: packet_id.value,
        payload: body[packet_id.consumed..].to_vec(),
    })
}

pub(crate) fn encode_packet_body(frame: &PacketFrame) -> Result<Vec<u8>, ProtocolError> {
    if frame.packet_id < 0 {
        return Err(ProtocolError::NegativePacketId(frame.packet_id));
    }

    let mut body = Vec::with_capacity(frame.payload.len().saturating_add(5));
    encode_varint(frame.packet_id, &mut body);
    body.extend_from_slice(&frame.payload);
    if body.len() > MAX_PACKET_BYTES {
        return Err(ProtocolError::PacketTooLarge { actual: body.len() });
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::{decode_frame, encode_frame, DecodeStatus, PacketFrame};
    use crate::{encode_varint, ProtocolError, MAX_PACKET_BYTES};

    #[test]
    fn round_trips_frame_and_reports_consumed_bytes() {
        let frame = PacketFrame {
            packet_id: 0x2a,
            payload: vec![1, 2, 3, 4],
        };
        let mut encoded = encode_frame(&frame).unwrap_or_default();
        encoded.extend_from_slice(&[0xaa, 0xbb]);

        assert_eq!(
            decode_frame(&encoded),
            Ok(DecodeStatus::Complete {
                value: frame,
                consumed: encoded.len() - 2,
            })
        );
    }

    #[test]
    fn partial_frame_waits_for_more_bytes() {
        let encoded = encode_frame(&PacketFrame {
            packet_id: 1,
            payload: vec![10, 20, 30],
        })
        .unwrap_or_default();

        assert_eq!(
            decode_frame(&encoded[..encoded.len() - 1]),
            Ok(DecodeStatus::Incomplete)
        );
    }

    #[test]
    fn oversized_length_is_rejected_before_payload_arrives() {
        let mut encoded = Vec::new();
        encode_varint((MAX_PACKET_BYTES + 1) as i32, &mut encoded);

        assert_eq!(
            decode_frame(&encoded),
            Err(ProtocolError::PacketTooLarge {
                actual: MAX_PACKET_BYTES + 1,
            })
        );
    }

    #[test]
    fn empty_packet_is_rejected() {
        assert_eq!(decode_frame(&[0]), Err(ProtocolError::EmptyPacket));
    }

    #[test]
    fn negative_packet_id_is_rejected() {
        let mut body = Vec::new();
        encode_varint(-1, &mut body);
        let mut encoded = Vec::new();
        encode_varint(body.len() as i32, &mut encoded);
        encoded.extend_from_slice(&body);

        assert_eq!(
            decode_frame(&encoded),
            Err(ProtocolError::NegativePacketId(-1))
        );
    }
}
