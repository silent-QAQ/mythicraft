use std::{collections::BTreeSet, fs, path::PathBuf};

use mythicraft_client_services::{
    CapabilityPolicy, ClientCapability, ClientHello, MessageType, PayloadEnvelope, ProtocolError,
    ProtocolLimits, SUPPORTED_SCHEMA_VERSION,
};

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/client")
        .join(name);
    fs::read(path).expect("fixture must be readable")
}

#[test]
fn decodes_hello_golden_fixture() {
    let envelope = PayloadEnvelope::decode(&fixture("hello-v1.json"), ProtocolLimits::default())
        .expect("fixture must satisfy the v1 envelope");
    assert_eq!(envelope.message_type, MessageType::Hello);
    assert_eq!(envelope.schema_version, SUPPORTED_SCHEMA_VERSION);
    let hello: ClientHello = envelope.payload_as().expect("payload must be a hello");
    hello
        .validate(ProtocolLimits::default())
        .expect("hello fields must be valid");
    assert!(hello.capabilities.contains(&ClientCapability::UiHud));
    assert!(hello.capabilities.contains(&ClientCapability::AudioPlay));
}

#[test]
fn rejects_unknown_schema_instead_of_parsing_as_v1() {
    let error = PayloadEnvelope::decode(
        &fixture("hello-unknown-schema.json"),
        ProtocolLimits::default(),
    )
    .expect_err("unknown schema must be rejected");
    assert!(matches!(
        error,
        ProtocolError::UnsupportedSchemaVersion(999)
    ));
}

#[test]
fn rejects_forged_payload_length() {
    let error = PayloadEnvelope::decode(
        &fixture("hello-forged-length.json"),
        ProtocolLimits::default(),
    )
    .expect_err("declared length must match the payload");
    assert!(matches!(error, ProtocolError::PayloadLengthMismatch { .. }));
}

#[test]
fn missing_required_capability_blocks_rpg_play_state() {
    let envelope = PayloadEnvelope::decode(&fixture("hello-v1.json"), ProtocolLimits::default())
        .expect("fixture must decode");
    let hello: ClientHello = envelope.payload_as().expect("payload must be a hello");
    let policy = CapabilityPolicy {
        protocol_version: 1,
        supported: BTreeSet::from([
            ClientCapability::UiHud,
            ClientCapability::AudioPlay,
            ClientCapability::InputBind,
        ]),
        required: BTreeSet::from([ClientCapability::InputBind]),
    };
    let response = policy
        .negotiate(&hello, ProtocolLimits::default())
        .expect("valid hello must produce a response");
    assert!(!response.rpg_play_allowed);
    assert_eq!(
        response.required,
        BTreeSet::from([ClientCapability::InputBind])
    );
    assert!(!response.error_reasons.is_empty());
}

#[test]
fn protocol_version_mismatch_blocks_rpg_play_state() {
    let envelope = PayloadEnvelope::decode(&fixture("hello-v1.json"), ProtocolLimits::default())
        .expect("fixture must decode");
    let hello: ClientHello = envelope.payload_as().expect("payload must be a hello");
    let policy = CapabilityPolicy {
        protocol_version: 2,
        supported: hello.capabilities.clone(),
        required: BTreeSet::new(),
    };
    let response = policy
        .negotiate(&hello, ProtocolLimits::default())
        .expect("valid hello must produce a response");
    assert!(!response.rpg_play_allowed);
    assert!(response.error_reasons[0].contains("protocol_version_mismatch"));
}
