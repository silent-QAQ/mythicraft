mod asset;
mod audio;
mod client_session;
mod dispatcher;
mod experience;
mod ui;

pub use asset::*;
pub use audio::*;
pub use client_session::*;
pub use dispatcher::*;
pub use experience::*;
pub use ui::*;

use std::collections::BTreeSet;

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub const PROTOCOL_NAMESPACE: &str = "mythicraft";
pub const SUPPORTED_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolLimits {
    pub max_message_bytes: usize,
    pub max_payload_bytes: usize,
    pub max_nesting_depth: usize,
    pub max_array_items: usize,
    pub max_string_bytes: usize,
    pub max_capabilities: usize,
}

impl Default for ProtocolLimits {
    fn default() -> Self {
        Self {
            max_message_bytes: 64 * 1024,
            max_payload_bytes: 60 * 1024,
            max_nesting_depth: 16,
            max_array_items: 256,
            max_string_bytes: 4 * 1024,
            max_capabilities: 64,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    Hello,
    Capabilities,
    UiOpen,
    UiRun,
    UiUpdate,
    UiClose,
    UiAction,
    AssetManifest,
    AssetRequest,
    AssetResult,
    AudioPlay,
    AudioStop,
    ModelSpawn,
    ModelUpdate,
    ModelVisibility,
    InputBind,
    InputAction,
    CombatDamageDisplay,
    Hologram,
    Bossbar,
    Waypoint,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadEnvelope {
    pub namespace: String,
    pub message_type: MessageType,
    pub schema_version: u16,
    pub request_id: String,
    pub payload_length: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_unix_ms: Option<u64>,
    pub payload: Value,
}

impl PayloadEnvelope {
    pub fn new<T: Serialize>(
        message_type: MessageType,
        request_id: impl Into<String>,
        nonce: Option<String>,
        expires_at_unix_ms: Option<u64>,
        payload: &T,
    ) -> Result<Self, ProtocolError> {
        let payload = serde_json::to_value(payload)?;
        let payload_length = serde_json::to_vec(&payload)?.len();
        Ok(Self {
            namespace: PROTOCOL_NAMESPACE.to_owned(),
            message_type,
            schema_version: SUPPORTED_SCHEMA_VERSION,
            request_id: request_id.into(),
            payload_length,
            nonce,
            expires_at_unix_ms,
            payload,
        })
    }

    pub fn decode(bytes: &[u8], limits: ProtocolLimits) -> Result<Self, ProtocolError> {
        if bytes.len() > limits.max_message_bytes {
            return Err(ProtocolError::MessageTooLarge {
                actual: bytes.len(),
                maximum: limits.max_message_bytes,
            });
        }
        let envelope: Self = serde_json::from_slice(bytes)?;
        envelope.validate(limits)?;
        Ok(envelope)
    }

    pub fn encode(&self, limits: ProtocolLimits) -> Result<Vec<u8>, ProtocolError> {
        self.validate(limits)?;
        let encoded = serde_json::to_vec(self)?;
        if encoded.len() > limits.max_message_bytes {
            return Err(ProtocolError::MessageTooLarge {
                actual: encoded.len(),
                maximum: limits.max_message_bytes,
            });
        }
        Ok(encoded)
    }

    pub fn payload_as<T: DeserializeOwned>(&self) -> Result<T, ProtocolError> {
        serde_json::from_value(self.payload.clone()).map_err(ProtocolError::from)
    }

    pub fn validate(&self, limits: ProtocolLimits) -> Result<(), ProtocolError> {
        if self.namespace != PROTOCOL_NAMESPACE {
            return Err(ProtocolError::UnknownNamespace(self.namespace.clone()));
        }
        if self.schema_version != SUPPORTED_SCHEMA_VERSION {
            return Err(ProtocolError::UnsupportedSchemaVersion(self.schema_version));
        }
        validate_identifier("request_id", &self.request_id, 64)?;
        if let Some(nonce) = &self.nonce {
            validate_identifier("nonce", nonce, 128)?;
        }
        let actual_payload_length = serde_json::to_vec(&self.payload)?.len();
        if actual_payload_length != self.payload_length {
            return Err(ProtocolError::PayloadLengthMismatch {
                declared: self.payload_length,
                actual: actual_payload_length,
            });
        }
        if actual_payload_length > limits.max_payload_bytes {
            return Err(ProtocolError::PayloadTooLarge {
                actual: actual_payload_length,
                maximum: limits.max_payload_bytes,
            });
        }
        validate_value(&self.payload, 1, limits)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientCapability {
    UiHud,
    UiDamageDisplay,
    UiSkillBar,
    UiDialog,
    UiBossbar,
    UiHologram,
    UiWaypoint,
    AudioPlay,
    ModelVisibility,
    InputBind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoaderDescriptor {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientHello {
    pub protocol_version: u16,
    pub mod_version: String,
    pub loader: LoaderDescriptor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_manifest_hash: Option<String>,
    pub capabilities: BTreeSet<ClientCapability>,
}

impl ClientHello {
    pub fn validate(&self, limits: ProtocolLimits) -> Result<(), ProtocolError> {
        validate_version("mod_version", &self.mod_version)?;
        validate_identifier("loader.name", &self.loader.name, 32)?;
        validate_version("loader.version", &self.loader.version)?;
        if self.capabilities.len() > limits.max_capabilities {
            return Err(ProtocolError::TooManyCapabilities {
                actual: self.capabilities.len(),
                maximum: limits.max_capabilities,
            });
        }
        if let Some(hash) = &self.resource_manifest_hash {
            if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(ProtocolError::InvalidManifestHash);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityPolicy {
    pub protocol_version: u16,
    pub supported: BTreeSet<ClientCapability>,
    pub required: BTreeSet<ClientCapability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityResponse {
    pub accepted: BTreeSet<ClientCapability>,
    pub required: BTreeSet<ClientCapability>,
    pub degraded: BTreeSet<ClientCapability>,
    pub rpg_play_allowed: bool,
    pub error_reasons: Vec<String>,
}

impl CapabilityResponse {
    pub fn validate(&self, limits: ProtocolLimits) -> Result<(), ProtocolError> {
        if self.accepted.len() > limits.max_capabilities
            || self.required.len() > limits.max_capabilities
            || self.degraded.len() > limits.max_capabilities
        {
            return Err(ProtocolError::InvalidCapabilityResponse(
                "capability set exceeds configured limit",
            ));
        }
        if !self.accepted.is_disjoint(&self.degraded) {
            return Err(ProtocolError::InvalidCapabilityResponse(
                "accepted and degraded capabilities overlap",
            ));
        }
        if self.error_reasons.len() > limits.max_array_items
            || self
                .error_reasons
                .iter()
                .any(|reason| reason.is_empty() || reason.len() > limits.max_string_bytes)
        {
            return Err(ProtocolError::InvalidCapabilityResponse(
                "error reasons exceed configured limits",
            ));
        }
        if self.rpg_play_allowed {
            if !self.required.is_subset(&self.accepted) || !self.error_reasons.is_empty() {
                return Err(ProtocolError::InvalidCapabilityResponse(
                    "allowed response must accept all required capabilities and contain no errors",
                ));
            }
        } else if self.error_reasons.is_empty() {
            return Err(ProtocolError::InvalidCapabilityResponse(
                "rejected response must include an error reason",
            ));
        }
        Ok(())
    }
}

impl CapabilityPolicy {
    pub fn negotiate(
        &self,
        hello: &ClientHello,
        limits: ProtocolLimits,
    ) -> Result<CapabilityResponse, ProtocolError> {
        hello.validate(limits)?;
        let accepted = hello
            .capabilities
            .intersection(&self.supported)
            .cloned()
            .collect();
        let degraded = self.supported.difference(&accepted).cloned().collect();
        let missing_required = self
            .required
            .difference(&accepted)
            .cloned()
            .collect::<Vec<_>>();
        let mut error_reasons = Vec::new();
        if hello.protocol_version != self.protocol_version {
            error_reasons.push(format!(
                "protocol_version_mismatch: expected {}, received {}",
                self.protocol_version, hello.protocol_version
            ));
        }
        if !missing_required.is_empty() {
            error_reasons.push(format!(
                "missing_required_capabilities: {missing_required:?}"
            ));
        }
        Ok(CapabilityResponse {
            accepted,
            required: self.required.clone(),
            degraded,
            rpg_play_allowed: error_reasons.is_empty(),
            error_reasons,
        })
    }
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("message is {actual} bytes, maximum is {maximum}")]
    MessageTooLarge { actual: usize, maximum: usize },
    #[error("payload is {actual} bytes, maximum is {maximum}")]
    PayloadTooLarge { actual: usize, maximum: usize },
    #[error("payload length mismatch: declared {declared}, actual {actual}")]
    PayloadLengthMismatch { declared: usize, actual: usize },
    #[error("unknown namespace: {0}")]
    UnknownNamespace(String),
    #[error("unsupported schema version: {0}")]
    UnsupportedSchemaVersion(u16),
    #[error("{field} is invalid")]
    InvalidIdentifier { field: &'static str },
    #[error("{field} is not a valid version string")]
    InvalidVersion { field: &'static str },
    #[error("resource manifest hash must be a 64-character hexadecimal SHA-256")]
    InvalidManifestHash,
    #[error("invalid capability response: {0}")]
    InvalidCapabilityResponse(&'static str),
    #[error("JSON nesting depth exceeds {maximum}")]
    NestingTooDeep { maximum: usize },
    #[error("array contains {actual} items, maximum is {maximum}")]
    TooManyArrayItems { actual: usize, maximum: usize },
    #[error("string contains {actual} bytes, maximum is {maximum}")]
    StringTooLarge { actual: usize, maximum: usize },
    #[error("client reported {actual} capabilities, maximum is {maximum}")]
    TooManyCapabilities { actual: usize, maximum: usize },
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

fn validate_identifier(
    field: &'static str,
    value: &str,
    maximum_length: usize,
) -> Result<(), ProtocolError> {
    let valid = !value.is_empty()
        && value.len() <= maximum_length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'));
    if valid {
        Ok(())
    } else {
        Err(ProtocolError::InvalidIdentifier { field })
    }
}

fn validate_version(field: &'static str, value: &str) -> Result<(), ProtocolError> {
    let valid = !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+' | b'_'));
    if valid {
        Ok(())
    } else {
        Err(ProtocolError::InvalidVersion { field })
    }
}

fn validate_value(
    value: &Value,
    depth: usize,
    limits: ProtocolLimits,
) -> Result<(), ProtocolError> {
    if depth > limits.max_nesting_depth {
        return Err(ProtocolError::NestingTooDeep {
            maximum: limits.max_nesting_depth,
        });
    }
    match value {
        Value::String(value) if value.len() > limits.max_string_bytes => {
            Err(ProtocolError::StringTooLarge {
                actual: value.len(),
                maximum: limits.max_string_bytes,
            })
        }
        Value::Array(values) => {
            if values.len() > limits.max_array_items {
                return Err(ProtocolError::TooManyArrayItems {
                    actual: values.len(),
                    maximum: limits.max_array_items,
                });
            }
            for value in values {
                validate_value(value, depth + 1, limits)?;
            }
            Ok(())
        }
        Value::Object(values) => {
            for (key, value) in values {
                if key.len() > limits.max_string_bytes {
                    return Err(ProtocolError::StringTooLarge {
                        actual: key.len(),
                        maximum: limits.max_string_bytes,
                    });
                }
                validate_value(value, depth + 1, limits)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
