pub mod runtime;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const RPG_IR_VERSION: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RpgDocument {
    pub ir_version: u16,
    pub entities: Vec<RpgEntityDefinition>,
    pub items: Vec<ItemDefinition>,
    pub loot_tables: Vec<LootTable>,
    pub dialogs: Vec<DialogDefinition>,
}
impl Default for RpgDocument {
    fn default() -> Self {
        Self {
            ir_version: RPG_IR_VERSION,
            entities: vec![],
            items: vec![],
            loot_tables: vec![],
            dialogs: vec![],
        }
    }
}
impl RpgDocument {
    pub fn validate(&self) -> Result<(), Vec<ValidationError>> {
        let mut errors = vec![];
        if self.ir_version != RPG_IR_VERSION {
            errors.push(ValidationError::UnsupportedVersion(self.ir_version));
        }
        for e in &self.entities {
            if e.id.trim().is_empty() {
                errors.push(ValidationError::EmptyId("entity".into()));
            }
            if e.health <= 0.0 {
                errors.push(ValidationError::NonPositive("health".into()));
            }
            if e.damage < 0.0 {
                errors.push(ValidationError::NonPositive("damage".into()));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
    pub fn content_hash(&self) -> String {
        let bytes = serde_json::to_vec(self).expect("RPG IR is serializable");
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}
#[derive(Debug, Error, Clone, PartialEq)]
pub enum ValidationError {
    #[error("unsupported RPG IR version: {0}")]
    UnsupportedVersion(u16),
    #[error("empty {0} id")]
    EmptyId(String),
    #[error("{0} must be positive")]
    NonPositive(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RpgEntityDefinition {
    pub id: String,
    pub display: String,
    pub entity_type: String,
    pub health: f64,
    pub damage: f64,
    pub attributes: Vec<AttributeDefinition>,
    pub equipment: Vec<String>,
    pub options: EntityOptions,
    pub triggers: Vec<Trigger>,
    pub skills: Vec<SkillDefinition>,
    pub loot_table: Option<String>,
    pub experience: u32,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct EntityOptions {
    pub movement_speed: Option<f64>,
    pub prevent_other_drops: bool,
    pub invincible: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AttributeDefinition {
    pub name: String,
    pub base: f64,
    pub maximum: Option<f64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillDefinition {
    pub id: String,
    pub conditions: Vec<Condition>,
    pub effects: Vec<Effect>,
    pub cooldown_ticks: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Trigger {
    Spawn,
    Death,
    Timer { ticks: u32 },
    Damaged,
    TargetAcquired,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TargetSelector {
    SelfEntity,
    NearestEnemy { radius: f64 },
    TriggerTarget,
    Explicit(String),
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Condition {
    Always,
    HasPermission(String),
    HealthBelow(f64),
    TargetInRange(f64),
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Effect {
    Damage {
        amount: f64,
        target: TargetSelector,
    },
    Heal {
        amount: f64,
        target: TargetSelector,
    },
    Knockback {
        strength: f64,
        target: TargetSelector,
    },
    Status {
        effect: String,
        duration_ticks: u32,
        target: TargetSelector,
    },
    Skill {
        skill_id: String,
        target: TargetSelector,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DamageEvent {
    pub source: Option<String>,
    pub target: String,
    pub amount: f64,
    pub cause: String,
    pub tick: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ItemDefinition {
    pub id: String,
    pub material: String,
    pub amount: u32,
    pub metadata: serde_json::Value,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LootTable {
    pub id: String,
    pub entries: Vec<LootEntry>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LootEntry {
    pub item: String,
    pub chance: f64,
    pub min: u32,
    pub max: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DialogDefinition {
    pub id: String,
    pub lines: Vec<String>,
    pub actions: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hash_is_stable_and_validation_works() {
        let mut doc = RpgDocument::default();
        doc.entities.push(RpgEntityDefinition {
            id: "Goblin".into(),
            display: "Goblin".into(),
            entity_type: "ZOMBIE".into(),
            health: 20.0,
            damage: 3.0,
            attributes: vec![],
            equipment: vec![],
            options: Default::default(),
            triggers: vec![Trigger::Spawn],
            skills: vec![],
            loot_table: None,
            experience: 5,
        });
        assert!(doc.validate().is_ok());
        assert_eq!(doc.content_hash(), doc.content_hash());
    }
}
