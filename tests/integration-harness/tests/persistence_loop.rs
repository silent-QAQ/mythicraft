use flate2::{write::GzEncoder, Compression};
use mythicraft_client_services::{
    AssetLimits, AssetManifest, AudioClientGate, AudioDecision, AudioPlay, CapabilityPolicy,
    ClientCapability, ClientHello, PayloadEnvelope, ProtocolLimits, UiAction, UiActionContext,
    UiActionGate, UiOpen,
};
use mythicraft_compat::import_mythicmobs;
use mythicraft_integration_harness::{IntegrationRunner, REQUIRED_STAGES};
use mythicraft_observability::{StageResult, StageStatus};
use mythicraft_permission::{PermissionEngine, PermissionNode, User};
use mythicraft_persistence::{EconomyTransaction, PlayerState, SaveStore, TransactionOutcome};
use mythicraft_protocol::{decode_frame, encode_frame, DecodeStatus, PacketFrame};
use mythicraft_rpg::runtime::{Position, RpgRuntime, RuntimeEvent, TickBudget};
use mythicraft_rpg::{
    Effect, EntityOptions, ItemDefinition, LootEntry, LootTable, RpgEntityDefinition,
    SkillDefinition, TargetSelector,
};
use mythicraft_session::{HandshakeIntent, SessionMachine, SessionState};
use mythicraft_vanilla_data::load_version_matrix;
use mythicraft_world::{inspect_world_directory, ChunkNbtSchema, WorldInspectionLimits};
use serde_json::json;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

fn fixture(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(path)
}

