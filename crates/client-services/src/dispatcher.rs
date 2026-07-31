use thiserror::Error;

use crate::{
    AssetLimits, AssetManifest, AssetRequest, AssetResult, AudioPlay, AudioStop, BossbarState,
    CapabilityResponse, ClientHello, DamageDisplay, HologramHealthBar, MessageType,
    ModelVisibility, PayloadEnvelope, ProtocolError, ProtocolLimits, UiAction, UiClose, UiOpen,
    UiRun, UiUpdate, WaypointState,
};

#[derive(Debug, Clone, PartialEq)]
pub enum ProtocolMessage {
    Hello(ClientHello),
    Capabilities(CapabilityResponse),
    UiOpen(UiOpen),
    UiRun(UiRun),
    UiUpdate(UiUpdate),
    UiClose(UiClose),
    UiAction(UiAction),
    AssetManifest(AssetManifest),
    AssetRequest(AssetRequest),
    AssetResult(AssetResult),
    AudioPlay(AudioPlay),
    AudioStop(AudioStop),
    DamageDisplay(DamageDisplay),
    Hologram(HologramHealthBar),
    Bossbar(BossbarState),
    Waypoint(WaypointState),
    ModelVisibility(ModelVisibility),
}

#[derive(Debug, Clone, Copy)]
pub struct ProtocolDispatcher {
    limits: ProtocolLimits,
    asset_limits: AssetLimits,
}

impl ProtocolDispatcher {
    pub fn new(limits: ProtocolLimits, asset_limits: AssetLimits) -> Self {
        Self {
            limits,
            asset_limits,
        }
    }

    pub fn decode(&self, bytes: &[u8]) -> Result<ProtocolMessage, DispatchError> {
        let envelope = PayloadEnvelope::decode(bytes, self.limits)?;
        self.route(envelope)
    }

    pub fn route(&self, envelope: PayloadEnvelope) -> Result<ProtocolMessage, DispatchError> {
        envelope.validate(self.limits)?;
        match envelope.message_type {
            MessageType::Hello => {
                let message: ClientHello = envelope.payload_as()?;
                message.validate(self.limits)?;
                Ok(ProtocolMessage::Hello(message))
            }
            MessageType::Capabilities => {
                let message: CapabilityResponse = envelope.payload_as()?;
                message.validate(self.limits)?;
                Ok(ProtocolMessage::Capabilities(message))
            }
            MessageType::UiOpen => {
                let message: UiOpen = envelope.payload_as()?;
                message.validate(self.limits)?;
                Ok(ProtocolMessage::UiOpen(message))
            }
            MessageType::UiRun => {
                let message: UiRun = envelope.payload_as()?;
                message.validate(self.limits)?;
                Ok(ProtocolMessage::UiRun(message))
            }
            MessageType::UiUpdate => {
                let message: UiUpdate = envelope.payload_as()?;
                message.validate(self.limits)?;
                Ok(ProtocolMessage::UiUpdate(message))
            }
            MessageType::UiClose => {
                let message: UiClose = envelope.payload_as()?;
                message.validate()?;
                Ok(ProtocolMessage::UiClose(message))
            }
            MessageType::UiAction => {
                let message: UiAction = envelope.payload_as()?;
                message.validate(self.limits)?;
                Ok(ProtocolMessage::UiAction(message))
            }
            MessageType::AssetManifest => {
                let message: AssetManifest = envelope.payload_as()?;
                message.validate(self.asset_limits)?;
                Ok(ProtocolMessage::AssetManifest(message))
            }
            MessageType::AssetRequest => {
                let message: AssetRequest = envelope.payload_as()?;
                message.validate(self.asset_limits)?;
                Ok(ProtocolMessage::AssetRequest(message))
            }
            MessageType::AssetResult => Ok(ProtocolMessage::AssetResult(envelope.payload_as()?)),
            MessageType::AudioPlay => {
                let message: AudioPlay = envelope.payload_as()?;
                message.validate()?;
                Ok(ProtocolMessage::AudioPlay(message))
            }
            MessageType::AudioStop => {
                let message: AudioStop = envelope.payload_as()?;
                message.validate()?;
                Ok(ProtocolMessage::AudioStop(message))
            }
            MessageType::CombatDamageDisplay => {
                let message: DamageDisplay = envelope.payload_as()?;
                message.validate()?;
                Ok(ProtocolMessage::DamageDisplay(message))
            }
            MessageType::Hologram => {
                let message: HologramHealthBar = envelope.payload_as()?;
                message.validate()?;
                Ok(ProtocolMessage::Hologram(message))
            }
            MessageType::Bossbar => {
                let message: BossbarState = envelope.payload_as()?;
                message.validate()?;
                Ok(ProtocolMessage::Bossbar(message))
            }
            MessageType::Waypoint => {
                let message: WaypointState = envelope.payload_as()?;
                message.validate()?;
                Ok(ProtocolMessage::Waypoint(message))
            }
            MessageType::ModelVisibility => {
                let message: ModelVisibility = envelope.payload_as()?;
                message.validate()?;
                Ok(ProtocolMessage::ModelVisibility(message))
            }
            unsupported => Err(DispatchError::UnsupportedMessageType(unsupported)),
        }
    }
}

impl Default for ProtocolDispatcher {
    fn default() -> Self {
        Self::new(ProtocolLimits::default(), AssetLimits::default())
    }
}

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error(transparent)]
    Ui(#[from] crate::UiProtocolError),
    #[error(transparent)]
    Asset(#[from] crate::AssetError),
    #[error(transparent)]
    Audio(#[from] crate::AudioError),
    #[error(transparent)]
    Experience(#[from] crate::ExperienceError),
    #[error("message type is not implemented by the v1 dispatcher: {0:?}")]
    UnsupportedMessageType(MessageType),
}
