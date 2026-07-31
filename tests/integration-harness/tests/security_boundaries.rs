use mythicraft_api::TickId;
use mythicraft_client_services::{
    AssetLimits, AssetManifest, AudioClientGate, AudioDecision, AudioPlay, BossbarState,
    ClientHello, ComponentRevisionGate, DamageDisplay, DialogueModel, HologramHealthBar,
    MessageType, ModelVisibility, PayloadEnvelope, ProtocolLimits, RevisionDecision, RpgHudModel,
    UiAction, UiActionContext, UiActionError, UiActionGate, UiOpen, WaypointState,
};
use mythicraft_protocol::{decode_frame, decode_varint, ProtocolError, MAX_PACKET_BYTES};
use mythicraft_session::{KeepAliveError, KeepAliveId, KeepAliveTracker};
use serde_json::json;
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

fn fixture(path: &str) -> Vec<u8> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    fs::read(root.join(path)).expect("read fixture")
}

#[test]
fn client_payload_fixtures_and_truncations_fail_closed() {
    let limits = ProtocolLimits::default();
    let valid = fixture("client/hello-v1.json");
    let envelope = PayloadEnvelope::decode(&valid, limits).expect("valid hello envelope");
    let hello: ClientHello = envelope.payload_as().expect("hello payload");
    hello.validate(limits).expect("valid hello");

    for invalid in [
        "client/hello-forged-length.json",
        "client/hello-unknown-schema.json",
    ] {
        assert!(PayloadEnvelope::decode(&fixture(invalid), limits).is_err());
    }

    for prefix_length in 0..valid.len() {
        let prefix = &valid[..prefix_length];
        assert!(catch_unwind(AssertUnwindSafe(|| PayloadEnvelope::decode(prefix, limits))).is_ok());
    }
}

#[test]
fn client_payload_limits_reject_depth_and_flood() {
    let limits = ProtocolLimits::default();
    let mut nested = json!("leaf");
    for _ in 0..=limits.max_nesting_depth {
        nested = json!({"child": nested});
    }
    let envelope = PayloadEnvelope::new(
        MessageType::UiAction,
        "deep-payload",
        Some("nonce-1".into()),
        None,
        &nested,
    )
    .expect("build envelope");
    assert!(envelope.encode(limits).is_err());

    let flood = vec![b' '; limits.max_message_bytes + 1];
    assert!(PayloadEnvelope::decode(&flood, limits).is_err());
}

#[test]
fn protocol_random_corpus_and_replay_fail_closed_without_panics() {
    assert_eq!(decode_varint(&[0x80]), Err(ProtocolError::TruncatedVarInt));
    assert_eq!(
        decode_varint(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x00]),
        Err(ProtocolError::VarIntTooLong)
    );
    assert_eq!(
        decode_varint(&[0x81, 0x00]),
        Err(ProtocolError::NonCanonicalVarInt)
    );
    let mut oversized = Vec::new();
    mythicraft_protocol::encode_varint((MAX_PACKET_BYTES + 1) as i32, &mut oversized);
    assert!(matches!(
        decode_frame(&oversized),
        Err(ProtocolError::PacketTooLarge { .. })
    ));

    for seed in 0_u32..512 {
        let mut state = seed.wrapping_add(1);
        let length = (state as usize % 64) + 1;
        let mut bytes = Vec::with_capacity(length);
        for _ in 0..length {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            bytes.push((state >> 24) as u8);
        }
        assert!(catch_unwind(AssertUnwindSafe(|| decode_frame(&bytes))).is_ok());
    }

    let mut keep_alive = KeepAliveTracker::default();
    keep_alive
        .issue(KeepAliveId(7), TickId(100))
        .expect("issue keep alive");
    keep_alive
        .acknowledge(KeepAliveId(7))
        .expect("acknowledge keep alive");
    assert_eq!(
        keep_alive.acknowledge(KeepAliveId(7)),
        Err(KeepAliveError::UnexpectedResponse)
    );
}

