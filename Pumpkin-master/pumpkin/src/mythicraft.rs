use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::command::CommandSender;
use mythicraft_arcartx::{
    ActionDefinition, ActionType as ArcartxActionType, ArcartxDocument, DiagnosticSeverity,
    DocumentKind, parse_auto,
};
use mythicraft_client_services::{
    CapabilityPolicy, ClientCapability, MessageType, PayloadEnvelope, ProtocolDispatcher,
    ProtocolLimits, ProtocolMessage, UiActionContext, UiActionGate, UiActionType, UiOpen, UiRun,
    UiUpdate,
};
use mythicraft_compat::{
    ImportStatus, import_luckperms_engine, import_mythicmobs, import_vault_economy,
};
use mythicraft_economy::Economy;
use mythicraft_permission::PermissionEngine;
use mythicraft_persistence::{
    PlayerState as SavedPlayerState, Position as SavedPosition, SaveStore,
};
use mythicraft_rpg::runtime::{Position as RpgPosition, RuntimeError, RuntimeEvent, TickBudget};
use mythicraft_rpg::{RpgDocument, RpgRuntime};
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::potion::Effect;
use pumpkin_data::{damage::DamageType, entity::EntityType};
use pumpkin_util::math::vector3::Vector3;
use thiserror::Error;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::entity::experience_orb::ExperienceOrbEntity;
use crate::entity::player::Player;
use crate::entity::{EntityBase, r#type::from_type};
use crate::server::Server;

#[derive(Debug, Error)]
pub enum MythicraftCoreError {
    #[error("RPG runtime error: {0}")]
    Runtime(#[from] RuntimeError),
    #[error("world not found: {0}")]
    WorldNotFound(String),
    #[error("unsupported Pumpkin entity type: {0}")]
    UnsupportedEntityType(String),
    #[error("entity binding not found: {0}")]
    EntityBindingNotFound(String),
}

#[derive(Debug, Clone, Copy)]
struct EntityBinding {
    pumpkin_uuid: Uuid,
}

/// The channel used by the Mythicraft client Mod.
pub const CLIENT_CHANNEL: &str = "mythicraft:main";

struct PlayerState {
    accepted_capabilities: BTreeSet<ClientCapability>,
    active_pages: HashMap<String, ActiveUiPage>,
    ui_gate: UiActionGate,
}

#[derive(Debug, Clone)]
struct ActiveUiPage {
    version: u64,
    nonce: String,
    required_capabilities: Vec<ClientCapability>,
    required_permissions: Vec<String>,
}

impl PlayerState {
    fn new(accepted_capabilities: BTreeSet<ClientCapability>) -> Self {
        Self {
            accepted_capabilities,
            active_pages: HashMap::new(),
            ui_gate: UiActionGate::new(20, 1_000)
                .expect("fixed UI action gate configuration is valid"),
        }
    }
}

/// RPG state owned by the Pumpkin server, rather than by a separate plugin process.
///
/// This is intentionally the first host-side integration seam: Minecraft player and
/// tick callbacks enter here, while the domain crates remain independently testable.
pub struct MythicraftCore {
    dispatcher: ProtocolDispatcher,
    policy: CapabilityPolicy,
    players: Mutex<HashMap<Uuid, PlayerState>>,
    entity_bindings: Mutex<HashMap<String, EntityBinding>>,
    persistence: Option<Arc<SaveStore>>,
    pub rpg: Mutex<RpgRuntime>,
    pub economy: Mutex<Economy>,
    pub permissions: Mutex<PermissionEngine>,
    arcartx_documents: Arc<Vec<ArcartxDocument>>,
}

impl MythicraftCore {
    #[must_use]
    pub fn new() -> Self {
        Self::with_document(RpgDocument::default(), 0x4D59_5448)
    }

    #[must_use]
    pub fn load_from_root(root: &Path, seed: u64) -> Self {
        let mut document = RpgDocument::default();
        let mut files = 0;
        let mut diagnostics = 0;
        let mut visited_dirs = HashSet::new();

        for relative_dir in [
            "plugins/MythicMobs/Mobs",
            "plugins/MythicMobs/mobs",
            "MythicMobs/Mobs",
        ] {
            let directory = root.join(relative_dir);
            let directory_key = directory.to_string_lossy().to_ascii_lowercase();
            if !visited_dirs.insert(directory_key) {
                continue;
            }
            let Ok(entries) = fs::read_dir(&directory) else {
                continue;
            };
            let mut paths = entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.is_file()
                        && path.extension().is_some_and(|extension| {
                            extension.eq_ignore_ascii_case("yml")
                                || extension.eq_ignore_ascii_case("yaml")
                        })
                })
                .collect::<Vec<_>>();
            paths.sort();

            for path in paths {
                let source = match fs::read_to_string(&path) {
                    Ok(source) => source,
                    Err(error) => {
                        warn!(file = %path.display(), %error, "Failed to read MythicMobs file");
                        diagnostics += 1;
                        continue;
                    }
                };
                let file = path.display().to_string();
                let report = match import_mythicmobs(&file, &source, false) {
                    Ok(report) => report,
                    Err(error) => {
                        warn!(file = %file, %error, "Failed to parse MythicMobs file");
                        diagnostics += 1;
                        continue;
                    }
                };
                files += 1;
                diagnostics += report.diagnostics.len();
                for diagnostic in &report.diagnostics {
                    match &diagnostic.severity {
                        mythicraft_compat::Severity::Error => {
                            warn!(
                                file = %diagnostic.source.file,
                                path = %diagnostic.source.path,
                                code = %diagnostic.code,
                                "MythicMobs import error: {}",
                                diagnostic.message
                            );
                        }
                        mythicraft_compat::Severity::Warning => {
                            warn!(
                                file = %diagnostic.source.file,
                                path = %diagnostic.source.path,
                                code = %diagnostic.code,
                                "MythicMobs import warning: {}",
                                diagnostic.message
                            );
                        }
                        mythicraft_compat::Severity::Info => debug!(
                            file = %diagnostic.source.file,
                            path = %diagnostic.source.path,
                            code = %diagnostic.code,
                            "MythicMobs import info: {}",
                            diagnostic.message
                        ),
                    }
                }
                if let Some(mut imported) = report.document {
                    document.entities.append(&mut imported.entities);
                    document.items.append(&mut imported.items);
                    document.loot_tables.append(&mut imported.loot_tables);
                    document.dialogs.append(&mut imported.dialogs);
                }
                if matches!(report.status, ImportStatus::Invalid) {
                    warn!(file = %file, "MythicMobs file was not fully valid; supported definitions were retained with diagnostics");
                }
            }
        }

        if let Err(errors) = document.validate() {
            for error in errors {
                warn!(%error, "Merged Mythicraft RPG document is invalid");
            }
        }
        info!(
            files,
            entities = document.entities.len(),
            diagnostics,
            "Loaded Mythicraft RPG definitions"
        );
        let arcartx_documents = load_arcartx_documents(root);
        let (economy, permissions) = load_compatibility_state(root);
        let persistence = match SaveStore::open(root.join("mythicraft-data")) {
            Ok(store) => Some(Arc::new(store)),
            Err(error) => {
                warn!(%error, "Native Mythicraft persistence is unavailable; continuing without player saves");
                None
            }
        };
        Self::with_document_and_persistence(
            document,
            seed,
            persistence,
            economy,
            permissions,
            arcartx_documents,
        )
    }

    fn with_document(document: RpgDocument, seed: u64) -> Self {
        Self::with_document_and_persistence(
            document,
            seed,
            None,
            Economy::default(),
            PermissionEngine::default(),
            Arc::new(Vec::new()),
        )
    }

    fn with_document_and_persistence(
        document: RpgDocument,
        seed: u64,
        persistence: Option<Arc<SaveStore>>,
        economy: Economy,
        permissions: PermissionEngine,
        arcartx_documents: Arc<Vec<ArcartxDocument>>,
    ) -> Self {
        let supported = BTreeSet::from([
            ClientCapability::UiHud,
            ClientCapability::UiDamageDisplay,
            ClientCapability::UiSkillBar,
            ClientCapability::UiDialog,
            ClientCapability::UiBossbar,
            ClientCapability::UiHologram,
            ClientCapability::UiWaypoint,
            ClientCapability::AudioPlay,
            ClientCapability::ModelVisibility,
            ClientCapability::InputBind,
        ]);
        let required = BTreeSet::from([ClientCapability::UiHud]);

        Self {
            dispatcher: ProtocolDispatcher::default(),
            policy: CapabilityPolicy {
                protocol_version: 1,
                supported,
                required,
            },
            players: Mutex::new(HashMap::new()),
            entity_bindings: Mutex::new(HashMap::new()),
            persistence,
            rpg: Mutex::new(RpgRuntime::new(document, seed)),
            economy: Mutex::new(economy),
            permissions: Mutex::new(permissions),
            arcartx_documents,
        }
    }

    pub async fn on_player_join(&self, player: &Arc<Player>) {
        self.players
            .lock()
            .await
            .insert(player.gameprofile.id, PlayerState::new(BTreeSet::new()));
        info!(
            player = %player.gameprofile.name,
            uuid = %player.gameprofile.id,
            "Mythicraft core registered player"
        );
        self.hydrate_player(player).await;
    }

    pub async fn on_player_leave(&self, player: &Player) {
        self.persist_player(player).await;
        self.players.lock().await.remove(&player.gameprofile.id);
        info!(
            player = %player.gameprofile.name,
            uuid = %player.gameprofile.id,
            "Mythicraft core removed player"
        );
    }

    async fn hydrate_player(&self, player: &Arc<Player>) {
        let Some(store) = self.persistence.clone() else {
            return;
        };
        let player_id = player.gameprofile.id;
        let id = player_id.to_string();
        let loaded = tokio::task::spawn_blocking(move || store.load(&id)).await;
        let loaded = match loaded {
            Ok(Ok(loaded)) => loaded,
            Ok(Err(mythicraft_persistence::PersistenceError::NotFound(_))) => return,
            Ok(Err(error)) => {
                warn!(player = %player_id, %error, "Failed to load native Mythicraft player save");
                return;
            }
            Err(error) => {
                warn!(player = %player_id, %error, "Player save task failed");
                return;
            }
        };
        if let Err(error) = self
            .economy
            .lock()
            .await
            .restore_balance(player_id, loaded.state.economy_balance)
        {
            warn!(player = %player_id, %error, "Rejected persisted economy balance");
            return;
        }
        let saved_world = loaded
            .state
            .position
            .world
            .strip_prefix("minecraft:")
            .unwrap_or(&loaded.state.position.world)
            .to_ascii_lowercase();
        if saved_world == player.world().get_world_name().to_ascii_lowercase() {
            let position = Vector3::new(
                loaded.state.position.x,
                loaded.state.position.y,
                loaded.state.position.z,
            );
            player
                .request_teleport(
                    position,
                    loaded.state.position.yaw,
                    loaded.state.position.pitch,
                )
                .await;
        } else {
            warn!(
                player = %player_id,
                saved_world = %saved_world,
                current_world = player.world().get_world_name(),
                "Player save world differs from join world; position restore skipped"
            );
        }
        info!(player = %player_id, revision = loaded.revision, "Hydrated native Mythicraft player state");
    }

    async fn persist_player(&self, player: &Player) {
        let Some(store) = self.persistence.clone() else {
            return;
        };
        let position = player.position();
        let world = player.world();
        let balance = self.economy.lock().await.balance(player.gameprofile.id);
        let player_id = player.gameprofile.id;
        let state_id = player_id.to_string();
        let existing = match tokio::task::spawn_blocking({
            let store = store.clone();
            let state_id = state_id.clone();
            move || store.load(&state_id)
        })
        .await
        {
            Ok(Ok(loaded)) => loaded.state,
            Ok(Err(mythicraft_persistence::PersistenceError::NotFound(_))) => {
                SavedPlayerState::new(state_id.clone())
            }
            Ok(Err(error)) => {
                warn!(
                    player = %player_id,
                    %error,
                    "Failed to read existing player state before persist"
                );
                return;
            }
            Err(error) => {
                warn!(player = %player_id, %error, "Player state read task failed");
                return;
            }
        };
        let mut state = existing;
        state.player_id = state_id;
        state.position = SavedPosition {
            world: world.get_world_name().to_ascii_lowercase(),
            x: position.x,
            y: position.y,
            z: position.z,
            yaw: player.get_entity().yaw.load(),
            pitch: player.get_entity().pitch.load(),
        };
        state.economy_balance = balance;
        let result = tokio::task::spawn_blocking(move || store.save(&state, None)).await;
        match result {
            Ok(Ok(revision)) => {
                info!(player = %player_id, revision, "Persisted native Mythicraft player state")
            }
            Ok(Err(error)) => {
                warn!(player = %player_id, %error, "Failed to persist native Mythicraft player state")
            }
            Err(error) => warn!(player = %player_id, %error, "Player save task failed"),
        }
    }

    /// Runs after Pumpkin's world/player tick and before the tick is reported complete.
    pub async fn tick(&self, server: &Server, tick: i32, duration_nanos: i64) {
        let events = {
            let mut rpg = self.rpg.lock().await;
            let events = match rpg.tick(TickBudget::default()) {
                Ok(_) => std::mem::take(&mut rpg.events),
                Err(error) => {
                    warn!(%error, pumpkin_tick = tick, "Mythicraft RPG tick rejected an event");
                    rpg.events.clear();
                    Vec::new()
                }
            };
            debug!(
                pumpkin_tick = tick,
                rpg_tick = rpg.tick,
                duration_nanos,
                entities = rpg.entities.len(),
                "Mythicraft RPG core tick"
            );
            events
        };
        self.sync_bound_entities(server).await;
        self.commit_runtime_events(server, &events).await;
        if tick % 20 == 0 {
            self.push_hud_updates(server, tick).await;
        }
    }

    /// Spawns an imported RPG definition as a native Pumpkin entity.
    ///
    /// The RPG runtime owns the semantic entity ID while Pumpkin owns the
    /// network/entity lifecycle. The binding is created before the first
    /// spawn packet is sent so subsequent skill events can target the native
    /// entity without a second shadow entity system.
    pub async fn spawn_definition(
        &self,
        server: &Server,
        world_name: Option<&str>,
        definition_id: &str,
        position: RpgPosition,
    ) -> Result<String, MythicraftCoreError> {
        let world = server
            .worlds
            .load()
            .iter()
            .find(|world| world_name.is_none_or(|name| world.get_world_name() == name))
            .cloned()
            .ok_or_else(|| {
                MythicraftCoreError::WorldNotFound(world_name.unwrap_or("default world").to_owned())
            })?;

        let (entity_type_name, max_health, invincible, display) = {
            let rpg = self.rpg.lock().await;
            let definition = rpg
                .document
                .entities
                .iter()
                .find(|definition| definition.id == definition_id)
                .cloned()
                .ok_or_else(|| RuntimeError::UnknownDefinition(definition_id.to_owned()))?;
            (
                definition.entity_type,
                definition.health,
                definition.options.invincible,
                definition.display,
            )
        };

        let entity_type_name = entity_type_name
            .strip_prefix("minecraft:")
            .unwrap_or(&entity_type_name)
            .to_ascii_lowercase();
        let entity_type = EntityType::from_name(&entity_type_name)
            .ok_or_else(|| MythicraftCoreError::UnsupportedEntityType(entity_type_name.clone()))?;
        let runtime_id = Uuid::new_v4().to_string();
        let spawn_events = {
            let mut rpg = self.rpg.lock().await;
            rpg.spawn(runtime_id.clone(), definition_id, position)?;
            std::mem::take(&mut rpg.events)
        };
        let pumpkin_uuid = Uuid::parse_str(&runtime_id)
            .map_err(|_| MythicraftCoreError::EntityBindingNotFound(runtime_id.clone()))?;
        let entity = from_type(
            entity_type,
            Vector3::new(position.x, position.y, position.z),
            &world,
            pumpkin_uuid,
        );
        entity
            .get_entity()
            .invulnerable
            .store(invincible, std::sync::atomic::Ordering::Relaxed);
        if let Some(living) = entity.get_living_entity() {
            living.set_max_health(max_health as f32).await;
            living.set_health(max_health as f32);
        }
        entity.get_entity().custom_name.store(Arc::new(Some(
            pumpkin_util::text::TextComponent::text(display),
        )));
        entity
            .get_entity()
            .custom_name_visible
            .store(true, std::sync::atomic::Ordering::Relaxed);

        world.spawn_entity(entity).await;
        self.entity_bindings
            .lock()
            .await
            .insert(runtime_id.clone(), EntityBinding { pumpkin_uuid });
        self.commit_runtime_events(server, &spawn_events).await;
        info!(definition = %definition_id, runtime_id = %runtime_id, "Spawned native Mythicraft RPG entity");
        Ok(runtime_id)
    }

    /// Executes a skill in the RPG runtime and commits damage events into
    /// Pumpkin's native living-entity damage path.
    pub async fn execute_skill(
        &self,
        server: &Server,
        source: &str,
        skill_id: &str,
        trigger_target: Option<&str>,
    ) -> Result<usize, MythicraftCoreError> {
        let events = {
            let mut rpg = self.rpg.lock().await;
            rpg.execute_skill(source, skill_id, trigger_target, TickBudget::default())?;
            std::mem::take(&mut rpg.events)
        };
        let event_count = events.len();
        self.commit_runtime_events(server, &events).await;
        Ok(event_count)
    }

    async fn sync_bound_entities(&self, server: &Server) {
        let bindings = self.entity_bindings.lock().await.clone();
        if bindings.is_empty() {
            return;
        }
        let mut snapshots = HashMap::new();
        for world in server.worlds.load().iter() {
            for entity in world.entities.load().iter() {
                let base = entity.get_entity();
                let position = base.pos.load();
                let health = entity
                    .get_living_entity()
                    .map(|living| f64::from(living.health.load()));
                snapshots.insert(base.entity_uuid, (position, health));
            }
        }
        let mut rpg = self.rpg.lock().await;
        for (runtime_id, binding) in bindings {
            let Some((position, health)) = snapshots.get(&binding.pumpkin_uuid) else {
                continue;
            };
            let Some(entity) = rpg.entities.get_mut(&runtime_id) else {
                continue;
            };
            entity.position = RpgPosition {
                x: position.x,
                y: position.y,
                z: position.z,
            };
            if let Some(health) = health {
                entity.health = *health;
                entity.alive = *health > 0.0;
            }
        }
    }

    async fn commit_runtime_events(&self, server: &Server, events: &[RuntimeEvent]) {
        for event in events {
            match event {
                RuntimeEvent::Damage(damage) => {
                    let Some(target) = self.find_bound_entity(server, &damage.target).await else {
                        warn!(target = %damage.target, "RPG damage target has no native Pumpkin binding");
                        continue;
                    };
                    let source = match damage.source.as_deref() {
                        Some(source) => self.find_bound_entity(server, source).await,
                        None => None,
                    };
                    let _ = target
                        .damage_with_context(
                            target.as_ref(),
                            damage.amount.max(0.0) as f32,
                            DamageType::MAGIC,
                            None,
                            source.as_deref(),
                            source.as_deref(),
                        )
                        .await;
                }
                RuntimeEvent::Healed { target, amount, .. } => {
                    let Some(target) = self.find_bound_entity(server, target).await else {
                        warn!(target = %target, "RPG heal target has no native Pumpkin binding");
                        continue;
                    };
                    if *amount > 0.0 {
                        if let Some(living) = target.get_living_entity() {
                            living.heal(*amount as f32);
                        }
                    }
                }
                RuntimeEvent::Knockback {
                    target, strength, ..
                } => {
                    let Some(target) = self.find_bound_entity(server, target).await else {
                        warn!(target = %target, "RPG knockback target has no native Pumpkin binding");
                        continue;
                    };
                    let strength = strength.max(0.0);
                    let velocity = target.get_entity().velocity.load();
                    target
                        .get_entity()
                        .velocity
                        .store(velocity + Vector3::new(0.0, strength, 0.0));
                }
                RuntimeEvent::Status {
                    target,
                    effect,
                    duration_ticks,
                    ..
                } => {
                    let Some(target) = self.find_bound_entity(server, target).await else {
                        warn!(target = %target, "RPG status target has no native Pumpkin binding");
                        continue;
                    };
                    let effect_name = effect.strip_prefix("minecraft:").unwrap_or(effect);
                    let Some(effect_type) = StatusEffect::from_name(effect_name) else {
                        warn!(effect = %effect, "RPG status effect is not present in Pumpkin registry");
                        continue;
                    };
                    if let Some(living) = target.get_living_entity() {
                        living
                            .add_effect(Effect {
                                effect_type,
                                duration: (*duration_ticks).min(i32::MAX as u32) as i32,
                                amplifier: 0,
                                ambient: false,
                                show_particles: true,
                                show_icon: true,
                                blend: false,
                            })
                            .await;
                    }
                }
                RuntimeEvent::Drop {
                    entity,
                    item,
                    amount,
                    ..
                } => {
                    let Some(target) = self.find_bound_entity(server, entity).await else {
                        warn!(entity = %entity, "RPG drop source has no native Pumpkin binding");
                        continue;
                    };
                    let item_name = item.strip_prefix("minecraft:").unwrap_or(item);
                    let Some(item) = Item::from_registry_key(item_name) else {
                        warn!(item = %item, "RPG drop item is not present in Pumpkin registry");
                        continue;
                    };
                    let world = target.get_entity().world.load();
                    let position = target.get_entity().block_pos.load();
                    let mut remaining = *amount;
                    while remaining > 0 {
                        let count = remaining.min(u8::MAX as u32) as u8;
                        world
                            .drop_stack(&position, ItemStack::new(count, item))
                            .await;
                        remaining -= u32::from(count);
                    }
                }
                RuntimeEvent::Experience { entity, amount, .. } => {
                    let Some(target) = self.find_bound_entity(server, entity).await else {
                        warn!(entity = %entity, "RPG experience source has no native Pumpkin binding");
                        continue;
                    };
                    if *amount > 0 {
                        let world = target.get_entity().world.load();
                        ExperienceOrbEntity::spawn(&world, target.get_entity().pos.load(), *amount)
                            .await;
                    }
                }
                RuntimeEvent::Death { entity, .. } => {
                    if let Some(target) = self.find_bound_entity(server, entity).await {
                        target.kill(target.as_ref()).await;
                    }
                }
                RuntimeEvent::Spawned { .. } => {}
            }
        }
    }

    async fn find_bound_entity(
        &self,
        server: &Server,
        runtime_id: &str,
    ) -> Option<Arc<dyn EntityBase>> {
        let binding = self.entity_bindings.lock().await.get(runtime_id).copied()?;
        for world in server.worlds.load().iter() {
            if let Some(entity) = world
                .entities
                .load()
                .iter()
                .find(|entity| entity.get_entity().entity_uuid == binding.pumpkin_uuid)
            {
                return Some(entity.clone());
            }
        }
        None
    }

    /// Opens one ArcartX-compatible UI document through the native Mythicraft UI protocol.
    ///
    /// The source YAML remains the renderer model. This keeps ArcartX's original field names
    /// (`ui`, `root_control`, `attribute`, `action`, ...) available to the client Mod while the
    /// server still owns page version, nonce and permission validation.
    pub async fn open_arcartx_page(&self, player: &Arc<Player>, page_id: &str) -> bool {
        let Some(document) = self
            .arcartx_documents
            .iter()
            .find(|document| document.page_id == page_id)
            .cloned()
        else {
            return false;
        };
        self.open_arcartx_document(player, &document).await
    }

    async fn open_arcartx_default_pages(&self, player: &Arc<Player>) {
        let documents = self
            .arcartx_documents
            .iter()
            .filter(|document| {
                matches!(document.kind, DocumentKind::Page | DocumentKind::Tooltip)
                    && arcartx_is_hud(document)
                    && arcartx_default_open(document)
            })
            .cloned()
            .collect::<Vec<_>>();
        for document in documents {
            self.open_arcartx_document(player, &document).await;
        }
    }

    async fn open_arcartx_document(
        &self,
        player: &Arc<Player>,
        document: &ArcartxDocument,
    ) -> bool {
        let required_capabilities = arcartx_capabilities(document);
        let player_id = player.gameprofile.id;
        let nonce = Uuid::new_v4().to_string();
        let mut model = document.raw_model.clone();
        if let serde_json::Value::Object(object) = &mut model {
            object
                .entry("id".to_owned())
                .or_insert_with(|| serde_json::Value::String(document.page_id.clone()));
        }
        let open = UiOpen {
            page_id: document.page_id.clone(),
            page_version: document.version,
            model,
            required_capabilities: required_capabilities.clone(),
            required_permissions: document.permissions.clone(),
        };
        let envelope = match PayloadEnvelope::new(
            MessageType::UiOpen,
            Uuid::new_v4().to_string(),
            Some(nonce.clone()),
            None,
            &open,
        ) {
            Ok(envelope) => envelope,
            Err(error) => {
                warn!(
                    player = %player.gameprofile.name,
                    page = %document.page_id,
                    %error,
                    "Failed to create ArcartX-compatible UI open message"
                );
                return false;
            }
        };
        let bytes = match envelope.encode(ProtocolLimits::default()) {
            Ok(bytes) => bytes,
            Err(error) => {
                warn!(
                    player = %player.gameprofile.name,
                    page = %document.page_id,
                    %error,
                    "Failed to encode ArcartX-compatible UI open message"
                );
                return false;
            }
        };

        let mut players = self.players.lock().await;
        let Some(state) = players.get_mut(&player_id) else {
            return false;
        };
        if required_capabilities
            .iter()
            .any(|capability| !state.accepted_capabilities.contains(capability))
        {
            warn!(
                player = %player.gameprofile.name,
                page = %document.page_id,
                "Skipped ArcartX-compatible UI because the client lacks a required capability"
            );
            return false;
        }
        state.active_pages.insert(
            document.page_id.clone(),
            ActiveUiPage {
                version: document.version,
                nonce,
                required_capabilities,
                required_permissions: document.permissions.clone(),
            },
        );
        drop(players);
        player.send_custom_payload(CLIENT_CHANNEL, &bytes).await;
        true
    }

    async fn open_hud(&self, player: &Arc<Player>) {
        let page_id = "hud".to_owned();
        let nonce = Uuid::new_v4().to_string();
        let open = UiOpen {
            page_id: page_id.clone(),
            page_version: 1,
            model: serde_json::json!({
                "title": "Mythicraft",
                "currency": "coins",
                "action_format": "skill:<runtime_entity_uuid>:<skill_id>",
                "server_authoritative": true,
            }),
            required_capabilities: vec![ClientCapability::UiHud],
            required_permissions: vec!["mythicraft.ui.action".to_owned()],
        };
        let envelope = match PayloadEnvelope::new(
            MessageType::UiOpen,
            Uuid::new_v4().to_string(),
            Some(nonce.clone()),
            None,
            &open,
        ) {
            Ok(envelope) => envelope,
            Err(error) => {
                warn!(player = %player.gameprofile.name, %error, "Failed to create Mythicraft HUD open message");
                return;
            }
        };
        let bytes = match envelope.encode(ProtocolLimits::default()) {
            Ok(bytes) => bytes,
            Err(error) => {
                warn!(player = %player.gameprofile.name, %error, "Failed to encode Mythicraft HUD open message");
                return;
            }
        };

        let player_id = player.gameprofile.id;
        let mut players = self.players.lock().await;
        let Some(state) = players.get_mut(&player_id) else {
            return;
        };
        if !state
            .accepted_capabilities
            .contains(&ClientCapability::UiHud)
        {
            return;
        }
        state.active_pages.insert(
            page_id,
            ActiveUiPage {
                version: 1,
                nonce,
                required_capabilities: vec![ClientCapability::UiHud],
                required_permissions: vec!["mythicraft.ui.action".to_owned()],
            },
        );
        drop(players);
        player.send_custom_payload(CLIENT_CHANNEL, &bytes).await;
    }

    async fn push_hud_updates(&self, server: &Server, tick: i32) {
        let entity_count = self.rpg.lock().await.entities.len();
        for player in server.get_all_players() {
            let player_id = player.gameprofile.id;
            let balance = self.economy.lock().await.balance(player_id);
            let (envelope, bytes) = {
                let mut players = self.players.lock().await;
                let Some(state) = players.get_mut(&player_id) else {
                    continue;
                };
                if !state
                    .accepted_capabilities
                    .contains(&ClientCapability::UiHud)
                {
                    continue;
                }
                let Some(page) = state.active_pages.get_mut("hud") else {
                    continue;
                };
                let expected_page_version = page.version;
                let page_version = expected_page_version.saturating_add(1);
                let update = UiUpdate {
                    page_id: "hud".to_owned(),
                    expected_page_version,
                    page_version,
                    fields: BTreeMap::from([
                        ("tick".to_owned(), serde_json::json!(tick)),
                        ("rpg_entities".to_owned(), serde_json::json!(entity_count)),
                        ("balance".to_owned(), serde_json::json!(balance)),
                    ]),
                };
                let envelope = match PayloadEnvelope::new(
                    MessageType::UiUpdate,
                    Uuid::new_v4().to_string(),
                    None,
                    None,
                    &update,
                ) {
                    Ok(envelope) => envelope,
                    Err(error) => {
                        warn!(player = %player.gameprofile.name, %error, "Failed to create Mythicraft HUD update message");
                        continue;
                    }
                };
                let bytes = match envelope.encode(ProtocolLimits::default()) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        warn!(player = %player.gameprofile.name, %error, "Failed to encode Mythicraft HUD update message");
                        continue;
                    }
                };
                page.version = page_version;
                (envelope, bytes)
            };
            debug!(player = %player.gameprofile.name, request_id = %envelope.request_id, "Sending Mythicraft HUD update");
            player.send_custom_payload(CLIENT_CHANNEL, &bytes).await;
        }
    }

    /// Executes the safe native subset of an ArcartX UI action.
    ///
    /// `console:` and `player:` actions are explicit command bridges. Any other action body is
    /// treated as client-side Aria/UI code and sent through `ui_run`; arbitrary action text is
    /// never interpreted as a server command. This keeps old ArcartX UI files useful without
    /// giving a client-controlled payload command execution privileges.
    async fn execute_arcartx_action(
        &self,
        server: &Arc<Server>,
        player: &Arc<Player>,
        page_id: &str,
        action: &ActionDefinition,
        page: &ActiveUiPage,
    ) {
        let command = action.command.trim();
        if command.is_empty() {
            warn!(
                player = %player.gameprofile.name,
                page = %action.control_id,
                "Ignored empty ArcartX UI action"
            );
            return;
        }

        let (mode, body) = if let Some(body) = command.strip_prefix("console:") {
            (Some("console"), body.trim())
        } else if let Some(body) = command.strip_prefix("player:") {
            (Some("player"), body.trim())
        } else if let Some(body) = command.strip_prefix("command:") {
            (Some("console"), body.trim())
        } else {
            (None, command)
        };

        if let Some(mode) = mode {
            let command = body
                .trim_start_matches('/')
                .replace("<player>", &player.gameprofile.name)
                .replace("<uuid>", &player.gameprofile.id.to_string());
            if command.is_empty() || command.contains('\r') || command.contains('\n') {
                warn!(
                    player = %player.gameprofile.name,
                    control = %action.control_id,
                    "Rejected malformed ArcartX server command action"
                );
                return;
            }
            let source = if mode == "player" {
                player.get_command_source(server).await
            } else {
                CommandSender::Console.into_source(server).await
            };
            server
                .command_dispatcher
                .read()
                .await
                .handle_command(&source, &command)
                .await;
            return;
        }

        let run = UiRun {
            page_id: page_id.to_owned(),
            page_version: page.version,
            code: body.to_owned(),
        };
        let envelope = match PayloadEnvelope::new(
            MessageType::UiRun,
            Uuid::new_v4().to_string(),
            Some(page.nonce.clone()),
            None,
            &run,
        ) {
            Ok(envelope) => envelope,
            Err(error) => {
                warn!(player = %player.gameprofile.name, %error, "Rejected ArcartX client action");
                return;
            }
        };
        let bytes = match envelope.encode(ProtocolLimits::default()) {
            Ok(bytes) => bytes,
            Err(error) => {
                warn!(player = %player.gameprofile.name, %error, "Rejected oversized ArcartX client action");
                return;
            }
        };
        player.send_custom_payload(CLIENT_CHANNEL, &bytes).await;
    }

    /// Handles the client Mod payload at the core boundary.
    pub async fn handle_custom_payload(
        &self,
        server: &Arc<Server>,
        player: Arc<Player>,
        channel: &str,
        data: &[u8],
    ) {
        if channel != CLIENT_CHANNEL {
            return;
        }

        let envelope = match PayloadEnvelope::decode(data, ProtocolLimits::default()) {
            Ok(envelope) => envelope,
            Err(error) => {
                warn!(
                    player = %player.gameprofile.name,
                    error = %error,
                    "Rejected invalid Mythicraft client envelope"
                );
                return;
            }
        };
        let message = match self.dispatcher.route(envelope.clone()) {
            Ok(message) => message,
            Err(error) => {
                warn!(
                    player = %player.gameprofile.name,
                    error = %error,
                    "Rejected invalid Mythicraft client payload"
                );
                return;
            }
        };

        match message {
            ProtocolMessage::Hello(hello) => {
                let response = match self.policy.negotiate(&hello, ProtocolLimits::default()) {
                    Ok(response) => response,
                    Err(error) => {
                        warn!(
                            player = %player.gameprofile.name,
                            error = %error,
                            "Rejected Mythicraft capability hello"
                        );
                        return;
                    }
                };

                let player_id = player.gameprofile.id;
                self.players
                    .lock()
                    .await
                    .insert(player_id, PlayerState::new(response.accepted.clone()));

                let envelope = match PayloadEnvelope::new(
                    MessageType::Capabilities,
                    player_id.to_string(),
                    None,
                    None,
                    &response,
                ) {
                    Ok(envelope) => envelope,
                    Err(error) => {
                        warn!(%error, "Failed to create Mythicraft capability response");
                        return;
                    }
                };
                match envelope.encode(ProtocolLimits::default()) {
                    Ok(bytes) => player.send_custom_payload(CLIENT_CHANNEL, &bytes).await,
                    Err(error) => warn!(%error, "Failed to encode Mythicraft capability response"),
                }
                if response.rpg_play_allowed {
                    self.open_hud(&player).await;
                    self.open_arcartx_default_pages(&player).await;
                }
            }
            ProtocolMessage::UiAction(action) => {
                let player_id = player.gameprofile.id;
                let tick = self.rpg.lock().await.tick;
                let balance = self.economy.lock().await.balance(player_id);
                let arcartx_action = find_arcartx_action(
                    self.arcartx_documents.as_ref(),
                    &action.page_id,
                    &action.control_id,
                    action.action_type,
                )
                .cloned();
                let compat_permission = self.permissions.lock().await.check(
                    player_id,
                    "mythicraft.ui.action",
                    tick,
                    &HashMap::new(),
                );
                let native_permission = player
                    .has_permission(server.as_ref(), "mythicraft:command.rpg")
                    .await;
                let page_permissions = {
                    let players = self.players.lock().await;
                    players
                        .get(&player_id)
                        .and_then(|state| state.active_pages.get(&action.page_id))
                        .map(|page| page.required_permissions.clone())
                        .unwrap_or_default()
                };
                let page_permission = if page_permissions.is_empty() {
                    true
                } else {
                    let engine = self.permissions.lock().await;
                    let engine_permission = page_permissions.iter().all(|permission| {
                        engine.check(player_id, permission, tick, &HashMap::new())
                    });
                    drop(engine);
                    let mut native_page_permission = true;
                    for permission in &page_permissions {
                        if !player.has_permission(server.as_ref(), permission).await {
                            native_page_permission = false;
                            break;
                        }
                    }
                    engine_permission || native_page_permission
                };
                let configured_permission = if let Some(configured) = &arcartx_action {
                    if configured.permissions.is_empty() {
                        true
                    } else {
                        let engine = self.permissions.lock().await;
                        let engine_permission = !configured.permissions.is_empty()
                            && configured.permissions.iter().all(|permission| {
                                engine.check(player_id, permission, tick, &HashMap::new())
                            });
                        drop(engine);
                        let mut native_permission = true;
                        for permission in &configured.permissions {
                            if !player.has_permission(server.as_ref(), permission).await {
                                native_permission = false;
                                break;
                            }
                        }
                        engine_permission || native_permission
                    }
                } else {
                    false
                };
                let permission = if arcartx_action.is_some() {
                    page_permission && (compat_permission || configured_permission)
                } else {
                    page_permission && (compat_permission || native_permission)
                };
                let now_unix_ms = unix_time_ms();
                let action_result = {
                    let mut players = self.players.lock().await;
                    let Some(state) = players.get_mut(&player_id) else {
                        warn!(player = %player.gameprofile.name, "Ignored UI action before player session registration");
                        return;
                    };
                    let Some(page) = state.active_pages.get(&action.page_id).cloned() else {
                        warn!(player = %player.gameprofile.name, page = %action.page_id, "Rejected UI action for inactive page");
                        return;
                    };
                    if page
                        .required_capabilities
                        .iter()
                        .any(|capability| !state.accepted_capabilities.contains(capability))
                    {
                        warn!(
                            player = %player.gameprofile.name,
                            page = %action.page_id,
                            "Ignored UI action because the client lacks the page capability"
                        );
                        return;
                    }
                    let state_allowed = action.control_id.starts_with("skill:")
                        || arcartx_action
                            .as_ref()
                            .is_some_and(|configured| !configured.command.is_empty());
                    let action_result = state.ui_gate.validate_and_record(
                        &envelope,
                        &action,
                        UiActionContext {
                            now_unix_ms,
                            expected_page_version: page.version,
                            expected_nonce: &page.nonce,
                            permission_granted: permission,
                            in_range: true,
                            state_allowed,
                        },
                        ProtocolLimits::default(),
                    );
                    (action_result, page)
                };
                let (action_result, active_page) = action_result;
                if let Err(error) = action_result {
                    warn!(player = %player.gameprofile.name, %error, "Rejected Mythicraft UI action");
                    return;
                }
                debug!(
                    player = %player.gameprofile.name,
                    page = %action.page_id,
                    control = %action.control_id,
                    balance,
                    permission,
                    "Received syntactically valid Mythicraft UI action"
                );
                if let Some(skill_spec) = action.control_id.strip_prefix("skill:") {
                    let Some((source, skill_id)) = skill_spec.split_once(':') else {
                        warn!(player = %player.gameprofile.name, control = %action.control_id, "Rejected malformed Mythicraft skill control");
                        return;
                    };
                    if let Err(error) = self
                        .execute_skill(server.as_ref(), source, skill_id, None)
                        .await
                    {
                        warn!(player = %player.gameprofile.name, %error, "Mythicraft UI skill execution failed");
                    }
                }
                if let Some(configured) = arcartx_action {
                    self.execute_arcartx_action(
                        server,
                        &player,
                        &action.page_id,
                        &configured,
                        &active_page,
                    )
                    .await;
                    info!(
                        player = %player.gameprofile.name,
                        page = %action.page_id,
                        control = %action.control_id,
                        command = %configured.command,
                        "Accepted and dispatched ArcartX UI action through the native action bridge"
                    );
                }
            }
            ProtocolMessage::UiRun(_) => {
                warn!(
                    player = %player.gameprofile.name,
                    "Rejected client-originated ui_run message"
                );
            }
            other => debug!(
                player = %player.gameprofile.name,
                message = ?other,
                "Received Mythicraft payload"
            ),
        }
    }
}

