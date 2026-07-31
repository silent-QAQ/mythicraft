use std::io::{self, Read, Write};

use mythicraft_protocol::{
    decode_handshake, decode_status_ping, decode_status_request, encode_frame, encode_status_pong,
    encode_status_response, CompressionMode, FrameDecoder, HandshakeNextState, PacketFrame,
    ProtocolError, StatusPong, StatusResponse,
};
use serde_json::json;
use thiserror::Error;

use crate::{SessionError, SessionMachine};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusConfiguration {
    pub version_name: String,
    pub protocol_version: i32,
    pub motd: String,
    pub max_players: u32,
    pub online_players: u32,
}

impl StatusConfiguration {
    pub(crate) fn response(&self) -> Result<PacketFrame, StatusConnectionError> {
        let json = serde_json::to_string(&json!({
            "version": {
                "name": self.version_name,
                "protocol": self.protocol_version,
            },
            "players": {
                "max": self.max_players,
                "online": self.online_players,
            },
            "description": {
                "text": self.motd,
            },
        }))?;
        encode_status_response(&StatusResponse { json }).map_err(StatusConnectionError::from)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusConnectionPhase {
    Handshake,
    Request,
    Ping,
    Closed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusConnection {
    session: SessionMachine,
    configuration: StatusConfiguration,
    phase: StatusConnectionPhase,
}

impl StatusConnection {
    pub fn new(configuration: StatusConfiguration) -> Result<Self, StatusConnectionError> {
        if configuration.online_players > configuration.max_players {
            return Err(StatusConnectionError::InvalidPlayerCounts {
                online: configuration.online_players,
                maximum: configuration.max_players,
            });
        }
        Ok(Self {
            session: SessionMachine::new(configuration.protocol_version),
            configuration,
            phase: StatusConnectionPhase::Handshake,
        })
    }

    pub const fn phase(&self) -> StatusConnectionPhase {
        self.phase
    }

    pub fn handle_frame(
        &mut self,
        frame: &PacketFrame,
    ) -> Result<Option<PacketFrame>, StatusConnectionError> {
        match self.phase {
            StatusConnectionPhase::Handshake => {
                let handshake = decode_handshake(frame)?;
                if handshake.next_state != HandshakeNextState::Status {
                    return Err(StatusConnectionError::LoginIntentNotSupported);
                }
                self.session.begin_protocol_handshake(&handshake)?;
                self.phase = StatusConnectionPhase::Request;
                Ok(None)
            }
            StatusConnectionPhase::Request => {
                decode_status_request(frame)?;
                let response = self.configuration.response()?;
                self.phase = StatusConnectionPhase::Ping;
                Ok(Some(response))
            }
            StatusConnectionPhase::Ping => {
                let ping = decode_status_ping(frame)?;
                self.session.finish_status()?;
                self.phase = StatusConnectionPhase::Closed;
                Ok(Some(encode_status_pong(StatusPong {
                    payload: ping.payload,
                })))
            }
            StatusConnectionPhase::Closed => Err(StatusConnectionError::Closed),
        }
    }
}

#[derive(Debug, Error)]
pub enum StatusConnectionError {
    #[error("online player count {online} exceeds maximum {maximum}")]
    InvalidPlayerCounts { online: u32, maximum: u32 },
    #[error("status-only connection does not accept login intent")]
    LoginIntentNotSupported,
    #[error("status connection is closed")]
    Closed,
    #[error("protocol error: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("session error: {0}")]
    Session(#[from] SessionError),
    #[error("failed to serialize status JSON: {0}")]
    Serialize(#[from] serde_json::Error),
}

pub fn serve_status_io<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    configuration: StatusConfiguration,
) -> Result<(), StatusServerError> {
    let mut connection = StatusConnection::new(configuration)?;
    let mut decoder = FrameDecoder::new(CompressionMode::Disabled);
    let mut read_buffer = [0_u8; 4096];

    loop {
        let read = reader.read(&mut read_buffer)?;
        if read == 0 {
            return Err(StatusServerError::UnexpectedEof {
                phase: connection.phase(),
            });
        }
        decoder.push(&read_buffer[..read])?;

        while let Some(frame) = decoder.next_frame()? {
            if let Some(response) = connection.handle_frame(&frame)? {
                writer.write_all(&encode_frame(&response)?)?;
                writer.flush()?;
            }
            if connection.phase() == StatusConnectionPhase::Closed {
                return Ok(());
            }
        }
    }
}

#[derive(Debug, Error)]
pub enum StatusServerError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("protocol error: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("status connection error: {0}")]
    Connection(#[from] StatusConnectionError),
    #[error("connection closed before status exchange completed in phase {phase:?}")]
    UnexpectedEof { phase: StatusConnectionPhase },
}

#[cfg(test)]
mod tests {
    use mythicraft_protocol::{
        decode_status_pong, decode_status_response, encode_frame, encode_handshake,
        encode_status_ping, encode_status_request, CompressionMode, FrameDecoder,
        HandshakeNextState, HandshakePacket, StatusPing, StatusRequest,
    };
    use std::io::Cursor;

    use super::{
        serve_status_io, StatusConfiguration, StatusConnection, StatusConnectionError,
        StatusConnectionPhase,
    };

    fn connection() -> StatusConnection {
        StatusConnection::new(StatusConfiguration {
            version_name: "26.2".to_owned(),
            protocol_version: 776,
            motd: "Mythicraft".to_owned(),
            max_players: 20,
            online_players: 3,
        })
        .unwrap_or_else(|_| unreachable!())
    }

    #[test]
    fn completes_status_request_and_ping_sequence() {
        let mut connection = connection();
        let handshake = encode_handshake(&HandshakePacket {
            protocol_version: 776,
            server_address: "localhost".to_owned(),
            server_port: 25_565,
            next_state: HandshakeNextState::Status,
        });
        assert!(matches!(
            handshake
                .as_ref()
                .map_err(|error| error.to_string())
                .and_then(|frame| {
                    connection
                        .handle_frame(frame)
                        .map_err(|error| error.to_string())
                }),
            Ok(None)
        ));
        assert_eq!(connection.phase(), StatusConnectionPhase::Request);

        let response = connection.handle_frame(&encode_status_request(StatusRequest));
        let response = response.ok().flatten();
        assert!(response
            .as_ref()
            .and_then(|frame| decode_status_response(frame).ok())
            .is_some_and(|status| status.json.contains("Mythicraft")));
        assert_eq!(connection.phase(), StatusConnectionPhase::Ping);

        let pong = connection.handle_frame(&encode_status_ping(StatusPing { payload: 42 }));
        assert_eq!(
            pong.ok()
                .flatten()
                .as_ref()
                .and_then(|frame| decode_status_pong(frame).ok())
                .map(|pong| pong.payload),
            Some(42)
        );
        assert_eq!(connection.phase(), StatusConnectionPhase::Closed);
    }

    #[test]
    fn rejects_login_intent_without_mutating_phase() {
        let mut connection = connection();
        let handshake = encode_handshake(&HandshakePacket {
            protocol_version: 776,
            server_address: "localhost".to_owned(),
            server_port: 25_565,
            next_state: HandshakeNextState::Login,
        });
        let result = handshake
            .as_ref()
            .map_err(|error| error.to_string())
            .and_then(|frame| {
                connection
                    .handle_frame(frame)
                    .map_err(|error| error.to_string())
            });

        assert_eq!(
            result,
            Err(StatusConnectionError::LoginIntentNotSupported.to_string())
        );
        assert_eq!(connection.phase(), StatusConnectionPhase::Handshake);
    }

    #[test]
    fn complete_wire_exchange_writes_status_and_pong_frames() {
        let handshake = encode_handshake(&HandshakePacket {
            protocol_version: 776,
            server_address: "localhost".to_owned(),
            server_port: 25_565,
            next_state: HandshakeNextState::Status,
        });
        let mut input = handshake
            .and_then(|frame| encode_frame(&frame))
            .unwrap_or_default();
        input.extend_from_slice(
            &encode_frame(&encode_status_request(StatusRequest)).unwrap_or_default(),
        );
        input.extend_from_slice(
            &encode_frame(&encode_status_ping(StatusPing { payload: 99 })).unwrap_or_default(),
        );
        let mut reader = Cursor::new(input);
        let mut output = Vec::new();

        assert!(serve_status_io(
            &mut reader,
            &mut output,
            StatusConfiguration {
                version_name: "26.2".to_owned(),
                protocol_version: 776,
                motd: "Mythicraft".to_owned(),
                max_players: 20,
                online_players: 0,
            },
        )
        .is_ok());

        let mut decoder = FrameDecoder::new(CompressionMode::Disabled);
        assert_eq!(decoder.push(&output), Ok(()));
        let status = decoder.next_frame().ok().flatten();
        let pong = decoder.next_frame().ok().flatten();
        assert!(status
            .as_ref()
            .and_then(|frame| decode_status_response(frame).ok())
            .is_some_and(|response| response.json.contains("Mythicraft")));
        assert_eq!(
            pong.as_ref()
                .and_then(|frame| decode_status_pong(frame).ok())
                .map(|pong| pong.payload),
            Some(99)
        );
    }
}
