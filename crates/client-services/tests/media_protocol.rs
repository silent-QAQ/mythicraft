use std::{collections::BTreeSet, fs, path::PathBuf};

use mythicraft_client_services::{
    evaluate_local_asset, AssetError, AssetFallback, AssetLimits, AssetManifest, AssetResultStatus,
    AssetType, AudioClientGate, AudioDecision, AudioPlay, MessageType, PayloadEnvelope,
    ProtocolLimits,
};

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/client")
        .join(name);
    fs::read(path).expect("fixture must be readable")
}

#[test]
fn validates_asset_manifest_and_declared_hash() {
    let envelope = PayloadEnvelope::decode(
        &fixture("asset-manifest-v1.json"),
        ProtocolLimits::default(),
    )
    .expect("asset manifest envelope must decode");
    assert_eq!(envelope.message_type, MessageType::AssetManifest);
    let manifest: AssetManifest = envelope.payload_as().expect("payload must be a manifest");
    manifest
        .validate(AssetLimits::default())
        .expect("manifest and asset hashes must validate");
    assert_eq!(
        manifest.computed_hash().expect("hash must compute"),
        manifest.manifest_hash
    );
}

#[test]
fn rejects_manifest_hash_mismatch_fixture() {
    let envelope = PayloadEnvelope::decode(
        &fixture("asset-manifest-hash-mismatch-v1.json"),
        ProtocolLimits::default(),
    )
    .expect("envelope itself remains syntactically valid");
    let manifest: AssetManifest = envelope.payload_as().expect("payload must be a manifest");
    let error = manifest
        .validate(AssetLimits::default())
        .expect_err("manifest hash mismatch must be rejected");
    assert!(matches!(error, AssetError::ManifestHashMismatch { .. }));
}

#[test]
fn rejects_url_and_parent_directory_asset_paths() {
    let envelope = PayloadEnvelope::decode(
        &fixture("asset-manifest-v1.json"),
        ProtocolLimits::default(),
    )
    .expect("fixture must decode");
    let mut manifest: AssetManifest = envelope.payload_as().expect("payload must be a manifest");
    manifest.assets[0].path = "https://example.invalid/asset.png".to_owned();
    let error = manifest
        .validate(AssetLimits::default())
        .expect_err("URL assets must be rejected");
    assert!(matches!(error, AssetError::InvalidAssetPath));

    manifest.assets[0].path = "textures/../secret.png".to_owned();
    let error = manifest
        .validate(AssetLimits::default())
        .expect_err("parent traversal must be rejected");
    assert!(matches!(error, AssetError::InvalidAssetPath));
}

#[test]
fn reports_missing_and_hash_mismatch_with_explicit_fallback() {
    let envelope = PayloadEnvelope::decode(
        &fixture("asset-manifest-v1.json"),
        ProtocolLimits::default(),
    )
    .expect("fixture must decode");
    let manifest: AssetManifest = envelope.payload_as().expect("payload must be a manifest");
    let texture = manifest
        .find("mythicraft:textures/ui/quest_panel")
        .expect("texture fixture must exist");
    let supported = BTreeSet::from([AssetType::Texture, AssetType::Sound]);

    let missing = evaluate_local_asset(texture, None, &supported);
    assert_eq!(missing.status, AssetResultStatus::Missing);
    assert_eq!(missing.fallback, AssetFallback::PlaceholderTexture);

    let mismatch = evaluate_local_asset(
        texture,
        Some("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"),
        &supported,
    );
    assert_eq!(mismatch.status, AssetResultStatus::HashMismatch);
    assert_eq!(mismatch.fallback, AssetFallback::PlaceholderTexture);
}

#[test]
fn missing_audio_resource_degrades_to_drop_without_error() {
    let envelope =
        PayloadEnvelope::decode(&fixture("audio-play-v1.json"), ProtocolLimits::default())
            .expect("audio envelope must decode");
    assert_eq!(envelope.message_type, MessageType::AudioPlay);
    let event: AudioPlay = envelope.payload_as().expect("payload must be audio play");
    let mut gate = AudioClientGate::new(10, 1_000).expect("valid audio limit");
    let decision = gate
        .evaluate(&event, 1_800_000_000_000, false)
        .expect("missing resource is a degradation, not a protocol error");
    assert_eq!(decision, AudioDecision::DropMissingResource);
}

#[test]
fn high_frequency_audio_fixture_is_client_rate_limited() {
    let envelopes: Vec<PayloadEnvelope> =
        serde_json::from_slice(&fixture("audio-high-frequency-v1.json"))
            .expect("burst fixture must be an envelope array");
    let mut gate = AudioClientGate::new(2, 1_000).expect("valid audio limit");
    let mut decisions = Vec::new();

    for envelope in envelopes {
        envelope
            .validate(ProtocolLimits::default())
            .expect("each burst envelope must validate");
        let event: AudioPlay = envelope.payload_as().expect("payload must be audio play");
        decisions.push(
            gate.evaluate(&event, 1_800_000_000_000, true)
                .expect("valid audio event must produce a decision"),
        );
    }

    assert_eq!(
        decisions,
        vec![
            AudioDecision::Play,
            AudioDecision::Play,
            AudioDecision::DropRateLimited,
            AudioDecision::DropRateLimited,
        ]
    );
}

#[test]
fn expired_audio_is_dropped_before_resource_lookup() {
    let envelope =
        PayloadEnvelope::decode(&fixture("audio-play-v1.json"), ProtocolLimits::default())
            .expect("fixture must decode");
    let mut event: AudioPlay = envelope.payload_as().expect("payload must be audio play");
    event.expires_at_unix_ms = 1_000;
    let mut gate = AudioClientGate::new(10, 1_000).expect("valid audio limit");
    let decision = gate
        .evaluate(&event, 1_000, true)
        .expect("expired event must produce a drop decision");
    assert_eq!(decision, AudioDecision::DropExpired);
}
