use mythicraft_persistence::{PersistenceError, SaveStore};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture(path: &str) -> Vec<u8> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    fs::read(root.join(path)).expect("read fixture")
}

fn temp_dir(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("mythicraft-security-{name}-{nonce}"));
    fs::create_dir_all(path.join("players")).expect("create player directory");
    path
}

#[test]
fn corrupt_checksum_and_unknown_schema_fail_closed() {
    for (name, fixture_name, expected) in [
        (
            "checksum",
            "persistence/player-v1-corrupt-checksum.json",
            "checksum",
        ),
        (
            "schema",
            "persistence/player-v99-unsupported.json",
            "schema",
        ),
    ] {
        let root = temp_dir(name);
        fs::write(
            root.join("players/fixture-player.json"),
            fixture(fixture_name),
        )
        .expect("write fixture save");
        let error = SaveStore::open(&root)
            .expect("open store")
            .load("fixture-player")
            .expect_err("invalid save must fail");
        match expected {
            "checksum" => assert!(matches!(error, PersistenceError::Invalid(_))),
            "schema" => assert!(matches!(error, PersistenceError::UnsupportedSchema(99))),
            _ => unreachable!(),
        }
        fs::remove_dir_all(root).expect("cleanup");
    }
}