#[test]
fn ui_action_gate_rejects_expiry_permission_replay_and_rate_limit() {
    let limits = ProtocolLimits::default();
    let valid_envelope = PayloadEnvelope::decode(&fixture("client/ui-action-v1.json"), limits)
        .expect("valid action envelope");
    let valid_action: UiAction = valid_envelope.payload_as().expect("valid action");
    let expired_envelope =
        PayloadEnvelope::decode(&fixture("client/ui-action-expired-v1.json"), limits)
            .expect("expired action envelope is structurally valid");
    let expired_action: UiAction = expired_envelope.payload_as().expect("expired action");

    let context = UiActionContext {
        now_unix_ms: 1_800_000_000_000,
        expected_page_version: 3,
        expected_nonce: "ui-nonce-0001",
        permission_granted: true,
        in_range: true,
        state_allowed: true,
    };
    let mut expired_gate = UiActionGate::new(4, 1_000).expect("gate");
    assert!(matches!(
        expired_gate.validate_and_record(&expired_envelope, &expired_action, context, limits),
        Err(UiActionError::Expired)
    ));

    let mut permission_gate = UiActionGate::new(4, 1_000).expect("gate");
    assert!(matches!(
        permission_gate.validate_and_record(
            &valid_envelope,
            &valid_action,
            UiActionContext {
                permission_granted: false,
                ..context
            },
            limits
        ),
        Err(UiActionError::PermissionDenied)
    ));

    let mut replay_gate = UiActionGate::new(4, 1_000).expect("gate");
    replay_gate
        .validate_and_record(&valid_envelope, &valid_action, context, limits)
        .expect("first action");
    assert!(matches!(
        replay_gate.validate_and_record(&valid_envelope, &valid_action, context, limits),
        Err(UiActionError::DuplicateRequest)
    ));

    let mut second_action = valid_action.clone();
    second_action.request_id = "ui-action-0002".into();
    let second_envelope = PayloadEnvelope::new(
        MessageType::UiAction,
        &second_action.request_id,
        Some(second_action.nonce.clone()),
        Some(second_action.expires_at_unix_ms),
        &second_action,
    )
    .expect("second envelope");
    let mut rate_gate = UiActionGate::new(1, 1_000).expect("rate gate");
    rate_gate
        .validate_and_record(&valid_envelope, &valid_action, context, limits)
        .expect("first rate action");
    assert!(matches!(
        rate_gate.validate_and_record(&second_envelope, &second_action, context, limits),
        Err(UiActionError::RateLimited)
    ));
}

#[test]
fn asset_hash_and_audio_availability_expiry_and_flood_are_enforced() {
    let limits = ProtocolLimits::default();
    let valid_manifest_envelope =
        PayloadEnvelope::decode(&fixture("client/asset-manifest-v1.json"), limits)
            .expect("asset manifest envelope");
    let valid_manifest: AssetManifest = valid_manifest_envelope
        .payload_as()
        .expect("asset manifest payload");
    valid_manifest
        .validate(AssetLimits::default())
        .expect("valid asset manifest");

    let mismatch_envelope = PayloadEnvelope::decode(
        &fixture("client/asset-manifest-hash-mismatch-v1.json"),
        limits,
    )
    .expect("mismatch envelope is structurally valid");
    let mismatch: AssetManifest = mismatch_envelope.payload_as().expect("mismatch payload");
    assert!(mismatch.validate(AssetLimits::default()).is_err());

    let audio_envelope = PayloadEnvelope::decode(&fixture("client/audio-play-v1.json"), limits)
        .expect("audio envelope");
    let audio: AudioPlay = audio_envelope.payload_as().expect("audio payload");
    let mut availability_gate = AudioClientGate::new(4, 1_000).expect("audio gate");
    assert_eq!(
        availability_gate
            .evaluate(&audio, 1_800_000_000_000, false)
            .expect("missing resource decision"),
        AudioDecision::DropMissingResource
    );
    let mut expired = audio.clone();
    expired.expires_at_unix_ms = 1_000;
    assert_eq!(
        availability_gate
            .evaluate(&expired, 2_000, true)
            .expect("expired decision"),
        AudioDecision::DropExpired
    );

    let burst: Vec<PayloadEnvelope> =
        serde_json::from_slice(&fixture("client/audio-high-frequency-v1.json"))
            .expect("audio burst");
    let mut rate_gate = AudioClientGate::new(2, 1_000).expect("rate gate");
    let decisions = burst
        .iter()
        .map(|envelope| {
            envelope.validate(limits).expect("burst envelope");
            let event: AudioPlay = envelope.payload_as().expect("burst event");
            rate_gate
                .evaluate(&event, 1_800_000_000_000, true)
                .expect("burst decision")
        })
        .collect::<Vec<_>>();
    assert_eq!(decisions[0], AudioDecision::Play);
    assert_eq!(decisions[1], AudioDecision::Play);
    assert!(decisions[2..]
        .iter()
        .all(|decision| *decision == AudioDecision::DropRateLimited));
}

