use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClientLoader {
    Pending,
    Fabric,
    NeoForge,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClientVersion {
    pub loader: ClientLoader,
    pub loader_version: Option<String>,
    pub mod_version: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DataVersionRange {
    pub minimum: i32,
    pub maximum: i32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VersionMatrix {
    pub schema_version: u32,
    pub minecraft_version: String,
    pub protocol_version: i32,
    pub data_version: DataVersionRange,
    pub registry_sha256: String,
    pub client: ClientVersion,
}

impl VersionMatrix {
    pub fn validate(&self) -> Result<(), VersionMatrixError> {
        if self.schema_version == 0 {
            return Err(VersionMatrixError::InvalidSchemaVersion);
        }
        if self.minecraft_version.trim().is_empty() {
            return Err(VersionMatrixError::MissingMinecraftVersion);
        }
        if self.protocol_version <= 0 {
            return Err(VersionMatrixError::InvalidProtocolVersion(
                self.protocol_version,
            ));
        }
        if self.data_version.minimum > self.data_version.maximum {
            return Err(VersionMatrixError::InvalidDataVersionRange {
                minimum: self.data_version.minimum,
                maximum: self.data_version.maximum,
            });
        }
        if self.registry_sha256.len() != 64
            || !self
                .registry_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(VersionMatrixError::InvalidRegistryHash);
        }
        if self.client.loader == ClientLoader::Pending {
            return Err(VersionMatrixError::ClientLoaderNotFrozen);
        }
        if self
            .client
            .loader_version
            .as_deref()
            .map(str::trim)
            .is_none_or(str::is_empty)
        {
            return Err(VersionMatrixError::MissingClientLoaderVersion);
        }
        if self.client.mod_version.trim().is_empty() {
            return Err(VersionMatrixError::MissingClientModVersion);
        }
        Ok(())
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum VersionMatrixError {
    #[error("version matrix schema_version must be greater than zero")]
    InvalidSchemaVersion,
    #[error("Minecraft version is missing")]
    MissingMinecraftVersion,
    #[error("protocol version must be positive, got {0}")]
    InvalidProtocolVersion(i32),
    #[error("DataVersion range is invalid: minimum {minimum} exceeds maximum {maximum}")]
    InvalidDataVersionRange { minimum: i32, maximum: i32 },
    #[error("registry_sha256 must contain exactly 64 hexadecimal characters")]
    InvalidRegistryHash,
    #[error("client Mod loader has not been frozen by Window 3")]
    ClientLoaderNotFrozen,
    #[error("client Mod loader version is missing")]
    MissingClientLoaderVersion,
    #[error("client Mod version is missing")]
    MissingClientModVersion,
}

#[cfg(test)]
mod tests {
    use super::{ClientLoader, ClientVersion, DataVersionRange, VersionMatrix, VersionMatrixError};

    fn valid_matrix() -> VersionMatrix {
        VersionMatrix {
            schema_version: 1,
            minecraft_version: "26.2".to_owned(),
            protocol_version: 776,
            data_version: DataVersionRange {
                minimum: 4903,
                maximum: 4903,
            },
            registry_sha256: "3ffaca442dbbd1d9acb2b7bf2509cbd80e30dbc5349dfbad39eda7f4e6bd5a8b"
                .to_owned(),
            client: ClientVersion {
                loader: ClientLoader::Fabric,
                loader_version: Some("contract-test".to_owned()),
                mod_version: "0.1.0-dev".to_owned(),
            },
        }
    }

    #[test]
    fn accepts_complete_matrix() {
        assert_eq!(valid_matrix().validate(), Ok(()));
    }

    #[test]
    fn rejects_unfrozen_client_loader() {
        let mut matrix = valid_matrix();
        matrix.client.loader = ClientLoader::Pending;
        matrix.client.loader_version = None;

        assert_eq!(
            matrix.validate(),
            Err(VersionMatrixError::ClientLoaderNotFrozen)
        );
    }

    #[test]
    fn rejects_invalid_registry_hash() {
        let mut matrix = valid_matrix();
        matrix.registry_sha256 = "not-a-sha256".to_owned();

        assert_eq!(
            matrix.validate(),
            Err(VersionMatrixError::InvalidRegistryHash)
        );
    }
}
