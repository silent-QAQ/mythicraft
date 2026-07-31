use std::{collections::BTreeSet, fs, path::PathBuf};

use mythicraft_client_services::{
    AudioDecision, ClientEffect, ClientSession, ClientSessionError, ClientSessionPhase,
    DispatchError, ExperienceDecision, MessageType, PayloadEnvelope, ProtocolDispatcher,
    ProtocolMessage, UiOpen, UiRun,
};
use serde_json::json;

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/client")
        .join(name);
    fs::read(path).expect("fixture must be readable")
}

fn message(name: &str) -> ProtocolMessage {
    ProtocolDispatcher::default()
        .decode(&fixture(name))
        .expect("fixture must dispatch")
}

#[test]
fn dispatcher_routes_supported_v1_messages() {
    assert!(matches!(
        message("capabilities-v1.json"),
        ProtocolMessage::Capabilities(_)
    ));
    assert!(matches!(
        message("ui-open-hud-v1.json"),
        ProtocolMessage::UiOpen(_)
    ));
    assert!(matches!(
        message("asset-manifest-v1.json"),
        ProtocolMessage::AssetManifest(_)
    ));
    assert!(matches!(
        message("audio-play-v1.json"),
        ProtocolMessage::AudioPlay(_)
    ));
    assert!(matches!(
        message("damage-display-v1.json"),
        ProtocolMessage::DamageDisplay(_)
    ));
}

#[test]
fn dispatcher_rejects_declared_but_unimplemented_message_type() {
    let envelope = PayloadEnvelope::new(
        MessageType::ModelSpawn,
        "model-spawn-unsupported",
        None,
        None,
        &json!({"entity": 42}),
    )
    .expect("test envelope must serialize");
    let error = ProtocolDispatcher::default()
        .route(envelope)
        .expect_err("unimplemented message must not be silently accepted");
    assert!(matches!(
        error,
        DispatchError::UnsupportedMessageType(MessageType::ModelSpawn)
    ));
}

#[test]
fn dispatcher_rejects_inconsistent_capability_response() {
    let payload = json!({
        "accepted": ["ui_hud"],
        "required": ["ui_dialog"],
        "degraded": [],
        "rpg_play_allowed": true,
        "error_reasons": []
    });
    let envelope = PayloadEnvelope::new(
        MessageType::Capabilities,
        "bad-capabilities",
        None,
        None,
        &payload,
    )
    .expect("test envelope must serialize");
    let error = ProtocolDispatcher::default()
        .route(envelope)
        .expect_err("inconsistent response must be rejected");
    assert!(matches!(error, DispatchError::Protocol(_)));
}

#[test]
fn client_requires_capabilities_before_server_events() {
    let mut session = ClientSession::new(8, 1_000).expect("valid client session");
    let error = session
        .apply(
            message("ui-open-hud-v1.json"),
            1_800_000_000_000,
            &BTreeSet::new(),
        )
        .expect_err("UI before capabilities must be rejected");
    assert!(matches!(error, ClientSessionError::CapabilitiesRequired));
}

#[test]
fn client_rejects_stale_ui_update_sequence() {
    let mut session = ClientSession::new(8, 1_000).expect("valid client session");
    let resources = BTreeSet::new();
    session
        .apply(
            message("capabilities-v1.json"),
            1_800_000_000_000,
            &resources,
        )
        .expect("capabilities must apply");
    session
        .apply(
            message("ui-open-hud-v1.json"),
            1_800_000_000_000,
            &resources,
        )
        .expect("UI open must apply");
    session
        .apply(
            message("ui-update-hud-v1.json"),
            1_800_000_000_000,
            &resources,
        )
        .expect("first update must apply");
    let error = session
        .apply(
            message("ui-update-hud-v1.json"),
            1_800_000_000_000,
            &resources,
        )
        .expect_err("replayed stale update must be rejected");
    assert!(matches!(
        error,
        ClientSessionError::PageVersionMismatch { .. }
    ));
}