#[test]
fn experience_models_validate_and_component_revisions_reject_replay() {
    let limits = ProtocolLimits::default();

    let hud_envelope = PayloadEnvelope::decode(&fixture("client/ui-open-hud-v1.json"), limits)
        .expect("HUD envelope");
    let hud_open: UiOpen = hud_envelope.payload_as().expect("HUD open payload");
    hud_open.validate(limits).expect("valid HUD open");
    let hud: RpgHudModel = serde_json::from_value(hud_open.model).expect("HUD model");
    hud.validate().expect("valid HUD model");

    let dialogue_envelope =
        PayloadEnvelope::decode(&fixture("client/ui-open-dialogue-v1.json"), limits)
            .expect("dialogue envelope");
    let dialogue_open: UiOpen = dialogue_envelope
        .payload_as()
        .expect("dialogue open payload");
    dialogue_open.validate(limits).expect("valid dialogue open");
    let dialogue: DialogueModel =
        serde_json::from_value(dialogue_open.model).expect("dialogue model");
    dialogue.validate().expect("valid dialogue model");

    let damage_envelope =
        PayloadEnvelope::decode(&fixture("client/damage-display-v1.json"), limits)
            .expect("damage display envelope");
    let damage: DamageDisplay = damage_envelope
        .payload_as()
        .expect("damage display payload");
    damage.validate().expect("valid damage display");

    let bossbar_envelope = PayloadEnvelope::decode(&fixture("client/bossbar-v1.json"), limits)
        .expect("bossbar envelope");
    let bossbar: BossbarState = bossbar_envelope.payload_as().expect("bossbar payload");
    bossbar.validate().expect("valid bossbar");

    let hologram_envelope =
        PayloadEnvelope::decode(&fixture("client/hologram-health-v1.json"), limits)
            .expect("hologram envelope");
    let hologram: HologramHealthBar = hologram_envelope.payload_as().expect("hologram payload");
    hologram.validate().expect("valid hologram");

    let waypoint_envelope = PayloadEnvelope::decode(&fixture("client/waypoint-v1.json"), limits)
        .expect("waypoint envelope");
    let waypoint: WaypointState = waypoint_envelope.payload_as().expect("waypoint payload");
    waypoint.validate().expect("valid waypoint");

    let visibility_envelope =
        PayloadEnvelope::decode(&fixture("client/model-visibility-v1.json"), limits)
            .expect("model visibility envelope");
    let visibility: ModelVisibility = visibility_envelope
        .payload_as()
        .expect("model visibility payload");
    visibility.validate().expect("valid model visibility");

    let mut revisions = ComponentRevisionGate::default();
    assert_eq!(
        revisions
            .evaluate_and_record("bossbar:42", 1)
            .expect("first revision"),
        RevisionDecision::Applied
    );
    assert_eq!(
        revisions
            .evaluate_and_record("bossbar:42", 1)
            .expect("duplicate revision"),
        RevisionDecision::Duplicate
    );
    assert_eq!(
        revisions
            .evaluate_and_record("bossbar:42", 2)
            .expect("new revision"),
        RevisionDecision::Applied
    );
    assert_eq!(
        revisions
            .evaluate_and_record("bossbar:42", 1)
            .expect("stale revision"),
        RevisionDecision::Stale
    );
}
