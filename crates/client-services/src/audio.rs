use std::collections::VecDeque;

use mythicraft_api::EntityId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioPosition {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioChannel {
    Master,
    Music,
    Effects,
    Ambient,
    Voice,
    Ui,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioPlay {
    pub event_id: String,
    pub sound_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<AudioPosition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow_entity: Option<EntityId>,
    pub volume: f32,
    pub channel: AudioChannel,
    pub priority: u8,
    pub expires_at_unix_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_distance: Option<f32>,
}

impl AudioPlay {
    pub fn validate(&self) -> Result<(), AudioError> {
        validate_identifier("event_id", &self.event_id, 64)?;
        validate_resource_id(&self.sound_id)?;
        if self.position.is_some() && self.follow_entity.is_some() {
            return Err(AudioError::AmbiguousLocation);
        }
        if let Some(position) = self.position {
            if !position.x.is_finite() || !position.y.is_finite() || !position.z.is_finite() {
                return Err(AudioError::NonFinitePosition);
            }
        }
        if self.follow_entity == Some(EntityId(0)) {
            return Err(AudioError::InvalidEntityId);
        }
        if !self.volume.is_finite() || !(0.0..=4.0).contains(&self.volume) {
            return Err(AudioError::InvalidVolume(self.volume));
        }
        if self.priority > 100 {
            return Err(AudioError::InvalidPriority(self.priority));
        }
        if self.expires_at_unix_ms == 0 {
            return Err(AudioError::MissingExpiry);
        }
        if let Some(max_distance) = self.max_distance {
            if !max_distance.is_finite() || !(0.0..=512.0).contains(&max_distance) {
                return Err(AudioError::InvalidMaxDistance(max_distance));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioStop {
    pub event_id: String,
    pub fade_out_ms: u32,
}

impl AudioStop {
    pub fn validate(&self) -> Result<(), AudioError> {
        validate_identifier("event_id", &self.event_id, 64)?;
        if self.fade_out_ms > 10_000 {
            return Err(AudioError::FadeOutTooLong(self.fade_out_ms));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioDecision {
    Play,
    DropExpired,
    DropMissingResource,
    DropRateLimited,
}

#[derive(Debug)]
pub struct AudioClientGate {
    max_events: usize,
    window_ms: u64,
    accepted_events: VecDeque<u64>,
}

impl AudioClientGate {
    pub fn new(max_events: usize, window_ms: u64) -> Result<Self, AudioError> {
        if max_events == 0 || window_ms == 0 {
            return Err(AudioError::InvalidRateLimit);
        }
        Ok(Self {
            max_events,
            window_ms,
            accepted_events: VecDeque::new(),
        })
    }

    pub fn evaluate(
        &mut self,
        event: &AudioPlay,
        now_unix_ms: u64,
        resource_available: bool,
    ) -> Result<AudioDecision, AudioError> {
        event.validate()?;
        if event.expires_at_unix_ms <= now_unix_ms {
            return Ok(AudioDecision::DropExpired);
        }
        if !resource_available {
            return Ok(AudioDecision::DropMissingResource);
        }
        while self
            .accepted_events
            .front()
            .is_some_and(|timestamp| now_unix_ms.saturating_sub(*timestamp) >= self.window_ms)
        {
            self.accepted_events.pop_front();
        }
        if self.accepted_events.len() >= self.max_events {
            return Ok(AudioDecision::DropRateLimited);
        }
        self.accepted_events.push_back(now_unix_ms);
        Ok(AudioDecision::Play)
    }
}

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("{field} is invalid")]
    InvalidIdentifier { field: &'static str },
    #[error("sound resource ID is invalid")]
    InvalidSoundId,
    #[error("audio event cannot use position and follow_entity together")]
    AmbiguousLocation,
    #[error("audio position must contain finite coordinates")]
    NonFinitePosition,
    #[error("follow_entity must be greater than zero")]
    InvalidEntityId,
    #[error("volume {0} is outside 0.0..=4.0")]
    InvalidVolume(f32),
    #[error("priority {0} is outside 0..=100")]
    InvalidPriority(u8),
    #[error("audio event expiry is required")]
    MissingExpiry,
    #[error("max distance {0} is outside 0.0..=512.0")]
    InvalidMaxDistance(f32),
    #[error("fade out {0}ms exceeds 10000ms")]
    FadeOutTooLong(u32),
    #[error("audio rate limit must be non-zero")]
    InvalidRateLimit,
}

fn validate_identifier(
    field: &'static str,
    value: &str,
    maximum_length: usize,
) -> Result<(), AudioError> {
    let valid = !value.is_empty()
        && value.len() <= maximum_length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'));
    if valid {
        Ok(())
    } else {
        Err(AudioError::InvalidIdentifier { field })
    }
}

fn validate_resource_id(value: &str) -> Result<(), AudioError> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && value.contains(':')
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'_' | b'-' | b'.' | b':' | b'/')
        });
    if valid {
        Ok(())
    } else {
        Err(AudioError::InvalidSoundId)
    }
}
