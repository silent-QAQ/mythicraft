mod development_service;
mod status_service;

use mythicraft_api::TickId;
use mythicraft_protocol::{
    decode_login_acknowledged, HandshakeNextState, HandshakePacket, PacketFrame, ProtocolError,
};
use thiserror::Error;

pub use development_service::{
    serve_development_io, DevelopmentConfiguration, DevelopmentConnection,
    DevelopmentConnectionError, DevelopmentConnectionPhase, DevelopmentServerError,
};
pub use status_service::{
    serve_status_io, StatusConfiguration, StatusConnection, StatusConnectionError,
    StatusConnectionPhase, StatusServerError,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionState {
    Handshake,
    Status,
    Login,
    Configuration,
    Play,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandshakeIntent {
    Status,
    Login,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct KeepAliveId(pub i64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeepAliveChallenge {
    pub id: KeepAliveId,
    pub sent_tick: TickId,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct KeepAliveTracker {
    pending: Option<KeepAliveChallenge>,
}

impl KeepAliveTracker {
    pub const fn pending(&self) -> Option<KeepAliveChallenge> {
        self.pending
    }

    pub fn issue(&mut self, id: KeepAliveId, sent_tick: TickId) -> Result<(), KeepAliveError> {
        if let Some(pending) = self.pending {
            return Err(KeepAliveError::AlreadyPending(pending.id));
        }
        self.pending = Some(KeepAliveChallenge { id, sent_tick });
        Ok(())
    }

    pub fn acknowledge(
        &mut self,
        received: KeepAliveId,
    ) -> Result<KeepAliveChallenge, KeepAliveError> {
        let pending = self.pending.ok_or(KeepAliveError::UnexpectedResponse)?;
        if pending.id != received {
            return Err(KeepAliveError::MismatchedResponse {
                expected: pending.id,
                received,
            });
        }
        self.pending = None;
        Ok(pending)
    }

    pub fn expire(
        &mut self,
        current_tick: TickId,
        timeout_ticks: u64,
    ) -> Option<KeepAliveChallenge> {
        let pending = self.pending?;
        if current_tick.0.saturating_sub(pending.sent_tick.0) < timeout_ticks {
            return None;
        }
        self.pending = None;
        Some(pending)
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum KeepAliveError {
    #[error("keep-alive {0:?} is already awaiting a response")]
    AlreadyPending(KeepAliveId),
    #[error("received keep-alive response without a pending challenge")]
    UnexpectedResponse,
    #[error("keep-alive response mismatch: expected {expected:?}, received {received:?}")]
    MismatchedResponse {
        expected: KeepAliveId,
        received: KeepAliveId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionMachine {
    state: SessionState,
    supported_protocol: i32,
    negotiated_protocol: Option<i32>,
}

impl SessionMachine {
    pub const fn new(supported_protocol: i32) -> Self {
        Self {
            state: SessionState::Handshake,
            supported_protocol,
            negotiated_protocol: None,
        }
    }

    pub const fn state(&self) -> SessionState {
        self.state
    }

    pub const fn negotiated_protocol(&self) -> Option<i32> {
        self.negotiated_protocol
    }

    pub fn begin_handshake(
        &mut self,
        protocol_version: i32,
        intent: HandshakeIntent,
    ) -> Result<(), SessionError> {
        self.require_state(SessionState::Handshake, "begin handshake")?;
        if intent == HandshakeIntent::Login && protocol_version != self.supported_protocol {
            return Err(SessionError::UnsupportedProtocol {
                supported: self.supported_protocol,
                requested: protocol_version,
            });
        }

        self.negotiated_protocol = Some(protocol_version);
        self.state = match intent {
            HandshakeIntent::Status => SessionState::Status,
            HandshakeIntent::Login => SessionState::Login,
        };
        Ok(())
    }

    pub fn begin_protocol_handshake(
        &mut self,
        packet: &HandshakePacket,
    ) -> Result<(), SessionError> {
        let intent = match packet.next_state {
            HandshakeNextState::Status => HandshakeIntent::Status,
            HandshakeNextState::Login => HandshakeIntent::Login,
        };
        self.begin_handshake(packet.protocol_version, intent)
    }

    pub fn finish_status(&mut self) -> Result<(), SessionError> {
        self.require_state(SessionState::Status, "finish status")?;
        self.state = SessionState::Closed;
        Ok(())
    }

    pub fn finish_login(&mut self) -> Result<(), SessionError> {
        self.require_state(SessionState::Login, "finish login")?;
        self.state = SessionState::Configuration;
        Ok(())
    }

    pub fn accept_login_acknowledged(&mut self, frame: &PacketFrame) -> Result<(), SessionError> {
        self.require_state(SessionState::Login, "accept login acknowledgement")?;
        decode_login_acknowledged(frame)?;
        self.finish_login()
    }

    pub fn finish_configuration(&mut self) -> Result<(), SessionError> {
        self.require_state(SessionState::Configuration, "finish configuration")?;
        self.state = SessionState::Play;
        Ok(())
    }

    pub fn close(&mut self) {
        self.state = SessionState::Closed;
    }

    fn require_state(
        &self,
        expected: SessionState,
        action: &'static str,
    ) -> Result<(), SessionError> {
        if self.state == SessionState::Closed {
            return Err(SessionError::Closed);
        }
        if self.state != expected {
            return Err(SessionError::InvalidTransition {
                state: self.state,
                action,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum SessionError {
    #[error("session is closed")]
    Closed,
    #[error("cannot {action} while session is in {state:?}")]
    InvalidTransition {
        state: SessionState,
        action: &'static str,
    },
    #[error("unsupported protocol {requested}; server requires {supported}")]
    UnsupportedProtocol { supported: i32, requested: i32 },
    #[error("protocol packet rejected: {0}")]
    Protocol(#[from] ProtocolError),
}

#[cfg(test)]
mod tests {
    use mythicraft_api::TickId;
    use mythicraft_protocol::{
        encode_login_acknowledged, HandshakeNextState, HandshakePacket, LoginAcknowledged,
    };

    use super::{
        HandshakeIntent, KeepAliveError, KeepAliveId, KeepAliveTracker, SessionError,
        SessionMachine, SessionState,
    };

    const TARGET_PROTOCOL: i32 = 776;

    #[test]
    fn login_reaches_play_only_through_configuration() {
        let mut session = SessionMachine::new(TARGET_PROTOCOL);

        assert_eq!(
            session.begin_handshake(TARGET_PROTOCOL, HandshakeIntent::Login),
            Ok(())
        );
        assert_eq!(session.state(), SessionState::Login);
        assert_eq!(session.finish_login(), Ok(()));
        assert_eq!(session.state(), SessionState::Configuration);
        assert_eq!(session.finish_configuration(), Ok(()));
        assert_eq!(session.state(), SessionState::Play);
    }

    #[test]
    fn wrong_login_protocol_is_rejected_without_state_change() {
        let mut session = SessionMachine::new(TARGET_PROTOCOL);

        assert_eq!(
            session.begin_handshake(775, HandshakeIntent::Login),
            Err(SessionError::UnsupportedProtocol {
                supported: TARGET_PROTOCOL,
                requested: 775,
            })
        );
        assert_eq!(session.state(), SessionState::Handshake);
        assert_eq!(session.negotiated_protocol(), None);
    }

    #[test]
    fn status_query_can_report_version_mismatch_then_closes() {
        let mut session = SessionMachine::new(TARGET_PROTOCOL);

        assert_eq!(
            session.begin_handshake(775, HandshakeIntent::Status),
            Ok(())
        );
        assert_eq!(session.negotiated_protocol(), Some(775));
        assert_eq!(session.finish_status(), Ok(()));
        assert_eq!(session.state(), SessionState::Closed);
    }

    #[test]
    fn invalid_transition_is_rejected() {
        let mut session = SessionMachine::new(TARGET_PROTOCOL);

        assert_eq!(
            session.finish_configuration(),
            Err(SessionError::InvalidTransition {
                state: SessionState::Handshake,
                action: "finish configuration",
            })
        );
    }

    #[test]
    fn closed_session_rejects_transitions() {
        let mut session = SessionMachine::new(TARGET_PROTOCOL);
        session.close();

        assert_eq!(
            session.begin_handshake(TARGET_PROTOCOL, HandshakeIntent::Login),
            Err(SessionError::Closed)
        );
    }

    #[test]
    fn keep_alive_requires_matching_response() {
        let mut tracker = KeepAliveTracker::default();
        let expected = KeepAliveId(42);

        assert_eq!(tracker.issue(expected, TickId(100)), Ok(()));
        assert_eq!(
            tracker.acknowledge(KeepAliveId(43)),
            Err(KeepAliveError::MismatchedResponse {
                expected,
                received: KeepAliveId(43),
            })
        );
        assert_eq!(tracker.pending().map(|pending| pending.id), Some(expected));
        assert_eq!(
            tracker
                .acknowledge(expected)
                .map(|pending| pending.sent_tick),
            Ok(TickId(100))
        );
        assert_eq!(tracker.pending(), None);
    }

    #[test]
    fn keep_alive_expires_at_tick_boundary() {
        let mut tracker = KeepAliveTracker::default();
        let id = KeepAliveId(7);
        assert_eq!(tracker.issue(id, TickId(100)), Ok(()));

        assert_eq!(tracker.expire(TickId(119), 20), None);
        assert_eq!(
            tracker.expire(TickId(120), 20).map(|pending| pending.id),
            Some(id)
        );
        assert_eq!(tracker.pending(), None);
    }

    #[test]
    fn keep_alive_disallows_overlapping_challenges() {
        let mut tracker = KeepAliveTracker::default();
        assert_eq!(tracker.issue(KeepAliveId(1), TickId(5)), Ok(()));

        assert_eq!(
            tracker.issue(KeepAliveId(2), TickId(6)),
            Err(KeepAliveError::AlreadyPending(KeepAliveId(1)))
        );
    }

    #[test]
    fn protocol_handshake_and_login_acknowledgement_reach_configuration() {
        let mut session = SessionMachine::new(TARGET_PROTOCOL);
        let handshake = HandshakePacket {
            protocol_version: TARGET_PROTOCOL,
            server_address: "localhost".to_owned(),
            server_port: 25_565,
            next_state: HandshakeNextState::Login,
        };

        assert_eq!(session.begin_protocol_handshake(&handshake), Ok(()));
        assert_eq!(session.state(), SessionState::Login);
        assert_eq!(
            session.accept_login_acknowledged(&encode_login_acknowledged(LoginAcknowledged)),
            Ok(())
        );
        assert_eq!(session.state(), SessionState::Configuration);
    }
}