fn load_arcartx_documents(root: &Path) -> Arc<Vec<ArcartxDocument>> {
    let mut directories = HashSet::new();
    let mut paths = Vec::new();
    for relative in [
        "plugins/ArcartX/ui",
        "plugins/ArcartX/UI",
        "plugins/ArcartX/tooltip",
        "plugins/ArcartX/Tooltip",
        "plugins/ArcartX/assets/ui",
        "plugins/ArcartX/assets/tooltip",
        "config/arcartx/ui",
        "config/arcartx/tooltip",
        "config/ArcartX/ui",
        "config/ArcartX/tooltip",
        "config/arcartx",
        "arcartx/ui",
        "arcartx/tooltip",
        "arcartx",
    ] {
        let directory = root.join(relative);
        let key = directory.to_string_lossy().to_ascii_lowercase();
        if directories.insert(key) {
            collect_arcartx_files(&directory, &mut paths);
        }
    }
    paths.sort();

    let mut documents = Vec::new();
    let mut page_ids = HashSet::new();
    for path in paths {
        let source = match fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) => {
                warn!(file = %path.display(), %error, "Failed to read ArcartX configuration");
                continue;
            }
        };
        let source_id = path.to_string_lossy();
        let report = match parse_auto(&source, Some(source_id.as_ref())) {
            Ok(report) => report,
            Err(error) => {
                warn!(file = %path.display(), %error, "Failed to parse ArcartX configuration");
                continue;
            }
        };
        for diagnostic in &report.diagnostics {
            match diagnostic.severity {
                DiagnosticSeverity::Error => warn!(
                    file = %path.display(),
                    path = %diagnostic.path,
                    code = %diagnostic.code,
                    "ArcartX configuration error: {}",
                    diagnostic.message
                ),
                DiagnosticSeverity::Warning => warn!(
                    file = %path.display(),
                    path = %diagnostic.path,
                    code = %diagnostic.code,
                    "ArcartX configuration warning: {}",
                    diagnostic.message
                ),
            }
        }
        let document = report.document;
        if !page_ids.insert(document.page_id.clone()) {
            warn!(
                file = %path.display(),
                page = %document.page_id,
                "Duplicate ArcartX page ID; keeping the first definition"
            );
            continue;
        }
        documents.push(document);
    }
    info!(
        pages = documents.len(),
        "Loaded ArcartX-compatible native UI definitions"
    );
    Arc::new(documents)
}

