use crate::io::{write_string, PacketCursor};
use crate::{encode_varint, PacketFrame, ProtocolError, MAX_SERVER_ADDRESS_BYTES};

pub const HANDSHAKE_INTENTION_PACKET_ID: i32 = 0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandshakeNextState {
    Status,
    Login,
}

impl HandshakeNextState {
    const fn wire_value(self) -> i32 {
        match self {
            Self::Status => 1,
            Self::Login => 2,
        }
    }

    fn from_wire(value: i32) -> Result<Self, ProtocolError> {
        match value {
            1 => Ok(Self::Status),
            2 => Ok(Self::Login),
            _ => Err(ProtocolError::InvalidHandshakeNextState(value)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HandshakePacket {
    pub protocol_version: i32,
    pub server_address: String,
    pub server_port: u16,
    pub next_state: HandshakeNextState,
}

pub fn decode_handshake(frame: &PacketFrame) -> Result<HandshakePacket, ProtocolError> {
    require_packet_id(frame, HANDSHAKE_INTENTION_PACKET_ID)?;
    let mut cursor = PacketCursor::new(&frame.payload);
    let protocol_version = cursor.read_varint()?;
    let server_address = cursor.read_string(MAX_SERVER_ADDRESS_BYTES)?;
    let server_port = cursor.read_u16("server port")?;
    let next_state = HandshakeNextState::from_wire(cursor.read_varint()?)?;
    cursor.finish()?;

    Ok(HandshakePacket {
        protocol_version,
        server_address,
        server_port,
        next_state,
    })
}

pub fn encode_handshake(packet: &HandshakePacket) -> Result<PacketFrame, ProtocolError> {
    let mut payload = Vec::with_capacity(packet.server_address.len().saturating_add(10));
    encode_varint(packet.protocol_version, &mut payload);
    write_string(
        &packet.server_address,
        MAX_SERVER_ADDRESS_BYTES,
        &mut payload,
    )?;
    payload.extend_from_slice(&packet.server_port.to_be_bytes());
    encode_varint(packet.next_state.wire_value(), &mut payload);
    Ok(PacketFrame {
        packet_id: HANDSHAKE_INTENTION_PACKET_ID,
        payload,
    })
}

fn require_packet_id(frame: &PacketFrame, expected: i32) -> Result<(), ProtocolError> {
    if frame.packet_id != expected {
        return Err(ProtocolError::UnexpectedPacketId {
            expected,
            actual: frame.packet_id,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        decode_handshake, encode_handshake, HandshakeNextState, HandshakePacket,
        HANDSHAKE_INTENTION_PACKET_ID,
    };
    use crate::{encode_varint, PacketFrame, ProtocolError};

    #[test]
    fn round_trips_protocol_776_status_handshake() {
        let packet = HandshakePacket {
            protocol_version: 776,
            server_address: "localhost".to_owned(),
            server_port: 25_565,
            next_state: HandshakeNextState::Status,
        };
        let frame = encode_handshake(&packet).unwrap_or(PacketFrame {
            packet_id: -1,
            payload: Vec::new(),
        });

        assert_eq!(frame.packet_id, HANDSHAKE_INTENTION_PACKET_ID);
        assert_eq!(decode_handshake(&frame), Ok(packet));
    }

    #[test]
    fn rejects_unknown_handshake_state() {
        let mut payload = Vec::new();
        encode_varint(776, &mut payload);
        encode_varint(9, &mut payload);
        payload.extend_from_slice(b"localhost");
        payload.extend_from_slice(&25_565_u16.to_be_bytes());
        encode_varint(3, &mut payload);

        assert_eq!(
            decode_handshake(&PacketFrame {
                packet_id: HANDSHAKE_INTENTION_PACKET_ID,
                payload,
            }),
            Err(ProtocolError::InvalidHandshakeNextState(3))
        );
    }

    #[test]
    fn rejects_wrong_packet_id() {
        assert_eq!(
            decode_handshake(&PacketFrame {
                packet_id: 1,
                payload: Vec::new(),
            }),
            Err(ProtocolError::UnexpectedPacketId {
                expected: HANDSHAKE_INTENTION_PACKET_ID,
                actual: 1,
            })
        );
    }

    #[test]
    fn rejects_trailing_handshake_data() {
        let packet = HandshakePacket {
            protocol_version: 776,
            server_address: "localhost".to_owned(),
            server_port: 25_565,
            next_state: HandshakeNextState::Login,
        };
        let mut frame = encode_handshake(&packet).unwrap_or(PacketFrame {
            packet_id: -1,
            payload: Vec::new(),
        });
        frame.payload.push(0);

        assert_eq!(
            decode_handshake(&frame),
            Err(ProtocolError::TrailingPacketData { remaining: 1 })
        );
    }
}
