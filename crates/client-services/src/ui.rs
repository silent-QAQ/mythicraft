use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::{ClientCapability, MessageType, PayloadEnvelope, ProtocolError, ProtocolLimits};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiOpen {
    pub page_id: String,
    pub page_version: u64,
    pub model: Value,
    #[serde(default)]
    pub required_capabilities: Vec<ClientCapability>,
    #[serde(default)]
    pub required_permissions: Vec<String>,
}

/// Requests the native client mod to execute a server-approved Aria/UI snippet.
///
/// The snippet is always sourced from a loaded ArcartX configuration. It is not accepted as
/// executable server input from the client; the server only emits it after the UI action gate
/// has validated page version, nonce, permission, expiry and replay state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiRun {
    pub page_id: String,
    pub page_version: u64,
    pub code: String,
}

impl UiRun {
    pub fn validate(&self, limits: ProtocolLimits) -> Result<(), UiProtocolError> {
        validate_ui_identifier("page_id", &self.page_id, 96)?;
        validate_page_version(self.page_version)?;
        validate_string(&self.code, limits)
    }
}

impl UiOpen {
    pub fn validate(&self, limits: ProtocolLimits) -> Result<(), UiProtocolError> {
        validate_ui_identifier("page_id", &self.page_id, 96)?;
        validate_page_version(self.page_version)?;
        validate_requirements(self, limits)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiUpdate {
    pub page_id: String,
    pub expected_page_version: u64,
    pub page_version: u64,
    pub fields: BTreeMap<String, Value>,
}

impl UiUpdate {
    pub fn validate(&self, limits: ProtocolLimits) -> Result<(), UiProtocolError> {
        validate_ui_identifier("page_id", &self.page_id, 96)?;
        validate_page_version(self.expected_page_version)?;
        validate_page_version(self.page_version)?;
        if self.page_version <= self.expected_page_version {
            return Err(UiProtocolError::NonIncreasingPageVersion {
                expected: self.expected_page_version,
                received: self.page_version,
            });
        }
        if self.fields.is_empty() || self.fields.len() > limits.max_array_items {
            return Err(UiProtocolError::InvalidFieldCount {
                actual: self.fields.len(),
                maximum: limits.max_array_items,
            });
        }
        for (field, value) in &self.fields {
            validate_ui_identifier("field", field, 96)?;
            validate_ui_value(value, 1, limits)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiCloseReason {
    Completed,
    Cancelled,
    Replaced,
    PermissionChanged,
    OutOfRange,
    ServerShutdown,
    ProtocolError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiCleanupPolicy {
    Immediate,
    KeepVisualState,
    ClearInputs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiClose {
    pub page_id: String,
    pub page_version: u64,
    pub reason: UiCloseReason,
    pub cleanup: UiCleanupPolicy,
}

impl UiClose {
    pub fn validate(&self) -> Result<(), UiProtocolError> {
        validate_ui_identifier("page_id", &self.page_id, 96)?;
        validate_page_version(self.page_version)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiActionType {
    Click,
    Submit,
    Change,
    KeyPress,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum UiInputValue {
    Text(String),
    Integer(i64),
    Decimal(f64),
    Boolean(bool),
    Selection(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiAction {
    pub page_id: String,
    pub control_id: String,
    pub action_type: UiActionType,
    pub page_version: u64,
    pub nonce: String,
    pub expires_at_unix_ms: u64,
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<UiInputValue>,
}

impl UiAction {
    pub fn validate(&self, limits: ProtocolLimits) -> Result<(), UiProtocolError> {
        validate_ui_identifier("page_id", &self.page_id, 96)?;
        validate_ui_identifier("control_id", &self.control_id, 96)?;
        validate_ui_identifier("nonce", &self.nonce, 128)?;
        validate_ui_identifier("request_id", &self.request_id, 64)?;
        validate_page_version(self.page_version)?;
        if self.expires_at_unix_ms == 0 {
            return Err(UiProtocolError::MissingExpiry);
        }
        if let Some(input) = &self.input {
            validate_input(input, limits)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct UiActionContext<'a> {
    pub now_unix_ms: u64,
    pub expected_page_version: u64,
    pub expected_nonce: &'a str,
    pub permission_granted: bool,
    pub in_range: bool,
    pub state_allowed: bool,
}

#[derive(Debug)]
pub struct UiActionGate {
    max_actions: usize,
    window_ms: u64,
    attempts: VecDeque<u64>,
    seen_requests: BTreeMap<String, u64>,
}

impl UiActionGate {
    pub fn new(max_actions: usize, window_ms: u64) -> Result<Self, UiActionError> {
        if max_actions == 0 || window_ms == 0 {
            return Err(UiActionError::InvalidRateLimit);
        }
        Ok(Self {
            max_actions,
            window_ms,
            attempts: VecDeque::new(),
            seen_requests: BTreeMap::new(),
        })
    }

    pub fn validate_and_record(
        &mut self,
        envelope: &PayloadEnvelope,
        action: &UiAction,
        context: UiActionContext<'_>,
        limits: ProtocolLimits,
    ) -> Result<(), UiActionError> {
        envelope.validate(limits)?;
        action.validate(limits)?;
        let envelope_action: UiAction = envelope.payload_as()?;
        if &envelope_action != action {
            return Err(UiActionError::PayloadMismatch);
        }
        if envelope.message_type != MessageType::UiAction {
            return Err(UiActionError::WrongMessageType);
        }
        if envelope.request_id != action.request_id {
            return Err(UiActionError::RequestIdMismatch);
        }
        if envelope.nonce.as_deref() != Some(action.nonce.as_str()) {
            return Err(UiActionError::EnvelopeNonceMismatch);
        }
        if envelope.expires_at_unix_ms != Some(action.expires_at_unix_ms) {
            return Err(UiActionError::EnvelopeExpiryMismatch);
        }
        if action.expires_at_unix_ms <= context.now_unix_ms {
            return Err(UiActionError::Expired);
        }
        if action.page_version != context.expected_page_version {
            return Err(UiActionError::StalePageVersion {
                expected: context.expected_page_version,
                received: action.page_version,
            });
        }
        if action.nonce != context.expected_nonce {
            return Err(UiActionError::InvalidNonce);
        }
        if !context.permission_granted {
            return Err(UiActionError::PermissionDenied);
        }
        if !context.in_range {
            return Err(UiActionError::OutOfRange);
        }
        if !context.state_allowed {
            return Err(UiActionError::InvalidState);
        }

        self.seen_requests
            .retain(|_, expires_at| *expires_at > context.now_unix_ms);
        if self.seen_requests.contains_key(&action.request_id) {
            return Err(UiActionError::DuplicateRequest);
        }

        while self.attempts.front().is_some_and(|timestamp| {
            context.now_unix_ms.saturating_sub(*timestamp) >= self.window_ms
        }) {
            self.attempts.pop_front();
        }
        if self.attempts.len() >= self.max_actions {
            return Err(UiActionError::RateLimited);
        }

        self.attempts.push_back(context.now_unix_ms);
        self.seen_requests
            .insert(action.request_id.clone(), action.expires_at_unix_ms);
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum UiProtocolError {
    #[error("{field} is invalid")]
    InvalidIdentifier { field: &'static str },
    #[error("page version must be greater than zero")]
    InvalidPageVersion,
    #[error("page version must increase from {expected}, received {received}")]
    NonIncreasingPageVersion { expected: u64, received: u64 },
    #[error("UI update contains {actual} fields, maximum is {maximum}")]
    InvalidFieldCount { actual: usize, maximum: usize },
    #[error("UI value nesting exceeds {maximum}")]
    NestingTooDeep { maximum: usize },
    #[error("UI array contains {actual} items, maximum is {maximum}")]
    TooManyArrayItems { actual: usize, maximum: usize },
    #[error("UI string contains {actual} bytes, maximum is {maximum}")]
    StringTooLarge { actual: usize, maximum: usize },
    #[error("UI requirement count exceeds {maximum}")]
    TooManyRequirements { maximum: usize },
    #[error("action expiry is required")]
    MissingExpiry,
    #[error("decimal input must be finite")]
    NonFiniteDecimal,
}

#[derive(Debug, Error)]
pub enum UiActionError {
    #[error(transparent)]
    InvalidAction(#[from] UiProtocolError),
    #[error(transparent)]
    InvalidEnvelope(#[from] ProtocolError),
    #[error("action argument differs from the signed envelope payload")]
    PayloadMismatch,
    #[error("action gate rate limit must be non-zero")]
    InvalidRateLimit,
    #[error("envelope is not ui_action")]
    WrongMessageType,
    #[error("envelope and action request IDs differ")]
    RequestIdMismatch,
    #[error("envelope and action nonces differ")]
    EnvelopeNonceMismatch,
    #[error("envelope and action expiry values differ")]
    EnvelopeExpiryMismatch,
    #[error("action has expired")]
    Expired,
    #[error("page version is stale: expected {expected}, received {received}")]
    StalePageVersion { expected: u64, received: u64 },
    #[error("action nonce does not match the active page")]
    InvalidNonce,
    #[error("server permission check denied the action")]
    PermissionDenied,
    #[error("actor is outside the allowed interaction range")]
    OutOfRange,
    #[error("server state does not allow the action")]
    InvalidState,
    #[error("request ID was already processed")]
    DuplicateRequest,
    #[error("action rate limit exceeded")]
    RateLimited,
}

fn validate_requirements(open: &UiOpen, limits: ProtocolLimits) -> Result<(), UiProtocolError> {
    if open.required_capabilities.len() > limits.max_capabilities
        || open.required_permissions.len() > limits.max_array_items
    {
        return Err(UiProtocolError::TooManyRequirements {
            maximum: limits.max_array_items,
        });
    }
    for permission in &open.required_permissions {
        validate_ui_identifier("permission", permission, 128)?;
    }
    validate_ui_value(&open.model, 1, limits)
}

fn validate_page_version(version: u64) -> Result<(), UiProtocolError> {
    if version == 0 {
        Err(UiProtocolError::InvalidPageVersion)
    } else {
        Ok(())
    }
}

fn validate_input(input: &UiInputValue, limits: ProtocolLimits) -> Result<(), UiProtocolError> {
    match input {
        UiInputValue::Text(value) => validate_string(value, limits),
        UiInputValue::Decimal(value) if !value.is_finite() => {
            Err(UiProtocolError::NonFiniteDecimal)
        }
        UiInputValue::Selection(values) => {
            if values.len() > limits.max_array_items {
                return Err(UiProtocolError::TooManyArrayItems {
                    actual: values.len(),
                    maximum: limits.max_array_items,
                });
            }
            for value in values {
                validate_string(value, limits)?;
            }
            Ok(())
        }
        UiInputValue::Integer(_) | UiInputValue::Decimal(_) | UiInputValue::Boolean(_) => Ok(()),
    }
}

fn validate_ui_value(
    value: &Value,
    depth: usize,
    limits: ProtocolLimits,
) -> Result<(), UiProtocolError> {
    if depth > limits.max_nesting_depth {
        return Err(UiProtocolError::NestingTooDeep {
            maximum: limits.max_nesting_depth,
        });
    }
    match value {
        Value::String(value) => validate_string(value, limits),
        Value::Array(values) => {
            if values.len() > limits.max_array_items {
                return Err(UiProtocolError::TooManyArrayItems {
                    actual: values.len(),
                    maximum: limits.max_array_items,
                });
            }
            for value in values {
                validate_ui_value(value, depth + 1, limits)?;
            }
            Ok(())
        }
        Value::Object(values) => {
            for (key, value) in values {
                validate_ui_identifier("model_field", key, 96)?;
                validate_ui_value(value, depth + 1, limits)?;
            }
            Ok(())
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(()),
    }
}

fn validate_string(value: &str, limits: ProtocolLimits) -> Result<(), UiProtocolError> {
    if value.len() > limits.max_string_bytes {
        Err(UiProtocolError::StringTooLarge {
            actual: value.len(),
            maximum: limits.max_string_bytes,
        })
    } else {
        Ok(())
    }
}

fn validate_ui_identifier(
    field: &'static str,
    value: &str,
    maximum_length: usize,
) -> Result<(), UiProtocolError> {
    let valid = !value.is_empty()
        && value.len() <= maximum_length
        // ArcartX derives page/control IDs from filenames and configuration keys. Those IDs are
        // allowed to be Chinese or otherwise Unicode in real deployments, so the UI protocol
        // rejects control/whitespace characters while retaining the UTF-8 identifier itself.
        && value.chars().all(|character| {
            !character.is_control() && !character.is_whitespace()
        });
    if valid {
        Ok(())
    } else {
        Err(UiProtocolError::InvalidIdentifier { field })
    }
}
