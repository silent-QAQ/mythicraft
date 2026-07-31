use mythicraft_persistence::{FaultPoint, PlayerState, SaveStore};
use std::path::PathBuf;

fn main() {
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .expect("save root argument");
    let store = SaveStore::open(root).expect("open store");
    let mut state = PlayerState::new("crash-player");
    state.economy_balance = 10;
    store.save(&state, None).expect("initial save");
    state.economy_balance = 20;
    store
        .save_with_fault(&state, Some(1), Some(FaultPoint::AfterBackupRename))
        .expect_err("fault must interrupt save");
    std::process::abort();
}