fn collect_arcartx_files(directory: &Path, output: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_arcartx_files(&path, output);
        } else if path.is_file()
            && path.extension().is_some_and(|extension| {
                extension.eq_ignore_ascii_case("yml")
                    || extension.eq_ignore_ascii_case("yaml")
                    || extension.eq_ignore_ascii_case("json")
            })
        {
            output.push(path);
        }
    }
}

fn arcartx_is_hud(document: &ArcartxDocument) -> bool {
    document
        .page
        .ui
        .values
        .get("isHud")
        .or_else(|| document.page.ui.values.get("is_hud"))
        .is_some_and(value_is_true)
}

fn arcartx_default_open(document: &ArcartxDocument) -> bool {
    document
        .page
        .ui
        .values
        .get("defaultOpen")
        .or_else(|| document.page.ui.values.get("default_open"))
        .map(value_is_true)
        .unwrap_or(true)
}

fn value_is_true(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Bool(value) => *value,
        serde_json::Value::String(value) => value.eq_ignore_ascii_case("true"),
        serde_json::Value::Number(value) => value.as_u64() == Some(1),
        _ => false,
    }
}

fn arcartx_capabilities(document: &ArcartxDocument) -> Vec<ClientCapability> {
    let mut capabilities = Vec::new();
    for name in &document.required_capabilities {
        if let Some(capability) = capability_from_name(name) {
            if !capabilities.contains(&capability) {
                capabilities.push(capability);
            }
        }
    }
    if capabilities.is_empty() {
        capabilities.push(if arcartx_is_hud(document) {
            ClientCapability::UiHud
        } else {
            ClientCapability::UiDialog
        });
    }
    capabilities
}

