use std::{fs, path::Path};

use mythicraft_api::{VersionMatrix, VersionMatrixError};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const TARGET_MINECRAFT_VERSION: &str = "26.2";
pub const TARGET_PROTOCOL_VERSION: i32 = 776;
pub const TARGET_DATA_VERSION: i32 = 4903;
pub const TARGET_REGISTRY_SHA256: &str =
    "3ffaca442dbbd1d9acb2b7bf2509cbd80e30dbc5349dfbad39eda7f4e6bd5a8b";

pub fn load_version_matrix(path: &Path) -> Result<VersionMatrix, VanillaDataError> {
    let bytes = fs::read(path).map_err(|source| VanillaDataError::ReadMatrix {
        path: path.to_path_buf(),
        source,
    })?;
    let matrix =
        serde_json::from_slice::<VersionMatrix>(&bytes).map_err(VanillaDataError::ParseMatrix)?;
    matrix.validate().map_err(VanillaDataError::InvalidMatrix)?;
    validate_target(&matrix)?;
    Ok(matrix)
}

pub fn validate_registry_artifact(bytes: &[u8]) -> Result<(), VanillaDataError> {
    let actual = hex::encode(Sha256::digest(bytes));
    if actual != TARGET_REGISTRY_SHA256 {
        return Err(VanillaDataError::RegistryHashMismatch {
            expected: TARGET_REGISTRY_SHA256,
            actual,
        });
    }
    Ok(())
}

fn validate_target(matrix: &VersionMatrix) -> Result<(), VanillaDataError> {
    if matrix.minecraft_version != TARGET_MINECRAFT_VERSION {
        return Err(VanillaDataError::UnsupportedMinecraftVersion(
            matrix.minecraft_version.clone(),
        ));
    }
    if matrix.protocol_version != TARGET_PROTOCOL_VERSION {
        return Err(VanillaDataError::ProtocolVersionMismatch {
            expected: TARGET_PROTOCOL_VERSION,
            actual: matrix.protocol_version,
        });
    }
    if matrix.data_version.minimum != TARGET_DATA_VERSION
        || matrix.data_version.maximum != TARGET_DATA_VERSION
    {
        return Err(VanillaDataError::DataVersionMismatch {
            expected: TARGET_DATA_VERSION,
            minimum: matrix.data_version.minimum,
            maximum: matrix.data_version.maximum,
        });
    }
    if matrix.registry_sha256 != TARGET_REGISTRY_SHA256 {
        return Err(VanillaDataError::PinnedRegistryHashMismatch {
            expected: TARGET_REGISTRY_SHA256,
            actual: matrix.registry_sha256.clone(),
        });
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum VanillaDataError {
    #[error("failed to read version matrix {path}: {source}")]
    ReadMatrix {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse version matrix: {0}")]
    ParseMatrix(serde_json::Error),
    #[error("invalid version matrix: {0}")]
    InvalidMatrix(VersionMatrixError),
    #[error("unsupported Minecraft version {0}")]
    UnsupportedMinecraftVersion(String),
    #[error("protocol version mismatch: expected {expected}, got {actual}")]
    ProtocolVersionMismatch { expected: i32, actual: i32 },
    #[error(
        "DataVersion mismatch: expected {expected}, got inclusive range {minimum}..={maximum}"
    )]
    DataVersionMismatch {
        expected: i32,
        minimum: i32,
        maximum: i32,
    },
    #[error("pinned registry hash mismatch: expected {expected}, got {actual}")]
    PinnedRegistryHashMismatch {
        expected: &'static str,
        actual: String,
    },
    #[error("registry artifact hash mismatch: expected {expected}, got {actual}")]
    RegistryHashMismatch {
        expected: &'static str,
        actual: String,
    },
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{load_version_matrix, validate_registry_artifact, VanillaDataError};

    #[test]
    fn missing_fixture_has_path_diagnostic() {
        let path = Path::new("../../fixtures/version/does-not-exist.json");
        let result = load_version_matrix(path);

        assert!(matches!(result, Err(VanillaDataError::ReadMatrix { .. })));
    }

    #[test]
    fn draft_fixture_is_rejected_until_client_contract_is_frozen() {
        let path = Path::new("../../fixtures/version/26.2-draft.json");
        let result = load_version_matrix(path);

        assert!(matches!(result, Err(VanillaDataError::InvalidMatrix(_))));
    }

    #[test]
    fn registry_hash_mismatch_is_rejected() {
        let result = validate_registry_artifact(b"not the pinned registry artifact");

        assert!(matches!(
            result,
            Err(VanillaDataError::RegistryHashMismatch { .. })
        ));
    }
}
