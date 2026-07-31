use mythicraft_persistence::SaveStore;
use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn process_abort_after_backup_rename_recovers_temporary_revision() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("mythicraft-process-crash-{nonce}"));
    let status = Command::new(env!("CARGO_BIN_EXE_persistence-crash-helper"))
        .arg(&root)
        .status()
        .expect("launch crash helper");
    assert!(!status.success());

    let loaded = SaveStore::open(&root)
        .expect("open recovered store")
        .load("crash-player")
        .expect("recover interrupted save");
    assert_eq!(loaded.revision, 2);
    assert_eq!(loaded.state.economy_balance, 20);
    assert!(loaded.recovered_from.is_some());
    fs::remove_dir_all(root).expect("cleanup");
}