fn capability_from_name(name: &str) -> Option<ClientCapability> {
    match name
        .trim()
        .to_ascii_lowercase()
        .replace(['-', ':'], "_")
        .as_str()
    {
        "ui_hud" | "hud" => Some(ClientCapability::UiHud),
        "ui_damage_display" | "damage_display" => Some(ClientCapability::UiDamageDisplay),
        "ui_skill_bar" | "skill_bar" => Some(ClientCapability::UiSkillBar),
        "ui_dialog" | "dialog" | "menu" => Some(ClientCapability::UiDialog),
        "ui_bossbar" | "bossbar" => Some(ClientCapability::UiBossbar),
        "ui_hologram" | "hologram" => Some(ClientCapability::UiHologram),
        "ui_waypoint" | "waypoint" => Some(ClientCapability::UiWaypoint),
        "audio_play" | "audio" => Some(ClientCapability::AudioPlay),
        "model_visibility" | "model" => Some(ClientCapability::ModelVisibility),
        "input_bind" | "input" => Some(ClientCapability::InputBind),
        _ => None,
    }
}

fn find_arcartx_action(
    documents: &[ArcartxDocument],
    page_id: &str,
    control_id: &str,
    action_type: UiActionType,
) -> Option<&ActionDefinition> {
    documents
        .iter()
        .find(|document| document.page_id == page_id)?
        .actions
        .iter()
        .find(|action| {
            action.control_id == control_id && arcartx_action_type_matches(action, action_type)
        })
}

