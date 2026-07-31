use crate::{
    Condition, Effect, RpgDocument, RpgEntityDefinition, SkillDefinition, TargetSelector, Trigger,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistryDescriptor {
    pub name: String,
    pub version: String,
    pub input_schema: String,
    pub permission: Option<String>,
    pub tick_cost: u32,
    pub unsupported_reason: Option<String>,
}
#[derive(Debug, Clone, Default)]
pub struct RpgRegistry {
    pub mechanics: HashMap<String, RegistryDescriptor>,
    pub targeters: HashMap<String, RegistryDescriptor>,
    pub conditions: HashMap<String, RegistryDescriptor>,
}
impl RpgRegistry {
    pub fn builtin() -> Self {
        let mut registry = Self::default();
        for (name, schema, cost) in [
            ("damage", "amount:number,target:selector", 1),
            ("heal", "amount:number,target:selector", 1),
            ("knockback", "strength:number,target:selector", 2),
            (
                "status",
                "effect:string,duration_ticks:u32,target:selector",
                2,
            ),
        ] {
            registry.mechanics.insert(
                name.into(),
                RegistryDescriptor {
                    name: name.into(),
                    version: "1".into(),
                    input_schema: schema.into(),
                    permission: None,
                    tick_cost: cost,
                    unsupported_reason: None,
                },
            );
        }
        for (name, schema) in [
            ("self", "none"),
            ("nearest_enemy", "radius:number"),
            ("trigger_target", "none"),
            ("explicit", "entity_id:string"),
        ] {
            registry.targeters.insert(
                name.into(),
                RegistryDescriptor {
                    name: name.into(),
                    version: "1".into(),
                    input_schema: schema.into(),
                    permission: None,
                    tick_cost: 1,
                    unsupported_reason: None,
                },
            );
        }
        for (name, schema) in [
            ("always", "none"),
            ("health_below", "threshold:number"),
            ("target_in_range", "radius:number"),
            ("has_permission", "node:string"),
        ] {
            registry.conditions.insert(
                name.into(),
                RegistryDescriptor {
                    name: name.into(),
                    version: "1".into(),
                    input_schema: schema.into(),
                    permission: None,
                    tick_cost: 1,
                    unsupported_reason: None,
                },
            );
        }
        registry
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Position {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}
impl Position {
    fn distance_squared(self, other: Self) -> f64 {
        (self.x - other.x).powi(2) + (self.y - other.y).powi(2) + (self.z - other.z).powi(2)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeEntity {
    pub id: String,
    pub definition_id: String,
    pub health: f64,
    pub max_health: f64,
    pub position: Position,
    pub alive: bool,
    pub statuses: HashMap<String, u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RuntimeEvent {
    Spawned {
        entity: String,
    },
    Damage(crate::DamageEvent),
    Healed {
        source: String,
        target: String,
        amount: f64,
        tick: u64,
    },
    Knockback {
        target: String,
        strength: f64,
        tick: u64,
    },
    Status {
        target: String,
        effect: String,
        duration_ticks: u32,
        tick: u64,
    },
    Death {
        entity: String,
        tick: u64,
    },
    Drop {
        entity: String,
        item: String,
        amount: u32,
        tick: u64,
    },
    Experience {
        entity: String,
        amount: u32,
        tick: u64,
    },
}
#[derive(Debug, Clone, Copy)]
pub struct TickBudget {
    pub max_effects: usize,
    pub max_depth: usize,
    pub max_events: usize,
}
impl Default for TickBudget {
    fn default() -> Self {
        Self {
            max_effects: 64,
            max_depth: 8,
            max_events: 128,
        }
    }
}
#[derive(Debug, Error, PartialEq)]
pub enum RuntimeError {
    #[error("invalid RPG document: {0:?}")]
    InvalidDocument(Vec<crate::ValidationError>),
    #[error("entity definition not found: {0}")]
    UnknownDefinition(String),
    #[error("runtime entity already exists: {0}")]
    EntityAlreadyExists(String),
    #[error("entity not found: {0}")]
    UnknownEntity(String),
    #[error("skill not found: {0}")]
    UnknownSkill(String),
    #[error("skill execution depth exceeded")]
    DepthExceeded,
    #[error("skill effect budget exceeded")]
    EffectBudgetExceeded,
    #[error("event budget exceeded")]
    EventBudgetExceeded,
    #[error("entity is dead: {0}")]
    DeadEntity(String),
}

#[derive(Debug, Clone)]
pub struct RpgRuntime {
    pub document: RpgDocument,
    pub entities: HashMap<String, RuntimeEntity>,
    pub tick: u64,
    pub seed: u64,
    pub events: Vec<RuntimeEvent>,
    pub registry: RpgRegistry,
    pub permissions: HashMap<String, HashSet<String>>,
    skill_cooldowns: HashMap<(String, String), u64>,
}
impl RpgRuntime {
    pub fn new(document: RpgDocument, seed: u64) -> Self {
        Self {
            document,
            entities: HashMap::new(),
            tick: 0,
            seed,
            events: vec![],
            registry: RpgRegistry::builtin(),
            permissions: HashMap::new(),
            skill_cooldowns: HashMap::new(),
        }
    }

    pub fn try_new(document: RpgDocument, seed: u64) -> Result<Self, RuntimeError> {
        let mut runtime = Self::new(RpgDocument::default(), seed);
        runtime.register_document(document)?;
        Ok(runtime)
    }

    pub fn register_document(&mut self, document: RpgDocument) -> Result<(), RuntimeError> {
        document.validate().map_err(RuntimeError::InvalidDocument)?;
        self.document = document;
        self.entities.clear();
        self.skill_cooldowns.clear();
        self.events.clear();
        self.tick = 0;
        Ok(())
    }

    pub fn entity_definition(&self, id: &str) -> Result<&RpgEntityDefinition, RuntimeError> {
        self.definition(id)
    }

    pub fn skill_definition(&self, id: &str) -> Result<&SkillDefinition, RuntimeError> {
        self.find_skill(id)
    }

    pub fn spawn(
        &mut self,
        id: impl Into<String>,
        definition_id: &str,
        position: Position,
    ) -> Result<(), RuntimeError> {
        let id = id.into();
        if self.entities.contains_key(&id) {
            return Err(RuntimeError::EntityAlreadyExists(id));
        }
        let def = self.definition(definition_id)?;
        self.entities.insert(
            id.clone(),
            RuntimeEntity {
                id: id.clone(),
                definition_id: definition_id.into(),
                health: def.health,
                max_health: def.health,
                position,
                alive: true,
                statuses: HashMap::new(),
            },
        );
        self.emit(RuntimeEvent::Spawned { entity: id.clone() })?;
        self.trigger_inner(&id, Trigger::Spawn, None, TickBudget::default(), 0)
    }

    pub fn tick(&mut self, budget: TickBudget) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        let event_start = self.events.len();
        self.tick = self.tick.saturating_add(1);
        self.expire_statuses();

        let sources = self
            .entities
            .iter()
            .filter(|(_, entity)| entity.alive)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for source in sources {
            let definition_id = self
                .entities
                .get(&source)
                .ok_or_else(|| RuntimeError::UnknownEntity(source.clone()))?
                .definition_id
                .clone();
            let timers = self
                .definition(&definition_id)?
                .triggers
                .iter()
                .filter_map(|trigger| match trigger {
                    Trigger::Timer { ticks }
                        if *ticks > 0 && self.tick.is_multiple_of(u64::from(*ticks)) =>
                    {
                        Some(*ticks)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            for ticks in timers {
                self.trigger_inner(&source, Trigger::Timer { ticks }, None, budget, 0)?;
            }
        }

        Ok(self.events[event_start..].to_vec())
    }

    pub fn trigger(
        &mut self,
        source: &str,
        trigger: Trigger,
        trigger_target: Option<&str>,
        budget: TickBudget,
    ) -> Result<(), RuntimeError> {
        self.trigger_inner(source, trigger, trigger_target, budget, 0)
    }

    pub fn target_acquired(
        &mut self,
        source: &str,
        target: &str,
        budget: TickBudget,
    ) -> Result<(), RuntimeError> {
        if !self.entities.contains_key(target) {
            return Err(RuntimeError::UnknownEntity(target.into()));
        }
        self.trigger(source, Trigger::TargetAcquired, Some(target), budget)
    }

    pub fn skill_cooldown_remaining(
        &self,
        source: &str,
        skill_id: &str,
    ) -> Result<u64, RuntimeError> {
        if !self.entities.contains_key(source) {
            return Err(RuntimeError::UnknownEntity(source.into()));
        }
        self.find_skill(skill_id)?;
        Ok(self
            .skill_cooldowns
            .get(&(source.into(), skill_id.into()))
            .copied()
            .unwrap_or(0)
            .saturating_sub(self.tick))
    }
    pub fn execute_skill(
        &mut self,
        source: &str,
        skill_id: &str,
        trigger_target: Option<&str>,
        budget: TickBudget,
    ) -> Result<(), RuntimeError> {
        if !self.entities.contains_key(source) {
            return Err(RuntimeError::UnknownEntity(source.into()));
        }
        if !self.entities.get(source).is_some_and(|entity| entity.alive) {
            return Err(RuntimeError::DeadEntity(source.into()));
        }
        self.execute_skill_inner(source, skill_id, trigger_target, budget, 0)
    }
    fn execute_skill_inner(
        &mut self,
        source: &str,
        skill_id: &str,
        trigger_target: Option<&str>,
        budget: TickBudget,
        depth: usize,
    ) -> Result<(), RuntimeError> {
        if depth >= budget.max_depth {
            return Err(RuntimeError::DepthExceeded);
        }
        let skill = self.find_skill(skill_id)?.clone();
        let cooldown_key = (source.into(), skill_id.into());
        if self
            .skill_cooldowns
            .get(&cooldown_key)
            .is_some_and(|available_at| *available_at > self.tick)
        {
            return Ok(());
        }
        if !self.conditions_pass(source, &skill.conditions, trigger_target)? {
            return Ok(());
        }
        for (index, effect) in skill.effects.iter().enumerate() {
            if index >= budget.max_effects {
                return Err(RuntimeError::EffectBudgetExceeded);
            }
            self.apply_effect(source, effect, trigger_target, budget, depth)?;
        }
        if skill.cooldown_ticks == 0 {
            self.skill_cooldowns.remove(&cooldown_key);
        } else {
            self.skill_cooldowns.insert(
                cooldown_key,
                self.tick.saturating_add(u64::from(skill.cooldown_ticks)),
            );
        }
        Ok(())
    }
    fn apply_effect(
        &mut self,
        source: &str,
        effect: &Effect,
        trigger_target: Option<&str>,
        budget: TickBudget,
        depth: usize,
    ) -> Result<(), RuntimeError> {
        let target = self.select_target(source, target_of(effect), trigger_target)?;
        match effect {
            Effect::Damage { amount, .. } => {
                self.damage_with_budget(source, &target, *amount, budget, depth)
            }
            Effect::Heal { amount, .. } => self.heal_with_budget(source, &target, *amount, budget),
            Effect::Knockback { strength, .. } => self.emit_with_budget(
                RuntimeEvent::Knockback {
                    target,
                    strength: *strength,
                    tick: self.tick,
                },
                budget,
            ),
            Effect::Skill { skill_id, .. } => {
                self.execute_skill_inner(source, skill_id, Some(&target), budget, depth + 1)
            }

            Effect::Status {
                effect,
                duration_ticks,
                ..
            } => {
                if let Some(entity) = self.entities.get_mut(&target) {
                    entity.statuses.insert(effect.clone(), *duration_ticks);
                }
                self.emit_with_budget(
                    RuntimeEvent::Status {
                        target,
                        effect: effect.clone(),
                        duration_ticks: *duration_ticks,
                        tick: self.tick,
                    },
                    budget,
                )
            }
        }
    }
    pub fn damage(&mut self, source: &str, target: &str, amount: f64) -> Result<(), RuntimeError> {
        self.damage_with_budget(source, target, amount, TickBudget::default(), 0)
    }

    fn damage_with_budget(
        &mut self,
        source: &str,
        target: &str,
        amount: f64,
        budget: TickBudget,
        depth: usize,
    ) -> Result<(), RuntimeError> {
        let amount = amount.max(0.0);
        let died = {
            let entity = self
                .entities
                .get_mut(target)
                .ok_or_else(|| RuntimeError::UnknownEntity(target.into()))?;
            if !entity.alive {
                return Err(RuntimeError::DeadEntity(target.into()));
            }
            entity.health = (entity.health - amount).max(0.0);
            if entity.health == 0.0 {
                entity.alive = false;
                true
            } else {
                false
            }
        };
        self.emit_with_budget(
            RuntimeEvent::Damage(crate::DamageEvent {
                source: Some(source.into()),
                target: target.into(),
                amount,
                cause: "skill".into(),
                tick: self.tick,
            }),
            budget,
        )?;
        self.trigger_inner(
            target,
            Trigger::Damaged,
            Some(source),
            budget,
            depth.saturating_add(1),
        )?;
        if died {
            self.emit_with_budget(
                RuntimeEvent::Death {
                    entity: target.into(),
                    tick: self.tick,
                },
                budget,
            )?;
            self.resolve_death(target, budget, depth.saturating_add(1))?;
        }
        Ok(())
    }
    pub fn heal(&mut self, source: &str, target: &str, amount: f64) -> Result<(), RuntimeError> {
        self.heal_with_budget(source, target, amount, TickBudget::default())
    }

    fn heal_with_budget(
        &mut self,
        source: &str,
        target: &str,
        amount: f64,
        budget: TickBudget,
    ) -> Result<(), RuntimeError> {
        let entity = self
            .entities
            .get_mut(target)
            .ok_or_else(|| RuntimeError::UnknownEntity(target.into()))?;
        if !entity.alive {
            return Err(RuntimeError::DeadEntity(target.into()));
        }
        let actual = amount.max(0.0).min(entity.max_health - entity.health);
        entity.health += actual;
        self.emit_with_budget(
            RuntimeEvent::Healed {
                source: source.into(),
                target: target.into(),
                amount: actual,
                tick: self.tick,
            },
            budget,
        )
    }
    fn conditions_pass(
        &self,
        source: &str,
        conditions: &[Condition],
        trigger_target: Option<&str>,
    ) -> Result<bool, RuntimeError> {
        for condition in conditions {
            let passed = match condition {
                Condition::Always => true,
                Condition::HealthBelow(threshold) => self
                    .entities
                    .get(source)
                    .map(|e| e.health < *threshold)
                    .unwrap_or(false),
                Condition::TargetInRange(radius) => trigger_target
                    .and_then(|target| self.entities.get(target))
                    .map(|target| {
                        self.entities[source]
                            .position
                            .distance_squared(target.position)
                            <= radius * radius
                    })
                    .unwrap_or(false),
                Condition::HasPermission(node) => self
                    .permissions
                    .get(source)
                    .map(|nodes| nodes.contains(node))
                    .unwrap_or(false),
            };
            if !passed {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn select_target(
        &self,
        source: &str,
        selector: &TargetSelector,
        trigger_target: Option<&str>,
    ) -> Result<String, RuntimeError> {
        match selector {
            TargetSelector::SelfEntity => Ok(source.into()),
            TargetSelector::TriggerTarget => trigger_target
                .map(String::from)
                .ok_or_else(|| RuntimeError::UnknownEntity("trigger target".into())),
            TargetSelector::Explicit(id) => Ok(id.clone()),
            TargetSelector::NearestEnemy { radius } => self
                .entities
                .iter()
                .filter(|(id, e)| {
                    id.as_str() != source
                        && e.alive
                        && self.entities[source].position.distance_squared(e.position)
                            <= radius * radius
                })
                .min_by(|a, b| {
                    self.entities[source]
                        .position
                        .distance_squared(a.1.position)
                        .total_cmp(
                            &self.entities[source]
                                .position
                                .distance_squared(b.1.position),
                        )
                })
                .map(|(id, _)| id.clone())
                .ok_or_else(|| RuntimeError::UnknownEntity("nearest enemy".into())),
        }
    }
    fn resolve_death(
        &mut self,
        id: &str,
        budget: TickBudget,
        depth: usize,
    ) -> Result<(), RuntimeError> {
        let definition_id = self
            .entities
            .get(id)
            .ok_or_else(|| RuntimeError::UnknownEntity(id.into()))?
            .definition_id
            .clone();
        let def = self.definition(&definition_id)?.clone();
        if let Some(table_id) = def.loot_table {
            let entries = self
                .document
                .loot_tables
                .iter()
                .find(|t| t.id == table_id)
                .map(|t| t.entries.clone())
                .unwrap_or_default();
            for entry in entries {
                if self.next_random() < entry.chance {
                    let span = entry.max.saturating_sub(entry.min).saturating_add(1);
                    let amount = entry.min + (self.next_random_u32() % span.max(1));
                    self.emit_with_budget(
                        RuntimeEvent::Drop {
                            entity: id.into(),
                            item: entry.item,
                            amount,
                            tick: self.tick,
                        },
                        budget,
                    )?;
                }
            }
        }
        self.emit_with_budget(
            RuntimeEvent::Experience {
                entity: id.into(),
                amount: def.experience,
                tick: self.tick,
            },
            budget,
        )?;
        self.trigger_inner(id, Trigger::Death, None, budget, depth.saturating_add(1))
    }
    fn next_random_u32(&mut self) -> u32 {
        self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        (self.seed >> 32) as u32
    }
    fn next_random(&mut self) -> f64 {
        self.next_random_u32() as f64 / u32::MAX as f64
    }
    fn definition(&self, id: &str) -> Result<&RpgEntityDefinition, RuntimeError> {
        self.document
            .entities
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| RuntimeError::UnknownDefinition(id.into()))
    }
    fn find_skill(&self, id: &str) -> Result<&SkillDefinition, RuntimeError> {
        self.document
            .entities
            .iter()
            .flat_map(|e| e.skills.iter())
            .find(|s| s.id == id)
            .ok_or_else(|| RuntimeError::UnknownSkill(id.into()))
    }

    fn trigger_inner(
        &mut self,
        source: &str,
        trigger: Trigger,
        trigger_target: Option<&str>,
        budget: TickBudget,
        depth: usize,
    ) -> Result<(), RuntimeError> {
        if depth >= budget.max_depth {
            return Err(RuntimeError::DepthExceeded);
        }
        let definition_id = self
            .entities
            .get(source)
            .ok_or_else(|| RuntimeError::UnknownEntity(source.into()))?
            .definition_id
            .clone();
        let definition = self.definition(&definition_id)?;
        if !definition
            .triggers
            .iter()
            .any(|candidate| candidate == &trigger)
        {
            return Ok(());
        }
        let skill_ids = definition
            .skills
            .iter()
            .map(|skill| skill.id.clone())
            .collect::<Vec<_>>();
        for skill_id in skill_ids {
            self.execute_skill_inner(source, &skill_id, trigger_target, budget, depth)?;
        }
        Ok(())
    }

    fn expire_statuses(&mut self) {
        for entity in self.entities.values_mut() {
            entity.statuses.retain(|_, remaining| {
                *remaining = remaining.saturating_sub(1);
                *remaining > 0
            });
        }
    }

    fn emit_with_budget(
        &mut self,
        event: RuntimeEvent,
        budget: TickBudget,
    ) -> Result<(), RuntimeError> {
        if self.events.len() >= budget.max_events {
            return Err(RuntimeError::EventBudgetExceeded);
        }
        self.events.push(event);
        Ok(())
    }

    fn emit(&mut self, event: RuntimeEvent) -> Result<(), RuntimeError> {
        self.emit_with_budget(event, TickBudget::default())
    }
}
fn target_of(effect: &Effect) -> &TargetSelector {
    match effect {
        Effect::Damage { target, .. }
        | Effect::Heal { target, .. }
        | Effect::Knockback { target, .. }
        | Effect::Status { target, .. }
        | Effect::Skill { target, .. } => target,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Effect, EntityOptions, LootEntry, LootTable, RpgEntityDefinition, SkillDefinition,
        TargetSelector,
    };
    fn fixture() -> RpgDocument {
        let mut doc = RpgDocument::default();
        doc.loot_tables.push(LootTable {
            id: "goblin-drops".into(),
            entries: vec![LootEntry {
                item: "gold".into(),
                chance: 1.0,
                min: 2,
                max: 2,
            }],
        });
        doc.entities.push(RpgEntityDefinition {
            id: "Goblin".into(),
            display: "Goblin".into(),
            entity_type: "ZOMBIE".into(),
            health: 10.0,
            damage: 2.0,
            attributes: vec![],
            equipment: vec![],
            options: EntityOptions::default(),
            triggers: vec![],
            skills: vec![SkillDefinition {
                id: "smash".into(),
                conditions: vec![],
                effects: vec![Effect::Damage {
                    amount: 20.0,
                    target: TargetSelector::Explicit("Goblin-1".into()),
                }],
                cooldown_ticks: 0,
            }],
            loot_table: Some("goblin-drops".into()),
            experience: 25,
        });
        doc.entities.push(RpgEntityDefinition {
            id: "PlayerDef".into(),
            display: "Player".into(),
            entity_type: "PLAYER".into(),
            health: 20.0,
            damage: 1.0,
            attributes: vec![],
            equipment: vec![],
            options: EntityOptions::default(),
            triggers: vec![],
            skills: vec![],
            loot_table: None,
            experience: 0,
        });
        doc
    }
    #[test]
    fn combat_loop_is_deterministic_and_emits_rewards() {
        let mut runtime = RpgRuntime::new(fixture(), 7);
        runtime
            .spawn(
                "Goblin-1",
                "Goblin",
                Position {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
            )
            .unwrap();
        runtime
            .spawn(
                "Player",
                "PlayerDef",
                Position {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            )
            .unwrap();
        runtime
            .execute_skill("Player", "smash", None, TickBudget::default())
            .unwrap();
        assert!(!runtime.entities["Goblin-1"].alive);
        assert!(runtime.events.iter().any(
            |e| matches!(e,RuntimeEvent::Drop { item, amount, .. } if item=="gold" && *amount==2)
        ));
        assert!(runtime
            .events
            .iter()
            .any(|e| matches!(e, RuntimeEvent::Experience { amount: 25, .. })));
    }

    fn timer_fixture() -> RpgDocument {
        let mut doc = RpgDocument::default();
        doc.entities.push(RpgEntityDefinition {
            id: "Caster".into(),
            display: "Caster".into(),
            entity_type: "ZOMBIE".into(),
            health: 10.0,
            damage: 0.0,
            attributes: vec![],
            equipment: vec![],
            options: EntityOptions::default(),
            triggers: vec![crate::Trigger::Timer { ticks: 2 }],
            skills: vec![SkillDefinition {
                id: "pulse".into(),
                conditions: vec![crate::Condition::Always],
                effects: vec![Effect::Damage {
                    amount: 3.0,
                    target: TargetSelector::Explicit("Target".into()),
                }],
                cooldown_ticks: 3,
            }],
            loot_table: None,
            experience: 0,
        });
        doc.entities.push(RpgEntityDefinition {
            id: "Target".into(),
            display: "Target".into(),
            entity_type: "PLAYER".into(),
            health: 20.0,
            damage: 0.0,
            attributes: vec![],
            equipment: vec![],
            options: EntityOptions::default(),
            triggers: vec![],
            skills: vec![],
            loot_table: None,
            experience: 0,
        });
        doc
    }

    #[test]
    fn imported_document_registers_and_timer_tick_executes_with_cooldown() {
        let mut runtime = RpgRuntime::try_new(timer_fixture(), 7).unwrap();
        runtime
            .spawn(
                "Caster-1",
                "Caster",
                Position {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
            )
            .unwrap();
        runtime
            .spawn(
                "Target",
                "Target",
                Position {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            )
            .unwrap();

        assert_eq!(runtime.entity_definition("Caster").unwrap().id, "Caster");
        assert_eq!(runtime.skill_definition("pulse").unwrap().id, "pulse");
        assert!(runtime.tick(TickBudget::default()).unwrap().is_empty());
        let events = runtime.tick(TickBudget::default()).unwrap();
        assert!(events.iter().any(|event| {
            matches!(event, RuntimeEvent::Damage(damage) if damage.target == "Target")
        }));
        assert_eq!(runtime.skill_cooldown_remaining("Caster-1", "pulse"), Ok(3));

        runtime.tick(TickBudget::default()).unwrap();
        let events = runtime.tick(TickBudget::default()).unwrap();
        assert!(events.is_empty());
        assert_eq!(runtime.entities["Target"].health, 17.0);
        runtime.tick(TickBudget::default()).unwrap();
        let events = runtime.tick(TickBudget::default()).unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            RuntimeEvent::Damage(crate::DamageEvent { target, .. }) if target == "Target"
        )));
    }

    #[test]
    fn status_effects_are_maintained_by_tick() {
        let mut doc = RpgDocument::default();
        doc.entities.push(RpgEntityDefinition {
            id: "Player".into(),
            display: "Player".into(),
            entity_type: "PLAYER".into(),
            health: 20.0,
            damage: 0.0,
            attributes: vec![],
            equipment: vec![],
            options: EntityOptions::default(),
            triggers: vec![],
            skills: vec![SkillDefinition {
                id: "mark".into(),
                conditions: vec![],
                effects: vec![Effect::Status {
                    effect: "marked".into(),
                    duration_ticks: 2,
                    target: TargetSelector::SelfEntity,
                }],
                cooldown_ticks: 0,
            }],
            loot_table: None,
            experience: 0,
        });
        let mut runtime = RpgRuntime::new(doc, 1);
        runtime
            .spawn(
                "Player-1",
                "Player",
                Position {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
            )
            .unwrap();
        runtime
            .execute_skill("Player-1", "mark", None, TickBudget::default())
            .unwrap();
        assert_eq!(runtime.entities["Player-1"].statuses["marked"], 2);
        runtime.tick(TickBudget::default()).unwrap();
        assert_eq!(runtime.entities["Player-1"].statuses["marked"], 1);
        runtime.tick(TickBudget::default()).unwrap();
        assert!(!runtime.entities["Player-1"].statuses.contains_key("marked"));
    }
}
