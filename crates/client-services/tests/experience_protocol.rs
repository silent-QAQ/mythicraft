use std::{collections::BTreeSet, fs, path::PathBuf};

use mythicraft_client_services::{
    evaluate_experience_capability, BossbarState, ClientCapability, ComponentRevisionGate,
    DamageDisplay, DialogueModel, ExperienceDecision, ExperienceError, HologramHealthBar,
    MessageType, ModelVisibility, PayloadEnvelope, ProtocolLimits, RevisionDecision, RpgHudModel,
    UiOpen, WaypointState,
};

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/client")
        .join(name);
    fs::read(path).expect("fixture must be readable")
}

fn envelope(name: &str) -> PayloadEnvelope {
    PayloadEnvelope::decode(&fixture(name), ProtocolLimits::default())
        .expect("component fixture must decode")
}

#[test]
fn validates_hud_and_skill_bar_ui_model() {
    let envelope = envelope("ui-open-hud-v1.json");
    assert_eq!(envelope.message_type, MessageType::UiOpen);
    let open: UiOpen = envelope.payload_as().expect("payload must be UI open");
    open.validate(ProtocolLimits::default())
        .expect("UI wrapper must validate");
    let model: RpgHudModel = serde_json::from_value(open.model).expect("model must be RPG HUD");
    model.validate().expect("HUD and skill slots must validate");
    assert_eq!(model.hud.health, 82.0);
    assert_eq!(model.skill_bar.slots[0].remaining_cooldown_ticks, 12);
}

#[test]
fn validates_dialogue_as_data_driven_ui() {
    let envelope = envelope("ui-open-dialogue-v1.json");
    let open: UiOpen = envelope.payload_as().expect("payload must be UI open");
    open.validate(ProtocolLimits::default())
        .expect("dialogue UI wrapper must validate");
    let dialogue: DialogueModel =
        serde_json::from_value(open.model).expect("model must be dialogue data");
    dialogue.validate().expect("dialogue model must validate");
    assert_eq!(dialogue.choices.len(), 2);
    assert_eq!(dialogue.nonce, "dialogue-nonce-0001");
}

#[test]
fn validates_damage_bossbar_hologram_waypoint_and_model_events() {
    let damage_envelope = envelope("damage-display-v1.json");
    assert_eq!(
        damage_envelope.message_type,
        MessageType::CombatDamageDisplay
    );
    let damage: DamageDisplay = damage_envelope
        .payload_as()
        .expect("payload must be damage display");
    damage.validate().expect("damage display must validate");

    let bossbar: BossbarState = envelope("bossbar-v1.json")
        .payload_as()
        .expect("payload must be bossbar");
    bossbar.validate().expect("bossbar must validate");

    let hologram: HologramHealthBar = envelope("hologram-health-v1.json")
        .payload_as()
        .expect("payload must be hologram");
    hologram.validate().expect("hologram must validate");

    let waypoint: WaypointState = envelope("waypoint-v1.json")
        .payload_as()
        .expect("payload must be waypoint");
    waypoint.validate().expect("waypoint must validate");

    let model: ModelVisibility = envelope("model-visibility-v1.json")
        .payload_as()
        .expect("payload must be model visibility");
    model.validate().expect("model visibility must validate");
}

#[test]
fn rejects_invalid_authoritative_display_values() {
    let mut damage: DamageDisplay = envelope("damage-display-v1.json")
        .payload_as()
        .expect("payload must be damage display");
    damage.amount = -1.0;
    assert!(matches!(
        damage.validate(),
        Err(ExperienceError::InvalidDamageAmount(_))
    ));

    let mut waypoint: WaypointState = envelope("waypoint-v1.json")
        .payload_as()
        .expect("payload must be waypoint");
    waypoint.position.x = 30_000_001.0;
    assert!(matches!(
        waypoint.validate(),
        Err(ExperienceError::InvalidPosition)
    ));
}

#[test]
fn stale_and_duplicate_component_revisions_do_not_apply() {
    let mut gate = ComponentRevisionGate::default();
    assert_eq!(
        gate.evaluate_and_record("bossbar:42", 2)
            .expect("revision must validate"),
        RevisionDecision::Applied
    );
    assert_eq!(
        gate.evaluate_and_record("bossbar:42", 2)
            .expect("revision must validate"),
        RevisionDecision::Duplicate
    );
    assert_eq!(
        gate.evaluate_and_record("bossbar:42", 1)
            .expect("revision must validate"),
        RevisionDecision::Stale
    );
    assert_eq!(
        gate.evaluate_and_record("bossbar:42", 3)
            .expect("revision must validate"),
        RevisionDecision::Applied
    );
}

#[test]
fn missing_capabilities_have_explicit_component_fallbacks() {
    let available = BTreeSet::from([ClientCapability::UiHud]);
    assert_eq!(
        evaluate_experience_capability(ClientCapability::UiHud, &available, true),
        ExperienceDecision::Render
    );
    assert_eq!(
        evaluate_experience_capability(ClientCapability::UiDialog, &available, true),
        ExperienceDecision::UseChatDialogue
    );
    assert_eq!(
        evaluate_experience_capability(ClientCapability::UiBossbar, &available, false),
        ExperienceDecision::UseVanillaBossbar
    );
    assert_eq!(
        evaluate_experience_capability(ClientCapability::ModelVisibility, &available, false),
        ExperienceDecision::UseDefaultModel
    );
    assert_eq!(
        evaluate_experience_capability(ClientCapability::UiDamageDisplay, &available, true),
        ExperienceDecision::HideComponent
    );
}
