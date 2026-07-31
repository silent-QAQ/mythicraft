use std::collections::{BTreeMap, BTreeSet};

use mythicraft_api::EntityId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ClientCapability;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldPosition {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl WorldPosition {
    fn validate(self) -> Result<(), ExperienceError> {
        if self.x.is_finite()
            && self.y.is_finite()
            && self.z.is_finite()
            && self.x.abs() <= 30_000_000.0
            && self.z.abs() <= 30_000_000.0
            && self.y.abs() <= 4_096.0
        {
            Ok(())
        } else {
            Err(ExperienceError::InvalidPosition)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HudBar {
    pub id: String,
    pub label: String,
    pub current: f32,
    pub maximum: f32,
    pub color: String,
}

impl HudBar {
    fn validate(&self) -> Result<(), ExperienceError> {
        validate_identifier("hud_bar.id", &self.id, 64)?;
        validate_text("hud_bar.label", &self.label, 128)?;
        validate_color(&self.color)?;
        validate_range(self.current, self.maximum, "hud_bar")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HudState {
    pub revision: u64,
    pub health: f32,
    pub max_health: f32,
    pub level: u32,
    pub experience_fraction: f32,
    #[serde(default)]
    pub bars: Vec<HudBar>,
}

impl HudState {
    pub fn validate(&self) -> Result<(), ExperienceError> {
        validate_revision(self.revision)?;
        validate_range(self.health, self.max_health, "health")?;
        if !self.experience_fraction.is_finite() || !(0.0..=1.0).contains(&self.experience_fraction)
        {
            return Err(ExperienceError::InvalidExperienceFraction);
        }
        validate_count("hud.bars", self.bars.len(), 16)?;
        let mut ids = BTreeSet::new();
        for bar in &self.bars {
            bar.validate()?;
            if !ids.insert(&bar.id) {
                return Err(ExperienceError::DuplicateId(bar.id.clone()));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillSlot {
    pub slot: u8,
    pub skill_id: String,
    pub icon_resource_id: String,
    pub remaining_cooldown_ticks: u32,
    pub total_cooldown_ticks: u32,
    pub charges: u16,
    pub max_charges: u16,
}

impl SkillSlot {
    fn validate(&self) -> Result<(), ExperienceError> {
        if self.slot >= 16 {
            return Err(ExperienceError::InvalidSkillSlot(self.slot));
        }
        validate_identifier("skill_id", &self.skill_id, 96)?;
        validate_resource_id(&self.icon_resource_id)?;
        if self.total_cooldown_ticks == 0
            || self.remaining_cooldown_ticks > self.total_cooldown_ticks
        {
            return Err(ExperienceError::InvalidCooldown);
        }
        if self.max_charges == 0 || self.charges > self.max_charges {
            return Err(ExperienceError::InvalidCharges);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillBar {
    pub revision: u64,
    pub slots: Vec<SkillSlot>,
}

impl SkillBar {
    pub fn validate(&self) -> Result<(), ExperienceError> {
        validate_revision(self.revision)?;
        validate_count("skill_bar.slots", self.slots.len(), 16)?;
        let mut slots = BTreeSet::new();
        let mut skills = BTreeSet::new();
        for slot in &self.slots {
            slot.validate()?;
            if !slots.insert(slot.slot) || !skills.insert(&slot.skill_id) {
                return Err(ExperienceError::DuplicateId(slot.skill_id.clone()));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpgHudModel {
    pub hud: HudState,
    pub skill_bar: SkillBar,
}

impl RpgHudModel {
    pub fn validate(&self) -> Result<(), ExperienceError> {
        self.hud.validate()?;
        self.skill_bar.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DamageDisplay {
    pub event_id: String,
    pub target_entity: EntityId,
    pub amount: f32,
    pub damage_type: String,
    pub critical: bool,
    pub duration_ticks: u16,
}

impl DamageDisplay {
    pub fn validate(&self) -> Result<(), ExperienceError> {
        validate_identifier("event_id", &self.event_id, 64)?;
        validate_entity(self.target_entity)?;
        validate_identifier("damage_type", &self.damage_type, 64)?;
        if !self.amount.is_finite() || self.amount <= 0.0 || self.amount > 1_000_000_000.0 {
            return Err(ExperienceError::InvalidDamageAmount(self.amount));
        }
        if !(1..=200).contains(&self.duration_ticks) {
            return Err(ExperienceError::InvalidDuration(self.duration_ticks));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DialogueChoice {
    pub control_id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DialogueModel {
    pub dialogue_id: String,
    pub page_version: u64,
    pub speaker: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portrait_resource_id: Option<String>,
    pub choices: Vec<DialogueChoice>,
    pub nonce: String,
    pub expires_at_unix_ms: u64,
}

impl DialogueModel {
    pub fn validate(&self) -> Result<(), ExperienceError> {
        validate_identifier("dialogue_id", &self.dialogue_id, 96)?;
        validate_revision(self.page_version)?;
        validate_text("dialogue.speaker", &self.speaker, 128)?;
        validate_text("dialogue.text", &self.text, 4_096)?;
        validate_identifier("dialogue.nonce", &self.nonce, 128)?;
        if self.expires_at_unix_ms == 0 {
            return Err(ExperienceError::MissingExpiry);
        }
        if let Some(resource_id) = &self.portrait_resource_id {
            validate_resource_id(resource_id)?;
        }
        validate_count("dialogue.choices", self.choices.len(), 8)?;
        let mut controls = BTreeSet::new();
        for choice in &self.choices {
            validate_identifier("dialogue.control_id", &choice.control_id, 96)?;
            validate_text("dialogue.choice.label", &choice.label, 256)?;
            if !controls.insert(&choice.control_id) {
                return Err(ExperienceError::DuplicateId(choice.control_id.clone()));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BossbarColor {
    Pink,
    Blue,
    Red,
    Green,
    Yellow,
    Purple,
    White,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BossbarState {
    pub boss_entity: EntityId,
    pub revision: u64,
    pub title: String,
    pub current: f32,
    pub maximum: f32,
    pub color: BossbarColor,
    pub visible: bool,
}

impl BossbarState {
    pub fn validate(&self) -> Result<(), ExperienceError> {
        validate_entity(self.boss_entity)?;
        validate_revision(self.revision)?;
        validate_text("bossbar.title", &self.title, 256)?;
        validate_range(self.current, self.maximum, "bossbar")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HologramHealthBar {
    pub hologram_id: String,
    pub entity: EntityId,
    pub revision: u64,
    pub label: String,
    pub current: f32,
    pub maximum: f32,
    pub vertical_offset: f32,
    pub visible: bool,
}

impl HologramHealthBar {
    pub fn validate(&self) -> Result<(), ExperienceError> {
        validate_identifier("hologram_id", &self.hologram_id, 96)?;
        validate_entity(self.entity)?;
        validate_revision(self.revision)?;
        validate_text("hologram.label", &self.label, 256)?;
        validate_range(self.current, self.maximum, "hologram")?;
        if !self.vertical_offset.is_finite() || !(-16.0..=16.0).contains(&self.vertical_offset) {
            return Err(ExperienceError::InvalidVerticalOffset(self.vertical_offset));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaypointState {
    pub waypoint_id: String,
    pub revision: u64,
    pub label: String,
    pub dimension: String,
    pub position: WorldPosition,
    pub icon_resource_id: String,
    pub color: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_unix_ms: Option<u64>,
    pub visible: bool,
}

impl WaypointState {
    pub fn validate(&self) -> Result<(), ExperienceError> {
        validate_identifier("waypoint_id", &self.waypoint_id, 96)?;
        validate_revision(self.revision)?;
        validate_text("waypoint.label", &self.label, 256)?;
        validate_identifier("dimension", &self.dimension, 96)?;
        self.position.validate()?;
        validate_resource_id(&self.icon_resource_id)?;
        validate_color(&self.color)?;
        if self.expires_at_unix_ms == Some(0) {
            return Err(ExperienceError::MissingExpiry);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelVisibility {
    pub entity: EntityId,
    pub revision: u64,
    pub model_resource_id: String,
    pub visible: bool,
    pub reason: String,
}

impl ModelVisibility {
    pub fn validate(&self) -> Result<(), ExperienceError> {
        validate_entity(self.entity)?;
        validate_revision(self.revision)?;
        validate_resource_id(&self.model_resource_id)?;
        validate_identifier("model_visibility.reason", &self.reason, 64)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExperienceDecision {
    Render,
    UseVanillaHud,
    UseVanillaBossbar,
    UseChatDialogue,
    UseDefaultModel,
    HideComponent,
}

pub fn evaluate_experience_capability(
    required: ClientCapability,
    available: &BTreeSet<ClientCapability>,
    resources_available: bool,
) -> ExperienceDecision {
    if available.contains(&required) && resources_available {
        return ExperienceDecision::Render;
    }
    match required {
        ClientCapability::UiHud | ClientCapability::UiSkillBar => ExperienceDecision::UseVanillaHud,
        ClientCapability::UiDialog => ExperienceDecision::UseChatDialogue,
        ClientCapability::UiBossbar => ExperienceDecision::UseVanillaBossbar,
        ClientCapability::ModelVisibility => ExperienceDecision::UseDefaultModel,
        ClientCapability::UiDamageDisplay
        | ClientCapability::UiHologram
        | ClientCapability::UiWaypoint
        | ClientCapability::AudioPlay
        | ClientCapability::InputBind => ExperienceDecision::HideComponent,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionDecision {
    Applied,
    Duplicate,
    Stale,
}

#[derive(Debug, Default)]
pub struct ComponentRevisionGate {
    revisions: BTreeMap<String, u64>,
}

impl ComponentRevisionGate {
    pub fn evaluate_and_record(
        &mut self,
        component_id: &str,
        revision: u64,
    ) -> Result<RevisionDecision, ExperienceError> {
        validate_identifier("component_id", component_id, 128)?;
        validate_revision(revision)?;
        match self.revisions.get(component_id).copied() {
            None => {
                self.revisions.insert(component_id.to_owned(), revision);
                Ok(RevisionDecision::Applied)
            }
            Some(current) if revision > current => {
                self.revisions.insert(component_id.to_owned(), revision);
                Ok(RevisionDecision::Applied)
            }
            Some(current) if revision == current => Ok(RevisionDecision::Duplicate),
            Some(_) => Ok(RevisionDecision::Stale),
        }
    }
}

#[derive(Debug, Error)]
pub enum ExperienceError {
    #[error("{field} is invalid")]
    InvalidIdentifier { field: &'static str },
    #[error("{field} is invalid")]
    InvalidText { field: &'static str },
    #[error("resource ID is invalid")]
    InvalidResourceId,
    #[error("revision must be greater than zero")]
    InvalidRevision,
    #[error("entity ID must be greater than zero")]
    InvalidEntityId,
    #[error("world position is invalid")]
    InvalidPosition,
    #[error("{field} range is invalid")]
    InvalidRange { field: &'static str },
    #[error("experience fraction must be between zero and one")]
    InvalidExperienceFraction,
    #[error("{field} contains {actual} items, maximum is {maximum}")]
    TooManyItems {
        field: &'static str,
        actual: usize,
        maximum: usize,
    },
    #[error("duplicate component ID: {0}")]
    DuplicateId(String),
    #[error("skill slot {0} is outside 0..16")]
    InvalidSkillSlot(u8),
    #[error("cooldown values are invalid")]
    InvalidCooldown,
    #[error("skill charge values are invalid")]
    InvalidCharges,
    #[error("damage amount {0} is invalid")]
    InvalidDamageAmount(f32),
    #[error("duration {0} ticks is invalid")]
    InvalidDuration(u16),
    #[error("expiry must be greater than zero")]
    MissingExpiry,
    #[error("vertical offset {0} is invalid")]
    InvalidVerticalOffset(f32),
    #[error("color must use six lowercase hexadecimal characters")]
    InvalidColor,
}

fn validate_count(
    field: &'static str,
    actual: usize,
    maximum: usize,
) -> Result<(), ExperienceError> {
    if actual <= maximum {
        Ok(())
    } else {
        Err(ExperienceError::TooManyItems {
            field,
            actual,
            maximum,
        })
    }
}

fn validate_revision(revision: u64) -> Result<(), ExperienceError> {
    if revision == 0 {
        Err(ExperienceError::InvalidRevision)
    } else {
        Ok(())
    }
}

fn validate_entity(entity: EntityId) -> Result<(), ExperienceError> {
    if entity.0 == 0 {
        Err(ExperienceError::InvalidEntityId)
    } else {
        Ok(())
    }
}

fn validate_range(current: f32, maximum: f32, field: &'static str) -> Result<(), ExperienceError> {
    if current.is_finite()
        && maximum.is_finite()
        && maximum > 0.0
        && (0.0..=maximum).contains(&current)
    {
        Ok(())
    } else {
        Err(ExperienceError::InvalidRange { field })
    }
}

fn validate_identifier(
    field: &'static str,
    value: &str,
    maximum: usize,
) -> Result<(), ExperienceError> {
    let valid = !value.is_empty()
        && value.len() <= maximum
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':' | b'/')
        });
    if valid {
        Ok(())
    } else {
        Err(ExperienceError::InvalidIdentifier { field })
    }
}

fn validate_resource_id(value: &str) -> Result<(), ExperienceError> {
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
        Err(ExperienceError::InvalidResourceId)
    }
}

fn validate_text(field: &'static str, value: &str, maximum: usize) -> Result<(), ExperienceError> {
    if !value.trim().is_empty() && value.len() <= maximum && !value.chars().any(char::is_control) {
        Ok(())
    } else {
        Err(ExperienceError::InvalidText { field })
    }
}

fn validate_color(value: &str) -> Result<(), ExperienceError> {
    if value.len() == 6
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        Ok(())
    } else {
        Err(ExperienceError::InvalidColor)
    }
}
