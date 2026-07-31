use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::model::{ActionDefinition, ActionType, ArcartxDocument};

/// DTO matching `mythicraft_client_services::UiOpen` without linking that excluded crate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiOpenDto {
    pub page_id: String,
    pub page_version: u64,
    pub model: Value,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    #[serde(default)]
    pub required_permissions: Vec<String>,
}

/// DTO matching `mythicraft_client_services::UiUpdate` without linking that excluded crate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiUpdateDto {
    pub page_id: String,
    pub expected_page_version: u64,
    pub page_version: u64,
    pub fields: BTreeMap<String, Value>,
}

/// DTO matching `mythicraft_client_services::UiAction` without linking that excluded crate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiActionDto {
    pub page_id: String,
    pub control_id: String,
    pub action_type: UiActionTypeDto,
    pub page_version: u64,
    pub nonce: String,
    pub expires_at_unix_ms: u64,
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<UiActionInputDto>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiActionTypeDto {
    Click,
    Submit,
    Change,
    KeyPress,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum UiActionInputDto {
    Text(String),
    Integer(i64),
    Decimal(f64),
    Boolean(bool),
    Selection(Vec<String>),
}

/// Values supplied by the server-side action dispatcher, not by an ArcartX file.
///
/// In particular, `nonce` should be freshly bound to the active page. A configured nonce is only
/// a compatibility fallback and must not be treated as an authorization mechanism by itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionEnvelopeContext {
    pub request_id: String,
    pub nonce: Option<String>,
    pub expires_at_unix_ms: u64,
    #[serde(default)]
    pub input: Option<UiActionInputDto>,
}

#[derive(Debug, Error)]
pub enum ConversionError {
    #[error("action `{0}` was not found")]
    ActionNotFound(String),
    #[error("action `{0}` has no nonce; inject a page nonce before creating UiAction")]
    MissingNonce(String),
    #[error("page version must be greater than zero")]
    InvalidPageVersion,
    #[error("action expiry must be greater than zero")]
    InvalidExpiry,
    #[error("request_id must not be empty")]
    EmptyRequestId,
    #[error("page_id must not be empty")]
    EmptyPageId,
    #[error("UI update must contain at least one field")]
    EmptyUpdateFields,
}

impl ArcartxDocument {
    pub fn to_ui_open_dto(&self) -> Result<UiOpenDto, ConversionError> {
        if self.page_id.is_empty() {
            return Err(ConversionError::EmptyPageId);
        }
        if self.version == 0 {
            return Err(ConversionError::InvalidPageVersion);
        }

        let model = self.raw_model.clone();
        Ok(UiOpenDto {
            page_id: self.page_id.clone(),
            page_version: self.version,
            model,
            required_capabilities: self.required_capabilities.clone(),
            required_permissions: self.permissions.clone(),
        })
    }

    pub fn to_ui_update_dto(
        &self,
        expected_page_version: u64,
        page_version: u64,
        fields: BTreeMap<String, Value>,
    ) -> Result<UiUpdateDto, ConversionError> {
        if self.page_id.is_empty() {
            return Err(ConversionError::EmptyPageId);
        }
        if expected_page_version == 0 || page_version == 0 || page_version <= expected_page_version
        {
            return Err(ConversionError::InvalidPageVersion);
        }
        if fields.is_empty() {
            return Err(ConversionError::EmptyUpdateFields);
        }
        Ok(UiUpdateDto {
            page_id: self.page_id.clone(),
            expected_page_version,
            page_version,
            fields,
        })
    }

    pub fn to_ui_action_dto(
        &self,
        action_id: &str,
        context: ActionEnvelopeContext,
    ) -> Result<UiActionDto, ConversionError> {
        let action = self
            .actions
            .iter()
            .find(|candidate| candidate.id == action_id)
            .ok_or_else(|| ConversionError::ActionNotFound(action_id.to_owned()))?;
        action_to_dto(self, action, context)
    }
}

fn action_to_dto(
    document: &ArcartxDocument,
    action: &ActionDefinition,
    context: ActionEnvelopeContext,
) -> Result<UiActionDto, ConversionError> {
    if document.page_id.is_empty() {
        return Err(ConversionError::EmptyPageId);
    }
    if document.version == 0 {
        return Err(ConversionError::InvalidPageVersion);
    }
    if context.request_id.is_empty() {
        return Err(ConversionError::EmptyRequestId);
    }
    if context.expires_at_unix_ms == 0 {
        return Err(ConversionError::InvalidExpiry);
    }
    let nonce = context
        .nonce
        .or_else(|| action.nonce.clone())
        .or_else(|| document.nonce.clone())
        .ok_or_else(|| ConversionError::MissingNonce(action.id.clone()))?;

    Ok(UiActionDto {
        page_id: document.page_id.clone(),
        control_id: action.control_id.clone(),
        action_type: action.action_type.into(),
        page_version: document.version,
        nonce,
        expires_at_unix_ms: context.expires_at_unix_ms,
        request_id: context.request_id,
        input: context.input,
    })
}

impl From<ActionType> for UiActionTypeDto {
    fn from(value: ActionType) -> Self {
        match value {
            ActionType::Click => Self::Click,
            ActionType::Submit => Self::Submit,
            ActionType::Change => Self::Change,
            ActionType::KeyPress => Self::KeyPress,
        }
    }
}
