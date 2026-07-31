use crate::io::{write_string, PacketCursor};
use crate::{PacketFrame, ProtocolError, MAX_STATUS_JSON_BYTES};

pub const STATUS_REQUEST_PACKET_ID: i32 = 0;
pub const STATUS_PING_REQUEST_PACKET_ID: i32 = 1;
pub const STATUS_RESPONSE_PACKET_ID: i32 = 0;
pub const STATUS_PONG_RESPONSE_PACKET_ID: i32 = 1;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StatusRequest;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusResponse {
    pub json: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StatusPing {
    pub payload: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StatusPong {
    pub payload: i64,
}

pub fn decode_status_request(frame: &PacketFrame) -> Result<StatusRequest, ProtocolError> {
    require_packet_id(frame, STATUS_REQUEST_PACKET_ID)?;
    PacketCursor::new(&frame.payload).finish()?;
    Ok(StatusRequest)
}

pub fn encode_status_request(_: StatusRequest) -> PacketFrame {
    PacketFrame {
        packet_id: STATUS_REQUEST_PACKET_ID,
        payload: Vec::new(),
    }
}

pub fn decode_status_response(frame: &PacketFrame) -> Result<StatusResponse, ProtocolError> {
    require_packet_id(frame, STATUS_RESPONSE_PACKET_ID)?;
    let mut cursor = PacketCursor::new(&frame.payload);
    let json = cursor.read_string(MAX_STATUS_JSON_BYTES)?;
    cursor.finish()?;
    Ok(StatusResponse { json })
}

pub fn encode_status_response(response: &StatusResponse) -> Result<PacketFrame, ProtocolError> {
    let mut payload = Vec::with_capacity(response.json.len().saturating_add(5));
    write_string(&response.json, MAX_STATUS_JSON_BYTES, &mut payload)?;
    Ok(PacketFrame {
        packet_id: STATUS_RESPONSE_PACKET_ID,
        payload,
    })
}

pub fn decode_status_ping(frame: &PacketFrame) -> Result<StatusPing, ProtocolError> {
    require_packet_id(frame, STATUS_PING_REQUEST_PACKET_ID)?;
    let mut cursor = PacketCursor::new(&frame.payload);
    let payload = cursor.read_i64("status ping payload")?;
    cursor.finish()?;
    Ok(StatusPing { payload })
}

pub fn encode_status_ping(ping: StatusPing) -> PacketFrame {
    PacketFrame {
        packet_id: STATUS_PING_REQUEST_PACKET_ID,
        payload: ping.payload.to_be_bytes().to_vec(),
    }
}

pub fn decode_status_pong(frame: &PacketFrame) -> Result<StatusPong, ProtocolError> {
    require_packet_id(frame, STATUS_PONG_RESPONSE_PACKET_ID)?;
    let mut cursor = PacketCursor::new(&frame.payload);
    let payload = cursor.read_i64("status pong payload")?;
    cursor.finish()?;
    Ok(StatusPong { payload })
}

pub fn encode_status_pong(pong: StatusPong) -> PacketFrame {
    PacketFrame {
        packet_id: STATUS_PONG_RESPONSE_PACKET_ID,
        payload: pong.payload.to_be_bytes().to_vec(),
    }
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
        decode_status_ping, decode_status_pong, decode_status_request, decode_status_response,
        encode_status_ping, encode_status_pong, encode_status_request, encode_status_response,
        StatusPing, StatusPong, StatusRequest, StatusResponse,
    };
    use crate::{PacketFrame, ProtocolError};

    #[test]
    fn empty_status_request_round_trips() {
        let frame = encode_status_request(StatusRequest);
        assert_eq!(decode_status_request(&frame), Ok(StatusRequest));
    }

    #[test]
    fn status_request_rejects_payload() {
        assert_eq!(
            decode_status_request(&PacketFrame {
                packet_id: 0,
                payload: vec![0],
            }),
            Err(ProtocolError::TrailingPacketData { remaining: 1 })
        );
    }

    #[test]
    fn status_json_round_trips() {
        let response = StatusResponse {
            json: r#"{"version":{"name":"26.2","protocol":776},"players":{"max":20,"online":0},"description":{"text":"Mythicraft"}}"#.to_owned(),
        };
        let frame = encode_status_response(&response).unwrap_or(PacketFrame {
            packet_id: -1,
            payload: Vec::new(),
        });

        assert_eq!(decode_status_response(&frame), Ok(response));
    }

    #[test]
    fn ping_and_pong_preserve_payload() {
        let ping = StatusPing {
            payload: -9_223_372_036_854_775_000,
        };
        let pong = StatusPong {
            payload: ping.payload,
        };

        assert_eq!(decode_status_ping(&encode_status_ping(ping)), Ok(ping));
        assert_eq!(decode_status_pong(&encode_status_pong(pong)), Ok(pong));
    }

    #[test]
    fn truncated_ping_is_rejected() {
        assert_eq!(
            decode_status_ping(&PacketFrame {
                packet_id: 1,
                payload: vec![0; 7],
            }),
            Err(ProtocolError::TruncatedField {
                field: "status ping payload",
                needed: 8,
                remaining: 7,
            })
        );
    }
}
