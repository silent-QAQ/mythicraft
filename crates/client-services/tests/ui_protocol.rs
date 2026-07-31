use std::{fs, path::PathBuf};

use mythicraft_client_services::{
    MessageType, PayloadEnvelope, ProtocolLimits, UiAction, UiActionContext, UiActionError,
    UiActionGate, UiActionType, UiInputValue, UiOpen, UiRun,
};

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/client")
        .join(name);
    fs::read(path).expect("fixture must be readable")
}

fn decoded_action(name: &str) -> (PayloadEnvelope, UiAction) {
    let envelope = PayloadEnvelope::decode(&fixture(name), ProtocolLimits::default())
        .expect("action fixture must decode");
    let action = envelope.payload_as().expect("payload must be a UI action");
    (envelope, action)
}

fn context<'a>(nonce: &'a str) -> UiActionContext<'a> {
    UiActionContext {
        now_unix_ms: 1_800_000_000_000,
        expected_page_version: 3,
        expected_nonce: nonce,
        permission_granted: true,
        in_range: true,
        state_allowed: true,
    }
}

fn action_envelope(request_id: &str, nonce: &str) -> (PayloadEnvelope, UiAction) {
    let action = UiAction {
        page_id: "quest.dialogue".to_owned(),
        control_id: "accept".to_owned(),
        action_type: UiActionType::Submit,
        page_version: 3,
        nonce: nonce.to_owned(),
        expires_at_unix_ms: 1_900_000_000_000,
        request_id: request_id.to_owned(),
        input: Some(UiInputValue::Text("accepted".to_owned())),
    };
    let envelope = PayloadEnvelope::new(
        MessageType::UiAction,
        request_id,
        Some(nonce.to_owned()),
        Some(action.expires_at_unix_ms),
        &action,
    )
    .expect("test action must serialize");
    (envelope, action)
}

#[test]
fn decodes_ui_open_golden_fixture() {
    let envelope = PayloadEnvelope::decode(&fixture("ui-open-v1.json"), ProtocolLimits::default())
        .expect("UI open fixture must decode");
    assert_eq!(envelope.message_type, MessageType::UiOpen);
    let open: UiOpen = envelope.payload_as().expect("payload must be UI open");
    open.validate(ProtocolLimits::default())
        .expect("UI open must satisfy limits");
    assert_eq!(open.page_id, "quest.dialogue");
    assert_eq!(open.page_version, 3);
}

#[test]
fn decodes_server_emitted_arcartx_ui_run_fixture() {
    let envelope = PayloadEnvelope::decode(&fixture("ui-run-v1.json"), ProtocolLimits::default())
        .expect("UI run fixture must decode");
    assert_eq!(envelope.message_type, MessageType::UiRun);
    let run: UiRun = envelope.payload_as().expect("payload must be a UI run");
    run.validate(ProtocolLimits::default())
        .expect("Unicode ArcartX page IDs and UI code must be valid");
    assert_eq!(run.page_id, "任务面板");
}

#[test]
fn accepts_action_once_and_rejects_replay() {
    let (envelope, action) = decoded_action("ui-action-v1.json");
    let mut gate = UiActionGate::new(10, 1_000).expect("valid rate limit");
    gate.validate_and_record(
        &envelope,
        &action,
        context(&action.nonce),
        ProtocolLimits::default(),
    )
    .expect("first action must be accepted");
    let error = gate
        .validate_and_record(
            &envelope,
            &action,
            context(&action.nonce),
            ProtocolLimits::default(),
        )
        .expect_err("replayed action must be rejected");
    assert!(matches!(error, UiActionError::DuplicateRequest));
}

#[test]
fn rejects_expired_action_fixture() {
    let (envelope, action) = decoded_action("ui-action-expired-v1.json");
    let mut gate = UiActionGate::new(10, 1_000).expect("valid rate limit");
    let expired_context = UiActionContext {
        now_unix_ms: 1_000,
        expected_page_version: 3,
        expected_nonce: &action.nonce,
        permission_granted: true,
        in_range: true,
        state_allowed: true,
    };
    let error = gate
        .validate_and_record(
            &envelope,
            &action,
            expired_context,
            ProtocolLimits::default(),
        )
        .expect_err("expired action must be rejected");
    assert!(matches!(error, UiActionError::Expired));
}

#[test]
fn rejects_stale_page_version_and_nonce_mismatch() {
    let (envelope, action) = decoded_action("ui-action-v1.json");
    let mut gate = UiActionGate::new(10, 1_000).expect("valid rate limit");
    let stale = UiActionContext {
        expected_page_version: 4,
        ..context(&action.nonce)
    };
    let error = gate
        .validate_and_record(&envelope, &action, stale, ProtocolLimits::default())
        .expect_err("stale page version must be rejected");
    assert!(matches!(error, UiActionError::StalePageVersion { .. }));

    let wrong_nonce = context("different-nonce");
    let error = gate
        .validate_and_record(&envelope, &action, wrong_nonce, ProtocolLimits::default())
        .expect_err("wrong page nonce must be rejected");
    assert!(matches!(error, UiActionError::InvalidNonce));
}

#[test]
fn enforces_server_permission_distance_and_state_checks() {
    let (envelope, action) = decoded_action("ui-action-v1.json");
    let checks = [
        (
            UiActionContext {
                permission_granted: false,
                ..context(&action.nonce)
            },
            "permission",
        ),
        (
            UiActionContext {
                in_range: false,
                ..context(&action.nonce)
            },
            "distance",
        ),
        (
            UiActionContext {
                state_allowed: false,
                ..context(&action.nonce)
            },
            "state",
        ),
    ];

    for (validation_context, expected) in checks {
        let mut gate = UiActionGate::new(10, 1_000).expect("valid rate limit");
        let error = gate
            .validate_and_record(
                &envelope,
                &action,
                validation_context,
                ProtocolLimits::default(),
            )
            .expect_err("server authority check must reject action");
        match expected {
            "permission" => assert!(matches!(error, UiActionError::PermissionDenied)),
            "distance" => assert!(matches!(error, UiActionError::OutOfRange)),
            "state" => assert!(matches!(error, UiActionError::InvalidState)),
            _ => unreachable!(),
        }
    }
}

#[test]
fn rejects_action_detached_from_envelope_payload() {
    let (envelope, mut action) = decoded_action("ui-action-v1.json");
    action.control_id = "forged-control".to_owned();
    let mut gate = UiActionGate::new(10, 1_000).expect("valid rate limit");
    let error = gate
        .validate_and_record(
            &envelope,
            &action,
            context(&action.nonce),
            ProtocolLimits::default(),
        )
        .expect_err("detached action must be rejected");
    assert!(matches!(error, UiActionError::PayloadMismatch));
}

#[test]
fn rate_limits_actions_per_gate_window() {
    let mut gate = UiActionGate::new(2, 1_000).expect("valid rate limit");
    for request_id in ["action-1", "action-2"] {
        let (envelope, action) = action_envelope(request_id, "rate-nonce");
        gate.validate_and_record(
            &envelope,
            &action,
            context("rate-nonce"),
            ProtocolLimits::default(),
        )
        .expect("action within limit must pass");
    }
    let (envelope, action) = action_envelope("action-3", "rate-nonce");
    let error = gate
        .validate_and_record(
            &envelope,
            &action,
            context("rate-nonce"),
            ProtocolLimits::default(),
        )
        .expect_err("third action in window must be rejected");
    assert!(matches!(error, UiActionError::RateLimited));
}