#[test]
fn machine_readable_vertical_slice_uses_real_cross_window_contracts() -> Result<(), String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("mythicraft-integration-{nonce}"));
    let report_path = root.join("reports/run.jsonl");
    let store = SaveStore::open(root.join("saves")).map_err(|error| error.to_string())?;
    let mut runner = IntegrationRunner::open(&report_path, "vertical-slice-real-1")?;

    runner.stage("startup", || {
        let matrix_path = root.join("version.json");
        fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        fs::write(
            &matrix_path,
            serde_json::to_vec_pretty(&json!({
                "schema_version": 1,
                "minecraft_version": "26.2",
                "protocol_version": 776,
                "data_version": {"minimum": 4903, "maximum": 4903},
                "registry_sha256": "3ffaca442dbbd1d9acb2b7bf2509cbd80e30dbc5349dfbad39eda7f4e6bd5a8b",
                "client": {
                    "loader": "fabric",
                    "loader_version": "contract-test",
                    "mod_version": "0.1.0-dev"
                }
            }))
            .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        load_version_matrix(&matrix_path).map_err(|error| error.to_string())?;
        Ok::<_, String>(())
    })?;

    runner.stage("world_load", || {
        let world_root = root.join("world");
        let unsupported_root = root.join("unsupported-world");
        write_level_dat(&world_root, 4903)?;
        write_level_dat(&unsupported_root, i32::MAX)?;
        fs::create_dir_all(world_root.join("region")).map_err(|error| error.to_string())?;
        fs::create_dir_all(unsupported_root.join("region")).map_err(|error| error.to_string())?;
        fs::write(world_root.join("region/r.0.0.mca"), vec![0_u8; 8192])
            .map_err(|error| error.to_string())?;
        fs::write(world_root.join("region/r.1.0.mca"), vec![0_u8; 39])
            .map_err(|error| error.to_string())?;

        let schema = ChunkNbtSchema::new(
            mythicraft_api::DataVersionRange {
                minimum: 4435,
                maximum: 4903,
            },
            ["sections", "Heightmaps"],
        );
        let summary =
            inspect_world_directory(&world_root, &schema, WorldInspectionLimits::default())
                .map_err(|error| error.to_string())?;
        if !summary.level_dat.supported
            || summary.region_count != 1
            || !summary.issues.iter().any(|issue| {
                matches!(
                    issue.kind,
                    mythicraft_world::WorldFileIssueKind::RegionInspectionFailed { .. }
                )
            })
        {
            return Err(format!(
                "unexpected valid/corrupt world summary: {summary:?}"
            ));
        }

        let unsupported =
            inspect_world_directory(&unsupported_root, &schema, WorldInspectionLimits::default())
                .map_err(|error| error.to_string())?;
        if unsupported.level_dat.supported || unsupported.level_dat.data_version != i32::MAX {
            return Err(format!(
                "unsupported world version was accepted: {:?}",
                unsupported.level_dat
            ));
        }
        Ok::<_, String>(())
    })?;

    let mut session = SessionMachine::new(776);
    runner.stage("client_connect", || {
        let frame = PacketFrame {
            packet_id: 0,
            payload: b"login".to_vec(),
        };
        let encoded = encode_frame(&frame).map_err(|error| error.to_string())?;
        match decode_frame(&encoded).map_err(|error| error.to_string())? {
            DecodeStatus::Complete { value, consumed }
                if value == frame && consumed == encoded.len() => {}
            other => return Err(format!("unexpected decoded frame: {other:?}")),
        }
        session
            .begin_handshake(776, HandshakeIntent::Login)
            .map_err(|error| error.to_string())?;
        session.finish_login().map_err(|error| error.to_string())?;
        session
            .finish_configuration()
            .map_err(|error| error.to_string())?;
        if session.state() != SessionState::Play {
            return Err("session did not reach play".into());
        }
        Ok::<_, String>(())
    })?;

    let limits = ProtocolLimits::default();
    runner.stage("capability_negotiation", || {
        let hello_bytes =
            fs::read(fixture("client/hello-v1.json")).map_err(|error| error.to_string())?;
        let envelope =
            PayloadEnvelope::decode(&hello_bytes, limits).map_err(|error| error.to_string())?;
        let hello: ClientHello = envelope.payload_as().map_err(|error| error.to_string())?;
        let supported = hello.capabilities.clone();
        let policy = CapabilityPolicy {
            protocol_version: 1,
            supported,
            required: BTreeSet::from([ClientCapability::UiHud]),
        };
        let response = policy
            .negotiate(&hello, limits)
            .map_err(|error| error.to_string())?;
        if !response.rpg_play_allowed {
            return Err(format!(
                "capability negotiation rejected RPG play: {:?}",
                response.error_reasons
            ));
        }
        Ok::<_, String>(())
    })?;

    let mut player = PlayerState::new("integration-player");
    let mut revision = store
        .save(&player, None)
        .map_err(|error| error.to_string())?;
    runner.stage("player_move", || {
        player.position.x = 12.5;
        player.position.y = 65.0;
        revision = store
            .save(&player, Some(revision))
            .map_err(|error| error.to_string())?;
        Ok::<_, String>(())
    })?;

    let mut runtime = runner.stage("rpg_spawn", || {
        let yaml = fs::read_to_string(fixture("compat/mythicmobs/basic.yml"))
            .map_err(|error| error.to_string())?;
        let report = import_mythicmobs("basic.yml", yaml.trim_start_matches('\u{feff}'), false)
            .map_err(|error| error.to_string())?;
        let mut document = report
            .document
            .ok_or_else(|| "MythicMobs import produced no RPG document".to_string())?;
        let goblin = document
            .entities
            .iter_mut()
            .find(|entity| entity.id == "Goblin")
            .ok_or_else(|| "imported Goblin is missing".to_string())?;
        goblin.health = 10.0;
        goblin.loot_table = Some("goblin-drops".into());
        goblin.experience = 25;
        document.items.push(ItemDefinition {
            id: "gold".into(),
            material: "minecraft:gold_nugget".into(),
            amount: 1,
            metadata: json!({}),
        });
        document.loot_tables.push(LootTable {
            id: "goblin-drops".into(),
            entries: vec![LootEntry {
                item: "gold".into(),
                chance: 1.0,
                min: 2,
                max: 2,
            }],
        });
        document.entities.push(RpgEntityDefinition {
            id: "PlayerDef".into(),
            display: "Player".into(),
            entity_type: "PLAYER".into(),
            health: 20.0,
            damage: 1.0,
            attributes: vec![],
            equipment: vec![],
            options: EntityOptions::default(),
            triggers: vec![],
            skills: vec![SkillDefinition {
                id: "finisher".into(),
                conditions: vec![],
                effects: vec![Effect::Damage {
                    amount: 20.0,
                    target: TargetSelector::Explicit("Goblin-1".into()),
                }],
                cooldown_ticks: 0,
            }],
            loot_table: None,
            experience: 0,
        });
        document
            .validate()
            .map_err(|errors| format!("invalid RPG document: {errors:?}"))?;
        let mut runtime = RpgRuntime::new(document, 7);
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
            .map_err(|error| error.to_string())?;
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
            .map_err(|error| error.to_string())?;
        Ok::<_, String>(runtime)
    })?;

    runner.stage("skill_damage", || {
        runtime
            .execute_skill("Player", "finisher", None, TickBudget::default())
            .map_err(|error| error.to_string())?;
        if runtime.entities["Goblin-1"].alive {
            return Err("Goblin survived finisher".into());
        }
        Ok::<_, String>(())
    })?;

    runner.stage("loot_economy", || {
        let dropped = runtime
            .events
            .iter()
            .find_map(|event| match event {
                RuntimeEvent::Drop { item, amount, .. } if item == "gold" => Some(*amount),
                _ => None,
            })
            .ok_or_else(|| "RPG runtime emitted no gold drop".to_string())?;
        let experience = runtime
            .events
            .iter()
            .find_map(|event| match event {
                RuntimeEvent::Experience { amount, .. } => Some(*amount),
                _ => None,
            })
            .ok_or_else(|| "RPG runtime emitted no experience".to_string())?;
        let outcome = store
            .apply_economy_transaction(&EconomyTransaction {
                transaction_id: "integration-reward-1".into(),
                player_id: player.player_id.clone(),
                amount: i64::from(dropped) * 10 + i64::from(experience),
                reason: "RPG loot and experience reward".into(),
                tick: 120,
                config_hash: "abcd".into(),
            })
            .map_err(|error| error.to_string())?;
        revision = match outcome {
            TransactionOutcome::Applied {
                after: 45,
                revision,
                ..
            } => revision,
            other => return Err(format!("unexpected economy outcome: {other:?}")),
        };
        Ok::<_, String>(())
    })?;

    runner.stage("ui_audio", || {
        let user_id = Uuid::from_u128(1);
        let mut permissions = PermissionEngine::default();
        permissions.users.insert(
            user_id,
            User {
                id: user_id,
                groups: vec![],
                permissions: vec![PermissionNode {
                    node: "rpg.quest.accept".into(),
                    value: true,
                    expiry_tick: None,
                    contexts: HashMap::new(),
                }],
                meta: HashMap::new(),
            },
        );
        let permission_granted =
            permissions.check(user_id, "rpg.quest.accept", 120, &HashMap::new());

        let open_envelope = PayloadEnvelope::decode(
            &fs::read(fixture("client/ui-open-v1.json")).map_err(|error| error.to_string())?,
            limits,
        )
        .map_err(|error| error.to_string())?;
        let open: UiOpen = open_envelope
            .payload_as()
            .map_err(|error| error.to_string())?;
        open.validate(limits).map_err(|error| error.to_string())?;

        let action_envelope = PayloadEnvelope::decode(
            &fs::read(fixture("client/ui-action-v1.json")).map_err(|error| error.to_string())?,
            limits,
        )
        .map_err(|error| error.to_string())?;
        let action: UiAction = action_envelope
            .payload_as()
            .map_err(|error| error.to_string())?;
        let mut gate = UiActionGate::new(4, 1_000).map_err(|error| error.to_string())?;
        gate.validate_and_record(
            &action_envelope,
            &action,
            UiActionContext {
                now_unix_ms: 1_800_000_000_000,
                expected_page_version: open.page_version,
                expected_nonce: "ui-nonce-0001",
                permission_granted,
                in_range: true,
                state_allowed: true,
            },
            limits,
        )
        .map_err(|error| error.to_string())?;

        let manifest_envelope = PayloadEnvelope::decode(
            &fs::read(fixture("client/asset-manifest-v1.json"))
                .map_err(|error| error.to_string())?,
            limits,
        )
        .map_err(|error| error.to_string())?;
        let manifest: AssetManifest = manifest_envelope
            .payload_as()
            .map_err(|error| error.to_string())?;
        manifest
            .validate(AssetLimits::default())
            .map_err(|error| error.to_string())?;

        let audio_envelope = PayloadEnvelope::decode(
            &fs::read(fixture("client/audio-play-v1.json")).map_err(|error| error.to_string())?,
            limits,
        )
        .map_err(|error| error.to_string())?;
        let audio: AudioPlay = audio_envelope
            .payload_as()
            .map_err(|error| error.to_string())?;
        let resource_available = manifest.find(&audio.sound_id).is_some();
        let mut audio_gate = AudioClientGate::new(8, 1_000).map_err(|error| error.to_string())?;
        if audio_gate
            .evaluate(&audio, 1_800_000_000_000, resource_available)
            .map_err(|error| error.to_string())?
            != AudioDecision::Play
        {
            return Err("typed audio event was not accepted".into());
        }
        Ok::<_, String>(())
    })?;

    runner.stage("disconnect", || {
        session.close();
        if session.state() != SessionState::Closed {
            return Err("session did not close".into());
        }
        Ok::<_, String>(())
    })?;

    let mut reconnected = SessionMachine::new(776);
    runner.stage("reconnect", || {
        reconnected
            .begin_handshake(776, HandshakeIntent::Login)
            .map_err(|error| error.to_string())?;
        reconnected
            .finish_login()
            .map_err(|error| error.to_string())?;
        reconnected
            .finish_configuration()
            .map_err(|error| error.to_string())?;
        if reconnected.state() != SessionState::Play {
            return Err("reconnected session did not reach play".into());
        }
        Ok::<_, String>(())
    })?;

    runner.stage("save_restore", || {
        let loaded = store
            .load(&player.player_id)
            .map_err(|error| error.to_string())?;
        if loaded.revision != revision
            || loaded.state.position.x != 12.5
            || loaded.state.economy_balance != 45
        {
            return Err(format!(
                "restored state mismatch: revision={} x={} balance={}",
                loaded.revision, loaded.state.position.x, loaded.state.economy_balance
            ));
        }
        Ok::<_, String>(())
    })?;

    let lines = fs::read_to_string(&report_path).map_err(|error| error.to_string())?;
    let results = lines
        .lines()
        .map(|line| serde_json::from_str::<StageResult>(line).map_err(|error| error.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    if results.len() != REQUIRED_STAGES.len() {
        return Err(format!(
            "expected {} stage results, got {}",
            REQUIRED_STAGES.len(),
            results.len()
        ));
    }
    for (result, expected_stage) in results.iter().zip(REQUIRED_STAGES) {
        if result.stage != *expected_stage || result.status == StageStatus::Failed {
            return Err(format!("unexpected stage result: {result:?}"));
        }
    }
    if results
        .iter()
        .any(|result| result.status == StageStatus::Skipped)
    {
        return Err("world_load must be covered by the bounded Anvil diagnostic".into());
    }
    fs::remove_dir_all(root).map_err(|error| error.to_string())?;
    Ok(())
}

fn write_level_dat(root: &PathBuf, data_version: i32) -> Result<(), String> {
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    let mut nbt = vec![10, 0, 0, 10, 0, 4, b'D', b'a', b't', b'a'];
    push_named_int(&mut nbt, "DataVersion", data_version);
    nbt.extend_from_slice(&[0, 0]);
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    std::io::Write::write_all(&mut encoder, &nbt).map_err(|error| error.to_string())?;
    let compressed = encoder.finish().map_err(|error| error.to_string())?;
    fs::write(root.join("level.dat"), compressed).map_err(|error| error.to_string())
}

fn push_named_int(bytes: &mut Vec<u8>, name: &str, value: i32) {
    bytes.push(3);
    let name_length = u16::try_from(name.len()).expect("synthetic NBT name length");
    bytes.extend_from_slice(&name_length.to_be_bytes());
    bytes.extend_from_slice(name.as_bytes());
    bytes.extend_from_slice(&value.to_be_bytes());
}
