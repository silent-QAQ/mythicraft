use mythicraft_integration_harness::IntegrationRunner;
use mythicraft_observability::{StageResult, StageStatus};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn failed_stage_is_written_before_error_returns() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("mythicraft-reporting-{nonce}"));
    let report = root.join("run.jsonl");
    let mut runner = IntegrationRunner::open(&report, "failure-run").expect("open runner");
    assert!(runner
        .stage::<(), _>("world_load", || Err("synthetic world failure"))
        .is_err());
    let line = fs::read_to_string(&report).expect("read report");
    let result: StageResult = serde_json::from_str(line.trim()).expect("decode result");
    assert_eq!(result.status, StageStatus::Failed);
    assert_eq!(result.stage, "world_load");
    assert!(result.message.contains("synthetic world failure"));
    fs::remove_dir_all(root).expect("cleanup");
}