fn arcartx_action_type_matches(action: &ActionDefinition, action_type: UiActionType) -> bool {
    matches!(
        (action.action_type, action_type),
        (ArcartxActionType::Click, UiActionType::Click)
            | (ArcartxActionType::Submit, UiActionType::Submit)
            | (ArcartxActionType::Change, UiActionType::Change)
            | (ArcartxActionType::KeyPress, UiActionType::KeyPress)
    )
}

fn load_compatibility_state(root: &Path) -> (Economy, PermissionEngine) {
    let mut economy = Economy::default();
    for relative in [
        "plugins/Vault/config.yml",
        "plugins/Vault/config.yaml",
        "plugins/VaultUnlocked/config.yml",
        "plugins/VaultUnlocked/config.yaml",
        "config/vault.yml",
    ] {
        let path = root.join(relative);
        let Ok(source) = fs::read_to_string(&path) else {
            continue;
        };
        match import_vault_economy(&path.display().to_string(), &source) {
            Ok((candidate, report)) => {
                info!(
                    file = %path.display(),
                    status = ?report.status,
                    currency = %candidate.currency_name(),
                    "Loaded native economy from Vault-compatible configuration"
                );
                economy = candidate;
                break;
            }
            Err(error) => {
                warn!(file = %path.display(), %error, "Failed to import Vault-compatible economy configuration")
            }
        }
    }

    let mut permissions = PermissionEngine::default();
    for relative in [
        "plugins/LuckPerms/mythicraft.yml",
        "plugins/LuckPerms/permissions.yml",
        "plugins/LuckPerms/config.yml",
        "config/luckperms.yml",
    ] {
        let path = root.join(relative);
        let Ok(source) = fs::read_to_string(&path) else {
            continue;
        };
        match import_luckperms_engine(&path.display().to_string(), &source) {
            Ok((candidate, report)) => {
                info!(
                    file = %path.display(),
                    status = ?report.status,
                    groups = candidate.groups.len(),
                    users = candidate.users.len(),
                    "Loaded native permissions from LuckPerms-compatible configuration"
                );
                permissions = candidate;
                break;
            }
            Err(error) => {
                warn!(file = %path.display(), %error, "Failed to import LuckPerms-compatible configuration")
            }
        }
    }
    (economy, permissions)
}

impl Default for MythicraftCore {
    fn default() -> Self {
        Self::new()
    }
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}