#[test]
fn smoke_runner_processes_complete_client_sequence() {
    let names = [
        "capabilities-v1.json",
        "asset-manifest-v1.json",
        "ui-open-hud-v1.json",
        "audio-play-v1.json",
        "damage-display-v1.json",
        "bossbar-v1.json",
        "hologram-health-v1.json",
        "waypoint-v1.json",
        "model-visibility-v1.json",
        "ui-update-hud-v1.json",
        "ui-close-hud-v1.json",
    ];
    let frames = names.map(fixture);
    let resources = BTreeSet::from([
        "mythicraft:sounds/ui/quest_accept".to_owned(),
        "mythicraft:textures/ui/waypoint_trial".to_owned(),
        "mythicraft:models/entity/training_construct".to_owned(),
    ]);
    let dispatcher = ProtocolDispatcher::default();
    let mut session = ClientSession::new(8, 1_000).expect("valid client session");
    let report = mythicraft_client_services::run_client_smoke(
        &dispatcher,
        &mut session,
        frames.iter().map(Vec::as_slice),
        1_800_000_000_000,
        &resources,
    )
    .expect("smoke sequence must complete");

    assert_eq!(report.processed_frames, names.len());
    assert_eq!(report.final_phase, ClientSessionPhase::Ready);
    assert_eq!(session.active_page_version("rpg.hud"), None);
    assert!(report.effects.iter().any(|effect| matches!(
        effect,
        ClientEffect::AudioEvaluated {
            decision: AudioDecision::Play,
            ..
        }
    )));
    assert!(report.effects.iter().any(|effect| matches!(
        effect,
        ClientEffect::VersionedComponent {
            component_id,
            render_decision: ExperienceDecision::UseDefaultModel,
            ..
        } if component_id == "model:42"
    )));
    assert!(report.effects.iter().any(|effect| matches!(
        effect,
        ClientEffect::VersionedComponent {
            component_id,
            render_decision: ExperienceDecision::HideComponent,
            ..
        } if component_id == "trial-arena"
    )));
}

#[test]
fn smoke_runner_rejects_client_to_server_message_direction() {
    let dispatcher = ProtocolDispatcher::default();
    let mut session = ClientSession::new(8, 1_000).expect("valid client session");
    let resources = BTreeSet::new();
    session
        .apply(
            message("capabilities-v1.json"),
            1_800_000_000_000,
            &resources,
        )
        .expect("capabilities must apply");
    let action = dispatcher
        .decode(&fixture("ui-action-v1.json"))
        .expect("action fixture must dispatch structurally");
    let error = session
        .apply(action, 1_800_000_000_000, &resources)
        .expect_err("client session must reject client-originating action");
    assert!(matches!(error, ClientSessionError::WrongDirection));
}

#[test]
fn ui_open_payload_remains_renderer_independent() {
    let ProtocolMessage::UiOpen(open) = message("ui-open-hud-v1.json") else {
        panic!("fixture must route as UI open");
    };
    let UiOpen { model, .. } = open;
    assert!(model.is_object());
    assert!(model.get("hud").is_some());
    assert!(model.get("script").is_none());
}

#[test]
fn client_session_accepts_ui_run_for_an_open_page() {
    let resources = BTreeSet::new();
    let mut session = ClientSession::new(8, 1_000).expect("valid client session");
    session
        .apply(
            message("capabilities-v1.json"),
            1_800_000_000_000,
            &resources,
        )
        .expect("capabilities must apply");
    session
        .apply(
            ProtocolMessage::UiOpen(UiOpen {
                page_id: "任务面板".to_owned(),
                page_version: 1,
                model: json!({}),
                required_capabilities: Vec::new(),
                required_permissions: Vec::new(),
            }),
            1_800_000_000_000,
            &resources,
        )
        .expect("page must open");

    let effect = session
        .apply(
            ProtocolMessage::UiRun(UiRun {
                page_id: "任务面板".to_owned(),
                page_version: 1,
                code: "Message.chat('clicked')".to_owned(),
            }),
            1_800_000_000_000,
            &resources,
        )
        .expect("ui run must apply");

    assert_eq!(
        effect,
        ClientEffect::UiRunReceived {
            page_id: "任务面板".to_owned(),
            version: 1,
            code: "Message.chat('clicked')".to_owned(),
        }
    );
}
