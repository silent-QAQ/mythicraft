use std::io::{self, Read, Write};

use mythicraft_protocol::{
    decode_handshake, decode_login_start, decode_status_ping, decode_status_request, encode_frame,
    encode_login_disconnect, encode_status_pong, CompressionMode, FrameDecoder, HandshakeNextState,
    LoginDisconnect, PacketFrame, ProtocolError, StatusPong,
};
use serde_json::json;
use thiserror::Error;

use crate::{SessionError, SessionMachine, StatusConfiguration, StatusConnectionError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DevelopmentConfiguration {
    pub status: StatusConfiguration,
    pub login_rejection_message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DevelopmentConnectionPhase {
    Handshake,
    StatusRequest,
    StatusPing,
    LoginStart,
    Closed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DevelopmentConnection {
    session: SessionMachine,
    configuration: DevelopmentConfiguration,
    phase: DevelopmentConnectionPhase,
}

impl DevelopmentConnection {
    pub fn new(
        configuration: DevelopmentConfiguration,
    ) -> Result<Self, DevelopmentConnectionError> {
        if configuration.status.online_players > configuration.status.max_players {
            return Err(DevelopmentConnectionError::InvalidPlayerCounts {
                online: configuration.status.online_players,
                maximum: configuration.status.max_players,
            });
        }
        Ok(Self {
            session: SessionMachine::new(configuration.status.protocol_version),
            configuration,
            phase: DevelopmentConnectionPhase::Handshake,
        })
    }

    pub const fn phase(&self) -> DevelopmentConnectionPhase {
        self.phase
    }

    pub fn handle_frame(
        &mut self,
        frame: &PacketFrame,
    ) -> Result<Option<PacketFrame>, DevelopmentConnectionError> {
        match self.phase {
            DevelopmentConnectionPhase::Handshake => self.handle_handshake(frame),
            DevelopmentConnectionPhase::StatusRequest => {
                decode_status_request(frame)?;
                let response = self.configuration.status.response()?;
                self.phase = DevelopmentConnectionPhase::StatusPing;
                Ok(Some(response))
            }
            DevelopmentConnectionPhase::StatusPing => {
                let ping = decode_status_ping(frame)?;
                self.session.finish_status()?;
                self.phase = DevelopmentConnectionPhase::Closed;
                Ok(Some(encode_status_pong(StatusPong {
                    payload: ping.payload,
                })))
            }
            DevelopmentConnectionPhase::LoginStart => {
                let login = decode_login_start(frame)?;
                self.session.close();
                self.phase = DevelopmentConnectionPhase::Closed;
                Ok(Some(self.login_disconnect(&format!(
                    "{} Player: {}",
                    self.configuration.login_rejection_message, login.name
                ))?))
            }
            DevelopmentConnectionPhase::Closed => Err(DevelopmentConnectionError::Closed),
        }
    }

    fn handle_handshake(
        &mut self,
        frame: &PacketFrame,
    ) -> Result<Option<PacketFrame>, DevelopmentConnectionError> {
        let handshake = decode_handshake(frame)?;
        match handshake.next_state {
            HandshakeNextState::Status => {
                self.session.begin_protocol_handshake(&handshake)?;
                self.phase = DevelopmentConnectionPhase::StatusRequest;
                Ok(None)
            }
            HandshakeNextState::Login => {
                if handshake.protocol_version != self.configuration.status.protocol_version {
                    self.phase = DevelopmentConnectionPhase::Closed;
                    return Ok(Some(self.login_disconnect(&format!(
                        "Unsupported protocol {}. This server requires {}.",
                        handshake.protocol_version, self.configuration.status.protocol_version
                    ))?));
                }
                self.session.begin_protocol_handshake(&handshake)?;
                self.phase = DevelopmentConnectionPhase::LoginStart;
                Ok(None)
            }
        }
    }

    fn login_disconnect(&self, message: &str) -> Result<PacketFrame, DevelopmentConnectionError> {
        let json_reason = serde_json::to_string(&json!({ "text": message }))?;
        encode_login_disconnect(&LoginDisconnect { json_reason })
            .map_err(DevelopmentConnectionError::from)
    }
}

#[derive(Debug, Error)]
pub enum DevelopmentConnectionError {
    #[error("online player count {online} exceeds maximum {maximum}")]
    InvalidPlayerCounts { online: u32, maximum: u32 },
    #[error("development connection is closed")]
    Closed,
    #[error("protocol error: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("session error: {0}")]
    Session(#[from] SessionError),
    #[error("status configuration error: {0}")]
    Status(#[from] StatusConnectionError),
    #[error("failed to serialize disconnect JSON: {0}")]
    Serialize(#[from] serde_json::Error),
}

pub fn serve_development_io<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    configuration: DevelopmentConfiguration,
) -> Result<(), DevelopmentServerError> {
    let mut connection = DevelopmentConnection::new(configuration)?;
    let mut decoder = FrameDecoder::new(CompressionMode::Disabled);
    let mut read_buffer = [0_u8; 4096];

    loop {
        let read = reader.read(&mut read_buffer)?;
        if read == 0 {
            return Err(DevelopmentServerError::UnexpectedEof {
                phase: connection.phase(),
            });
        }
        decoder.push(&read_buffer[..read])?;
        while let Some(frame) = decoder.next_frame()? {
            if let Some(response) = connection.handle_frame(&frame)? {
                writer.write_all(&encode_frame(&response)?)?;
                writer.flush()?;
            }
            if connection.phase() == DevelopmentConnectionPhase::Closed {
                return Ok(());
            }
        }
    }
}

#[derive(Debug, Error)]
pub enum DevelopmentServerError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("protocol error: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("development connection error: {0}")]
    Connection(#[from] DevelopmentConnectionError),
    #[error("connection closed before exchange completed in phase {phase:?}")]
    UnexpectedEof { phase: DevelopmentConnectionPhase },
}

#[cfg(test)]
mod tests {
    use mythicraft_protocol::{
        decode_login_disconnect, encode_handshake, encode_login_start, HandshakeNextState,
        HandshakePacket, LoginStart,
    };
    use uuid::Uuid;

    use super::{DevelopmentConfiguration, DevelopmentConnection, DevelopmentConnectionPhase};
    use crate::StatusConfiguration;

    fn configuration() -> DevelopmentConfiguration {
        DevelopmentConfiguration {
            status: StatusConfiguration {
                version_name: "26.2".to_owned(),
                protocol_version: 776,
                motd: "Mythicraft".to_owned(),
                max_players: 20,
                online_players: 0,
            },
            login_rejection_message: "Login runtime is not enabled.".to_owned(),
        }
    }

    #[test]
    fn correct_login_is_parsed_then_explicitly_disconnected() {
        let mut connection =
            DevelopmentConnection::new(configuration()).unwrap_or_else(|_| unreachable!());
        let handshake = encode_handshake(&HandshakePacket {
            protocol_version: 776,
            server_address: "localhost".to_owned(),
            server_port: 25_565,
            next_state: HandshakeNextState::Login,
        })
        .unwrap_or_else(|_| unreachable!());
        assert!(matches!(connection.handle_frame(&handshake), Ok(None)));

        let start = encode_login_start(&LoginStart {
            name: "MythicPlayer".to_owned(),
            profile_id: Uuid::from_u128(1),
        })
        .unwrap_or_else(|_| unreachable!());
        let response = connection.handle_frame(&start).ok().flatten();

        assert!(response
            .as_ref()
            .and_then(|frame| decode_login_disconnect(frame).ok())
            .is_some_and(|disconnect| disconnect.json_reason.contains("MythicPlayer")));
        assert_eq!(connection.phase(), DevelopmentConnectionPhase::Closed);
    }

    #[test]
    fn wrong_protocol_is_disconnected_immediately() {
        let mut connection =
            DevelopmentConnection::new(configuration()).unwrap_or_else(|_| unreachable!());
        let handshake = encode_handshake(&HandshakePacket {
            protocol_version: 775,
            server_address: "localhost".to_owned(),
            server_port: 25_565,
            next_state: HandshakeNextState::Login,
        })
        .unwrap_or_else(|_| unreachable!());
        let response = connection.handle_frame(&handshake).ok().flatten();

        assert!(response
            .as_ref()
            .and_then(|frame| decode_login_disconnect(frame).ok())
            .is_some_and(|disconnect| disconnect.json_reason.contains("requires 776")));
        assert_eq!(connection.phase(), DevelopmentConnectionPhase::Closed);
    }
}
