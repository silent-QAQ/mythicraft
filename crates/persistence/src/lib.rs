use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use thiserror::Error;

pub const PLAYER_SCHEMA_VERSION: u32 = 1;
pub const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid JSON at {path}: {source}")]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("invalid state: {0}")]
    Invalid(String),
    #[error("unsupported player schema {0}")]
    UnsupportedSchema(u32),
    #[error("revision conflict: expected {expected}, actual {actual}")]
    RevisionConflict { expected: u64, actual: u64 },
    #[error("player {0} was not found")]
    NotFound(String),
    #[error("fault injected at {0:?}")]
    FaultInjected(FaultPoint),
}

fn io_error(path: &Path, source: std::io::Error) -> PersistenceError {
    PersistenceError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub world: String,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw: f32,
    pub pitch: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ItemStack {
    pub resource_id: String,
    pub count: u16,
    #[serde(default)]
    pub data: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct QuestProgress {
    pub stage: u32,
    #[serde(default)]
    pub counters: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct PermissionCache {
    pub source_revision: String,
    pub expires_at_tick: u64,
    #[serde(default)]
    pub decisions: BTreeMap<String, bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PlayerState {
    pub player_id: String,
    pub position: Position,
    #[serde(default)]
    pub attributes: BTreeMap<String, f64>,
    #[serde(default)]
    pub inventory: Vec<ItemStack>,
    #[serde(default)]
    pub quests: BTreeMap<String, QuestProgress>,
    #[serde(default)]
    pub permission_cache: Option<PermissionCache>,
    pub economy_balance: i64,
    #[serde(default)]
    pub applied_transactions: BTreeSet<String>,
}

impl PlayerState {
    pub fn new(player_id: impl Into<String>) -> Self {
        Self {
            player_id: player_id.into(),
            position: Position {
                world: "minecraft:overworld".into(),
                x: 0.0,
                y: 64.0,
                z: 0.0,
                yaw: 0.0,
                pitch: 0.0,
            },
            attributes: BTreeMap::new(),
            inventory: Vec::new(),
            quests: BTreeMap::new(),
            permission_cache: None,
            economy_balance: 0,
            applied_transactions: BTreeSet::new(),
        }
    }

    pub fn validate(&self) -> Result<(), PersistenceError> {
        validate_id(&self.player_id, "player_id")?;
        validate_resource_id(&self.position.world)?;
        if !self.position.x.is_finite()
            || !self.position.y.is_finite()
            || !self.position.z.is_finite()
            || !self.position.yaw.is_finite()
            || !self.position.pitch.is_finite()
        {
            return Err(PersistenceError::Invalid(
                "position contains a non-finite number".into(),
            ));
        }
        if self.inventory.len() > 256 {
            return Err(PersistenceError::Invalid(
                "inventory exceeds 256 entries".into(),
            ));
        }
        for item in &self.inventory {
            validate_resource_id(&item.resource_id)?;
            if item.count == 0 || item.count > 127 {
                return Err(PersistenceError::Invalid(format!(
                    "item {} count must be in 1..=127",
                    item.resource_id
                )));
            }
        }
        if self.economy_balance < 0 {
            return Err(PersistenceError::Invalid(
                "economy balance cannot be negative".into(),
            ));
        }
        if self.attributes.values().any(|value| !value.is_finite()) {
            return Err(PersistenceError::Invalid(
                "attribute contains a non-finite number".into(),
            ));
        }
        Ok(())
    }
}

fn validate_id(value: &str, field: &str) -> Result<(), PersistenceError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(PersistenceError::Invalid(format!(
            "invalid {field}: {value}"
        )));
    }
    Ok(())
}

fn validate_resource_id(value: &str) -> Result<(), PersistenceError> {
    if value.is_empty()
        || value.len() > 256
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b':' | b'/' | b'_' | b'-' | b'.')
        })
    {
        return Err(PersistenceError::Invalid(format!(
            "invalid resource id: {value}"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlayerEnvelope {
    schema_version: u32,
    revision: u64,
    state: PlayerState,
    checksum: String,
}

#[derive(Serialize)]
struct ChecksumBody<'a> {
    schema_version: u32,
    revision: u64,
    state: &'a PlayerState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoadedPlayer {
    pub revision: u64,
    pub state: PlayerState,
    pub recovered_from: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultPoint {
    BeforeTemporaryWrite,
    AfterTemporarySync,
    AfterBackupRename,
}

pub struct SaveStore {
    root: PathBuf,
    lock_path: PathBuf,
    lock_file: File,
    gate: Mutex<()>,
}

impl SaveStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, PersistenceError> {
        let root = root.into();
        for directory in ["players", "transactions", "audit", "backups"] {
            let path = root.join(directory);
            fs::create_dir_all(&path).map_err(|source| io_error(&path, source))?;
        }
        let lock_path = root.join("persistence.lock");
        let lock_file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|source| io_error(&lock_path, source))?;
        Ok(Self {
            root,
            lock_path,
            lock_file,
            gate: Mutex::new(()),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn load(&self, player_id: &str) -> Result<LoadedPlayer, PersistenceError> {
        validate_id(player_id, "player_id")?;
        let _guard = self
            .gate
            .lock()
            .map_err(|_| PersistenceError::Invalid("save lock poisoned".into()))?;
        let _file_lock = ExclusiveFileLock::acquire(&self.lock_file, &self.lock_path)?;
        self.load_unlocked(player_id)
    }

    pub fn save(
        &self,
        state: &PlayerState,
        expected_revision: Option<u64>,
    ) -> Result<u64, PersistenceError> {
        self.save_with_fault(state, expected_revision, None)
    }

    pub fn save_with_fault(
        &self,
        state: &PlayerState,
        expected_revision: Option<u64>,
        fault: Option<FaultPoint>,
    ) -> Result<u64, PersistenceError> {
        let _guard = self
            .gate
            .lock()
            .map_err(|_| PersistenceError::Invalid("save lock poisoned".into()))?;
        let _file_lock = ExclusiveFileLock::acquire(&self.lock_file, &self.lock_path)?;
        self.save_unlocked(state, expected_revision, fault)
    }

    pub fn create_backup(&self, player_id: &str, label: &str) -> Result<PathBuf, PersistenceError> {
        validate_id(player_id, "player_id")?;
        validate_id(label, "backup label")?;
        let _guard = self
            .gate
            .lock()
            .map_err(|_| PersistenceError::Invalid("save lock poisoned".into()))?;
        let _file_lock = ExclusiveFileLock::acquire(&self.lock_file, &self.lock_path)?;
        let loaded = self.load_unlocked(player_id)?;
        let path = self
            .root
            .join("backups")
            .join(format!("{player_id}-{label}.json"));
        write_atomic(
            &path,
            &encode_envelope(&loaded.state, loaded.revision)?,
            None,
        )?;
        Ok(path)
    }

    pub fn restore_backup(&self, backup: &Path) -> Result<u64, PersistenceError> {
        let _guard = self
            .gate
            .lock()
            .map_err(|_| PersistenceError::Invalid("save lock poisoned".into()))?;
        let _file_lock = ExclusiveFileLock::acquire(&self.lock_file, &self.lock_path)?;
        let bytes = read_limited(backup, 16 * 1024 * 1024)?;
        let envelope = decode_envelope(backup, &bytes)?;
        let expected = self
            .load_unlocked(&envelope.state.player_id)
            .ok()
            .map(|loaded| loaded.revision);
        self.save_unlocked(&envelope.state, expected, None)
    }

    pub fn apply_economy_transaction(
        &self,
        transaction: &EconomyTransaction,
    ) -> Result<TransactionOutcome, PersistenceError> {
        transaction.validate()?;
        let _guard = self
            .gate
            .lock()
            .map_err(|_| PersistenceError::Invalid("save lock poisoned".into()))?;
        let _file_lock = ExclusiveFileLock::acquire(&self.lock_file, &self.lock_path)?;
        let mut loaded = match self.load_unlocked(&transaction.player_id) {
            Ok(loaded) => loaded,
            Err(PersistenceError::NotFound(_)) => LoadedPlayer {
                revision: 0,
                state: PlayerState::new(&transaction.player_id),
                recovered_from: None,
            },
            Err(error) => return Err(error),
        };

        if loaded
            .state
            .applied_transactions
            .contains(&transaction.transaction_id)
        {
            self.finish_pending_audit(transaction)?;
            return Ok(TransactionOutcome::Duplicate {
                balance: loaded.state.economy_balance,
            });
        }

        let before = loaded.state.economy_balance;
        let after = before
            .checked_add(transaction.amount)
            .ok_or_else(|| PersistenceError::Invalid("economy balance overflow".into()))?;
        if after < 0 {
            return Err(PersistenceError::Invalid(
                "transaction would make balance negative".into(),
            ));
        }

        let record = AuditRecord {
            schema_version: 1,
            transaction: transaction.clone(),
            before,
            after,
            resulting_revision: loaded.revision + 1,
        };
        self.write_transaction_marker(&record, false)?;
        loaded.state.economy_balance = after;
        loaded
            .state
            .applied_transactions
            .insert(transaction.transaction_id.clone());
        let revision = self.save_unlocked(&loaded.state, Some(loaded.revision), None)?;
        self.write_audit(&record)?;
        self.write_transaction_marker(&record, true)?;
        Ok(TransactionOutcome::Applied {
            before,
            after,
            revision,
        })
    }

    pub fn audit_record(&self, transaction_id: &str) -> Result<AuditRecord, PersistenceError> {
        validate_id(transaction_id, "transaction_id")?;
        let _guard = self
            .gate
            .lock()
            .map_err(|_| PersistenceError::Invalid("save lock poisoned".into()))?;
        let _file_lock = ExclusiveFileLock::acquire(&self.lock_file, &self.lock_path)?;
        let path = self
            .root
            .join("audit")
            .join(format!("{transaction_id}.json"));
        read_json(&path, 1024 * 1024)
    }

    fn load_unlocked(&self, player_id: &str) -> Result<LoadedPlayer, PersistenceError> {
        let final_path = self.player_path(player_id);
        let candidates = [
            final_path.clone(),
            final_path.with_extension("json.tmp"),
            final_path.with_extension("json.bak"),
        ];
        let mut valid = Vec::new();
        let mut first_error = None;
        let mut found = false;
        for path in candidates {
            if !path.exists() {
                continue;
            }
            found = true;
            match read_limited(&path, 16 * 1024 * 1024)
                .and_then(|bytes| decode_envelope(&path, &bytes))
            {
                Ok(envelope) if envelope.state.player_id == player_id => {
                    valid.push((path, envelope))
                }
                Ok(_) => {
                    first_error.get_or_insert_with(|| {
                        PersistenceError::Invalid("player id mismatch".into())
                    });
                }
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        if valid.is_empty() {
            return Err(if found {
                first_error
                    .unwrap_or_else(|| PersistenceError::Invalid("no valid save candidate".into()))
            } else {
                PersistenceError::NotFound(player_id.into())
            });
        }
        valid.sort_by_key(|(path, envelope)| {
            let source_priority = if *path == final_path { 2 } else { 1 };
            (envelope.revision, source_priority)
        });
        let (source, envelope) = valid.pop().expect("valid is non-empty");
        let recovered_from = (source != final_path).then_some(source.clone());
        if recovered_from.is_some() {
            let recovery = final_path.with_extension("json.recover");
            let bytes = encode_envelope(&envelope.state, envelope.revision)?;
            write_synced(&recovery, &bytes)?;
            if final_path.exists() {
                fs::remove_file(&final_path).map_err(|source| io_error(&final_path, source))?;
            }
            fs::rename(&recovery, &final_path).map_err(|source| io_error(&final_path, source))?;
            if source != final_path
                && source.extension().and_then(|value| value.to_str()) == Some("tmp")
            {
                let _ = fs::remove_file(&source);
            }
        }
        Ok(LoadedPlayer {
            revision: envelope.revision,
            state: envelope.state,
            recovered_from,
        })
    }

    fn save_unlocked(
        &self,
        state: &PlayerState,
        expected_revision: Option<u64>,
        fault: Option<FaultPoint>,
    ) -> Result<u64, PersistenceError> {
        state.validate()?;
        let actual = match self.load_unlocked(&state.player_id) {
            Ok(loaded) => loaded.revision,
            Err(PersistenceError::NotFound(_)) => 0,
            Err(error) => return Err(error),
        };
        if let Some(expected) = expected_revision {
            if expected != actual {
                return Err(PersistenceError::RevisionConflict { expected, actual });
            }
        }
        let revision = actual
            .checked_add(1)
            .ok_or_else(|| PersistenceError::Invalid("revision overflow".into()))?;
        let path = self.player_path(&state.player_id);
        write_atomic(&path, &encode_envelope(state, revision)?, fault)?;
        Ok(revision)
    }

    fn player_path(&self, player_id: &str) -> PathBuf {
        self.root.join("players").join(format!("{player_id}.json"))
    }

    fn write_transaction_marker(
        &self,
        record: &AuditRecord,
        committed: bool,
    ) -> Result<(), PersistenceError> {
        let marker = TransactionMarker {
            committed,
            record: record.clone(),
        };
        let path = self
            .root
            .join("transactions")
            .join(format!("{}.json", record.transaction.transaction_id));
        let bytes =
            serde_json::to_vec_pretty(&marker).map_err(|source| PersistenceError::Json {
                path: path.clone(),
                source,
            })?;
        write_atomic(&path, &bytes, None)
    }

    fn write_audit(&self, record: &AuditRecord) -> Result<(), PersistenceError> {
        let path = self
            .root
            .join("audit")
            .join(format!("{}.json", record.transaction.transaction_id));
        let bytes = serde_json::to_vec_pretty(record).map_err(|source| PersistenceError::Json {
            path: path.clone(),
            source,
        })?;
        write_atomic(&path, &bytes, None)
    }

    fn finish_pending_audit(
        &self,
        transaction: &EconomyTransaction,
    ) -> Result<(), PersistenceError> {
        let marker_path = self
            .root
            .join("transactions")
            .join(format!("{}.json", transaction.transaction_id));
        if marker_path.exists() {
            let marker: TransactionMarker = read_json(&marker_path, 1024 * 1024)?;
            if !marker.committed {
                self.write_audit(&marker.record)?;
                self.write_transaction_marker(&marker.record, true)?;
            }
        }
        Ok(())
    }
}

struct ExclusiveFileLock<'a> {
    file: &'a File,
}

impl<'a> ExclusiveFileLock<'a> {
    fn acquire(file: &'a File, path: &Path) -> Result<Self, PersistenceError> {
        fs2::FileExt::lock_exclusive(file).map_err(|source| io_error(path, source))?;
        Ok(Self { file })
    }
}

impl Drop for ExclusiveFileLock<'_> {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(self.file);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EconomyTransaction {
    pub transaction_id: String,
    pub player_id: String,
    pub amount: i64,
    pub reason: String,
    pub tick: u64,
    pub config_hash: String,
}

impl EconomyTransaction {
    fn validate(&self) -> Result<(), PersistenceError> {
        validate_id(&self.transaction_id, "transaction_id")?;
        validate_id(&self.player_id, "player_id")?;
        if self.amount == 0 {
            return Err(PersistenceError::Invalid(
                "transaction amount cannot be zero".into(),
            ));
        }
        if self.reason.is_empty() || self.reason.len() > 512 {
            return Err(PersistenceError::Invalid(
                "transaction reason length is invalid".into(),
            ));
        }
        if self.config_hash.len() > 128
            || !self
                .config_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(PersistenceError::Invalid(
                "config hash must be hexadecimal".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuditRecord {
    pub schema_version: u32,
    pub transaction: EconomyTransaction,
    pub before: i64,
    pub after: i64,
    pub resulting_revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransactionMarker {
    committed: bool,
    record: AuditRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionOutcome {
    Applied {
        before: i64,
        after: i64,
        revision: u64,
    },
    Duplicate {
        balance: i64,
    },
}

pub struct LastKnownGood<T> {
    current: Option<T>,
    max_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReloadStatus {
    Applied,
    Retained { error: String },
}

impl<T: DeserializeOwned> LastKnownGood<T> {
    pub fn new(max_bytes: u64) -> Self {
        Self {
            current: None,
            max_bytes,
        }
    }

    pub fn current(&self) -> Option<&T> {
        self.current.as_ref()
    }

    pub fn reload_json(&mut self, path: &Path) -> Result<ReloadStatus, PersistenceError> {
        match read_json(path, self.max_bytes) {
            Ok(value) => {
                self.current = Some(value);
                Ok(ReloadStatus::Applied)
            }
            Err(error) if self.current.is_some() => Ok(ReloadStatus::Retained {
                error: error.to_string(),
            }),
            Err(error) => Err(error),
        }
    }
}

fn encode_envelope(state: &PlayerState, revision: u64) -> Result<Vec<u8>, PersistenceError> {
    let body = ChecksumBody {
        schema_version: PLAYER_SCHEMA_VERSION,
        revision,
        state,
    };
    let body_bytes = serde_json::to_vec(&body).map_err(|source| PersistenceError::Json {
        path: PathBuf::from("<memory>"),
        source,
    })?;
    let checksum = hex::encode(Sha256::digest(body_bytes));
    let envelope = PlayerEnvelope {
        schema_version: PLAYER_SCHEMA_VERSION,
        revision,
        state: state.clone(),
        checksum,
    };
    serde_json::to_vec_pretty(&envelope).map_err(|source| PersistenceError::Json {
        path: PathBuf::from("<memory>"),
        source,
    })
}

fn decode_envelope(path: &Path, bytes: &[u8]) -> Result<PlayerEnvelope, PersistenceError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|source| PersistenceError::Json {
            path: path.to_path_buf(),
            source,
        })?;
    let schema = value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            PersistenceError::Invalid(format!("{} has no schema_version", path.display()))
        })? as u32;
    let envelope = match schema {
        PLAYER_SCHEMA_VERSION => {
            serde_json::from_value::<PlayerEnvelope>(value).map_err(|source| {
                PersistenceError::Json {
                    path: path.to_path_buf(),
                    source,
                }
            })?
        }
        0 => migrate_v0(path, value)?,
        other => return Err(PersistenceError::UnsupportedSchema(other)),
    };
    envelope.state.validate()?;
    let expected = encode_envelope(&envelope.state, envelope.revision)?;
    let expected: PlayerEnvelope =
        serde_json::from_slice(&expected).map_err(|source| PersistenceError::Json {
            path: path.to_path_buf(),
            source,
        })?;
    if envelope.checksum != expected.checksum {
        return Err(PersistenceError::Invalid(format!(
            "checksum mismatch at {}",
            path.display()
        )));
    }
    Ok(envelope)
}

#[derive(Deserialize)]
struct LegacyEnvelope {
    revision: u64,
    state: LegacyPlayerState,
}

#[derive(Deserialize)]
struct LegacyPlayerState {
    player_id: String,
    world: String,
    x: f64,
    y: f64,
    z: f64,
    economy_balance: i64,
}

fn migrate_v0(path: &Path, value: serde_json::Value) -> Result<PlayerEnvelope, PersistenceError> {
    let legacy: LegacyEnvelope =
        serde_json::from_value(value).map_err(|source| PersistenceError::Json {
            path: path.to_path_buf(),
            source,
        })?;
    let mut state = PlayerState::new(legacy.state.player_id);
    state.position.world = legacy.state.world;
    state.position.x = legacy.state.x;
    state.position.y = legacy.state.y;
    state.position.z = legacy.state.z;
    state.economy_balance = legacy.state.economy_balance;
    let bytes = encode_envelope(&state, legacy.revision)?;
    serde_json::from_slice(&bytes).map_err(|source| PersistenceError::Json {
        path: path.to_path_buf(),
        source,
    })
}

fn read_limited(path: &Path, max_bytes: u64) -> Result<Vec<u8>, PersistenceError> {
    let metadata = fs::metadata(path).map_err(|source| io_error(path, source))?;
    if metadata.len() > max_bytes {
        return Err(PersistenceError::Invalid(format!(
            "{} exceeds {max_bytes} bytes",
            path.display()
        )));
    }
    let mut file = File::open(path).map_err(|source| io_error(path, source))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut bytes)
        .map_err(|source| io_error(path, source))?;
    Ok(bytes)
}

fn read_json<T: DeserializeOwned>(path: &Path, max_bytes: u64) -> Result<T, PersistenceError> {
    let bytes = read_limited(path, max_bytes)?;
    serde_json::from_slice(&bytes).map_err(|source| PersistenceError::Json {
        path: path.to_path_buf(),
        source,
    })
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), PersistenceError> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .map_err(|source| io_error(path, source))?;
    file.write_all(bytes)
        .map_err(|source| io_error(path, source))?;
    file.sync_all().map_err(|source| io_error(path, source))
}

fn write_atomic(
    path: &Path,
    bytes: &[u8],
    fault: Option<FaultPoint>,
) -> Result<(), PersistenceError> {
    if fault == Some(FaultPoint::BeforeTemporaryWrite) {
        return Err(PersistenceError::FaultInjected(
            FaultPoint::BeforeTemporaryWrite,
        ));
    }
    let temporary = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("data")
    ));
    let backup = path.with_extension(format!(
        "{}.bak",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("data")
    ));
    write_synced(&temporary, bytes)?;
    if fault == Some(FaultPoint::AfterTemporarySync) {
        return Err(PersistenceError::FaultInjected(
            FaultPoint::AfterTemporarySync,
        ));
    }
    if path.exists() {
        if backup.exists() {
            fs::remove_file(&backup).map_err(|source| io_error(&backup, source))?;
        }
        fs::rename(path, &backup).map_err(|source| io_error(path, source))?;
    }
    if fault == Some(FaultPoint::AfterBackupRename) {
        return Err(PersistenceError::FaultInjected(
            FaultPoint::AfterBackupRename,
        ));
    }
    fs::rename(&temporary, path).map_err(|source| io_error(path, source))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mythicraft-{name}-{nonce}"));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn atomic_save_recovers_highest_valid_candidate() {
        let root = temp_dir("recovery");
        let store = SaveStore::open(&root).expect("open store");
        let mut state = PlayerState::new("player-1");
        state.economy_balance = 10;
        assert_eq!(store.save(&state, None).expect("initial save"), 1);
        state.economy_balance = 25;
        let error = store
            .save_with_fault(&state, Some(1), Some(FaultPoint::AfterBackupRename))
            .expect_err("fault");
        assert!(matches!(
            error,
            PersistenceError::FaultInjected(FaultPoint::AfterBackupRename)
        ));
        let loaded = store.load("player-1").expect("recover save");
        assert_eq!(loaded.revision, 2);
        assert_eq!(loaded.state.economy_balance, 25);
        assert!(loaded.recovered_from.is_some());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn failed_temporary_write_preserves_previous_revision() {
        let root = temp_dir("disk-full");
        let store = SaveStore::open(&root).expect("open store");
        let mut state = PlayerState::new("player-1");
        state.economy_balance = 10;
        store.save(&state, None).expect("initial save");
        state.economy_balance = 20;
        assert!(matches!(
            store.save_with_fault(&state, Some(1), Some(FaultPoint::BeforeTemporaryWrite)),
            Err(PersistenceError::FaultInjected(
                FaultPoint::BeforeTemporaryWrite
            ))
        ));
        let loaded = store.load("player-1").expect("load original");
        assert_eq!(loaded.revision, 1);
        assert_eq!(loaded.state.economy_balance, 10);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn half_written_primary_recovers_the_valid_backup() {
        let root = temp_dir("half-write");
        let store = SaveStore::open(&root).expect("open store");
        let mut state = PlayerState::new("player-1");
        state.economy_balance = 10;
        store.save(&state, None).expect("initial save");
        state.economy_balance = 20;
        store.save(&state, Some(1)).expect("second save");
        fs::write(root.join("players/player-1.json"), b"{\"schema_version\":1")
            .expect("truncate primary");
        let loaded = store.load("player-1").expect("recover backup");
        assert_eq!(loaded.revision, 1);
        assert_eq!(loaded.state.economy_balance, 10);
        assert!(loaded.recovered_from.is_some());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn economy_transaction_is_idempotent_and_audited() {
        let root = temp_dir("economy");
        let store = SaveStore::open(&root).expect("open store");
        let transaction = EconomyTransaction {
            transaction_id: "reward-001".into(),
            player_id: "player-1".into(),
            amount: 50,
            reason: "quest reward".into(),
            tick: 42,
            config_hash: "aabbcc".into(),
        };
        assert!(matches!(
            store
                .apply_economy_transaction(&transaction)
                .expect("apply"),
            TransactionOutcome::Applied { after: 50, .. }
        ));
        assert_eq!(
            store
                .apply_economy_transaction(&transaction)
                .expect("retry"),
            TransactionOutcome::Duplicate { balance: 50 }
        );
        assert_eq!(store.audit_record("reward-001").expect("audit").after, 50);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn duplicate_retry_repairs_a_pending_audit() {
        let root = temp_dir("audit-repair");
        let store = SaveStore::open(&root).expect("open store");
        let transaction = EconomyTransaction {
            transaction_id: "reward-001".into(),
            player_id: "player-1".into(),
            amount: 50,
            reason: "quest reward".into(),
            tick: 42,
            config_hash: "aabbcc".into(),
        };
        store
            .apply_economy_transaction(&transaction)
            .expect("apply transaction");
        let audit_path = root.join("audit/reward-001.json");
        fs::remove_file(&audit_path).expect("remove audit");
        let marker_path = root.join("transactions/reward-001.json");
        let mut marker: TransactionMarker = read_json(&marker_path, 1024 * 1024).expect("marker");
        marker.committed = false;
        let marker_bytes = serde_json::to_vec_pretty(&marker).expect("encode marker");
        write_atomic(&marker_path, &marker_bytes, None).expect("write pending marker");

        assert_eq!(
            store
                .apply_economy_transaction(&transaction)
                .expect("retry transaction"),
            TransactionOutcome::Duplicate { balance: 50 }
        );
        assert_eq!(
            store
                .audit_record("reward-001")
                .expect("repaired audit")
                .after,
            50
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn concurrent_reconnect_transactions_are_serialized() {
        let root = temp_dir("concurrent-reconnect");
        let stores = (0..4)
            .map(|_| Arc::new(SaveStore::open(&root).expect("open store")))
            .collect::<Vec<_>>();
        let mut handles = Vec::new();
        for index in 0..16 {
            let store = Arc::clone(&stores[index as usize % stores.len()]);
            handles.push(thread::spawn(move || {
                store.apply_economy_transaction(&EconomyTransaction {
                    transaction_id: format!("reconnect-{index}"),
                    player_id: "player-1".into(),
                    amount: 1,
                    reason: "concurrent reconnect reward".into(),
                    tick: index,
                    config_hash: "00".into(),
                })
            }));
        }
        for handle in handles {
            assert!(matches!(
                handle.join().expect("thread join").expect("transaction"),
                TransactionOutcome::Applied { .. }
            ));
        }
        let loaded = stores[0].load("player-1").expect("load final state");
        assert_eq!(loaded.state.economy_balance, 16);
        assert_eq!(loaded.state.applied_transactions.len(), 16);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn old_schema_is_migrated_on_read() {
        let root = temp_dir("migration");
        let store = SaveStore::open(&root).expect("open store");
        let path = root.join("players/player-1.json");
        fs::write(&path, br#"{
          "schema_version": 0,
          "revision": 7,
          "state": {"player_id":"player-1","world":"minecraft:overworld","x":1.0,"y":70.0,"z":2.0,"economy_balance":9}
        }"#).expect("write legacy");
        let loaded = store.load("player-1").expect("load legacy");
        assert_eq!(loaded.revision, 7);
        assert_eq!(loaded.state.position.y, 70.0);
        assert_eq!(loaded.state.economy_balance, 9);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn invalid_reload_retains_last_known_good_configuration() {
        let root = temp_dir("config");
        let path = root.join("config.json");
        fs::write(&path, r#"{"enabled":true}"#).expect("write config");
        let mut config = LastKnownGood::<BTreeMap<String, bool>>::new(MAX_CONFIG_BYTES);
        assert_eq!(
            config.reload_json(&path).expect("load config"),
            ReloadStatus::Applied
        );
        fs::write(&path, "{").expect("break config");
        assert!(matches!(
            config.reload_json(&path).expect("retain config"),
            ReloadStatus::Retained { .. }
        ));
        assert_eq!(
            config.current().and_then(|value| value.get("enabled")),
            Some(&true)
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn stale_revision_and_negative_balance_are_rejected() {
        let root = temp_dir("guards");
        let store = SaveStore::open(&root).expect("open store");
        let state = PlayerState::new("player-1");
        store.save(&state, None).expect("save");
        assert!(matches!(
            store.save(&state, Some(0)),
            Err(PersistenceError::RevisionConflict { .. })
        ));
        let debit = EconomyTransaction {
            transaction_id: "debit-1".into(),
            player_id: "player-1".into(),
            amount: -1,
            reason: "purchase".into(),
            tick: 1,
            config_hash: "00".into(),
        };
        assert!(matches!(
            store.apply_economy_transaction(&debit),
            Err(PersistenceError::Invalid(_))
        ));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn economy_overflow_fails_before_state_or_audit_changes() {
        let root = temp_dir("overflow");
        let store = SaveStore::open(&root).expect("open store");
        let mut state = PlayerState::new("player-1");
        state.economy_balance = i64::MAX;
        store.save(&state, None).expect("save maximum balance");
        let transaction = EconomyTransaction {
            transaction_id: "overflow-1".into(),
            player_id: "player-1".into(),
            amount: 1,
            reason: "overflow regression".into(),
            tick: 1,
            config_hash: "00".into(),
        };
        assert!(matches!(
            store.apply_economy_transaction(&transaction),
            Err(PersistenceError::Invalid(_))
        ));
        assert_eq!(
            store
                .load("player-1")
                .expect("load unchanged state")
                .state
                .economy_balance,
            i64::MAX
        );
        assert!(!root.join("audit/overflow-1.json").exists());
        assert!(!root.join("transactions/overflow-1.json").exists());
        fs::remove_dir_all(root).expect("cleanup");
    }
}
