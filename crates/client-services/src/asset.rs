use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::ProtocolLimits;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetLimits {
    pub protocol: ProtocolLimits,
    pub max_assets: usize,
    pub max_asset_bytes: u64,
    pub max_total_bytes: u64,
}

impl Default for AssetLimits {
    fn default() -> Self {
        Self {
            protocol: ProtocolLimits::default(),
            max_assets: 4_096,
            max_asset_bytes: 64 * 1024 * 1024,
            max_total_bytes: 512 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetType {
    Texture,
    Font,
    Model,
    Sound,
    UiLayout,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetLicense {
    pub source: String,
    pub license: String,
    pub redistribution_allowed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetEntry {
    pub resource_id: String,
    pub asset_type: AssetType,
    pub path: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub version: String,
    pub license: AssetLicense,
}

impl AssetEntry {
    pub fn validate(&self, limits: AssetLimits) -> Result<(), AssetError> {
        validate_resource_id(&self.resource_id)?;
        validate_relative_asset_path(&self.path)?;
        validate_sha256(&self.sha256)?;
        validate_version(&self.version)?;
        validate_text("license.source", &self.license.source, 256)?;
        validate_text("license.license", &self.license.license, 128)?;
        if !self.license.redistribution_allowed {
            return Err(AssetError::RedistributionNotAllowed(
                self.resource_id.clone(),
            ));
        }
        if self.size_bytes == 0 || self.size_bytes > limits.max_asset_bytes {
            return Err(AssetError::InvalidAssetSize {
                resource_id: self.resource_id.clone(),
                actual: self.size_bytes,
                maximum: limits.max_asset_bytes,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetManifest {
    pub manifest_version: u32,
    pub manifest_hash: String,
    pub assets: Vec<AssetEntry>,
}

#[derive(Serialize)]
struct ManifestHashBody<'a> {
    manifest_version: u32,
    assets: &'a [AssetEntry],
}

impl AssetManifest {
    pub fn validate(&self, limits: AssetLimits) -> Result<(), AssetError> {
        if self.manifest_version == 0 {
            return Err(AssetError::InvalidManifestVersion);
        }
        validate_sha256(&self.manifest_hash)?;
        if self.assets.len() > limits.max_assets {
            return Err(AssetError::TooManyAssets {
                actual: self.assets.len(),
                maximum: limits.max_assets,
            });
        }

        let mut ids = BTreeSet::new();
        let mut total_bytes = 0_u64;
        for asset in &self.assets {
            asset.validate(limits)?;
            if !ids.insert(&asset.resource_id) {
                return Err(AssetError::DuplicateResourceId(asset.resource_id.clone()));
            }
            total_bytes = total_bytes
                .checked_add(asset.size_bytes)
                .ok_or(AssetError::TotalSizeOverflow)?;
            if total_bytes > limits.max_total_bytes {
                return Err(AssetError::ManifestTooLarge {
                    actual: total_bytes,
                    maximum: limits.max_total_bytes,
                });
            }
        }

        let computed = self.computed_hash()?;
        if computed != self.manifest_hash.to_ascii_lowercase() {
            return Err(AssetError::ManifestHashMismatch {
                declared: self.manifest_hash.clone(),
                computed,
            });
        }
        Ok(())
    }

    pub fn computed_hash(&self) -> Result<String, AssetError> {
        let mut assets = self.assets.clone();
        assets.sort_by(|left, right| left.resource_id.cmp(&right.resource_id));
        let bytes = serde_json::to_vec(&ManifestHashBody {
            manifest_version: self.manifest_version,
            assets: &assets,
        })?;
        let digest = Sha256::digest(bytes);
        Ok(to_lower_hex(&digest))
    }

    pub fn find(&self, resource_id: &str) -> Option<&AssetEntry> {
        self.assets
            .iter()
            .find(|asset| asset.resource_id == resource_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetRequest {
    pub manifest_hash: String,
    pub resource_ids: Vec<String>,
}

impl AssetRequest {
    pub fn validate(&self, limits: AssetLimits) -> Result<(), AssetError> {
        validate_sha256(&self.manifest_hash)?;
        if self.resource_ids.is_empty() || self.resource_ids.len() > limits.protocol.max_array_items
        {
            return Err(AssetError::InvalidRequestCount {
                actual: self.resource_ids.len(),
                maximum: limits.protocol.max_array_items,
            });
        }
        let mut unique = BTreeSet::new();
        for resource_id in &self.resource_ids {
            validate_resource_id(resource_id)?;
            if !unique.insert(resource_id) {
                return Err(AssetError::DuplicateResourceId(resource_id.clone()));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetResultStatus {
    Ready,
    Missing,
    HashMismatch,
    NotAuthorized,
    UnsupportedType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetFallback {
    None,
    PlaceholderTexture,
    DefaultFont,
    HideModel,
    SilenceAudio,
    RejectUi,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetResult {
    pub resource_id: String,
    pub status: AssetResultStatus,
    pub fallback: AssetFallback,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

pub fn evaluate_local_asset(
    entry: &AssetEntry,
    local_sha256: Option<&str>,
    supported_types: &BTreeSet<AssetType>,
) -> AssetResult {
    let (status, fallback, reason) = if !entry.license.redistribution_allowed {
        (
            AssetResultStatus::NotAuthorized,
            fallback_for(entry.asset_type),
            Some("asset redistribution is not authorized".to_owned()),
        )
    } else if !supported_types.contains(&entry.asset_type) {
        (
            AssetResultStatus::UnsupportedType,
            fallback_for(entry.asset_type),
            Some("client does not support this asset type".to_owned()),
        )
    } else {
        match local_sha256 {
            None => (
                AssetResultStatus::Missing,
                fallback_for(entry.asset_type),
                Some("asset is missing".to_owned()),
            ),
            Some(hash) if !hash.eq_ignore_ascii_case(&entry.sha256) => (
                AssetResultStatus::HashMismatch,
                fallback_for(entry.asset_type),
                Some("asset hash does not match manifest".to_owned()),
            ),
            Some(_) => (AssetResultStatus::Ready, AssetFallback::None, None),
        }
    };

    AssetResult {
        resource_id: entry.resource_id.clone(),
        status,
        fallback,
        reason,
    }
}

#[derive(Debug, Error)]
pub enum AssetError {
    #[error("manifest version must be greater than zero")]
    InvalidManifestVersion,
    #[error("manifest contains {actual} assets, maximum is {maximum}")]
    TooManyAssets { actual: usize, maximum: usize },
    #[error("asset request contains {actual} resources, maximum is {maximum}")]
    InvalidRequestCount { actual: usize, maximum: usize },
    #[error("duplicate resource ID: {0}")]
    DuplicateResourceId(String),
    #[error("resource ID is invalid")]
    InvalidResourceId,
    #[error("asset path must be a safe relative path")]
    InvalidAssetPath,
    #[error("SHA-256 must be a 64-character hexadecimal string")]
    InvalidSha256,
    #[error("asset version is invalid")]
    InvalidVersion,
    #[error("{field} is invalid")]
    InvalidText { field: &'static str },
    #[error("asset {0} is not authorized for redistribution")]
    RedistributionNotAllowed(String),
    #[error("asset {resource_id} is {actual} bytes, maximum is {maximum}")]
    InvalidAssetSize {
        resource_id: String,
        actual: u64,
        maximum: u64,
    },
    #[error("manifest total asset size overflowed")]
    TotalSizeOverflow,
    #[error("manifest is {actual} bytes, maximum is {maximum}")]
    ManifestTooLarge { actual: u64, maximum: u64 },
    #[error("manifest hash mismatch: declared {declared}, computed {computed}")]
    ManifestHashMismatch { declared: String, computed: String },
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

fn fallback_for(asset_type: AssetType) -> AssetFallback {
    match asset_type {
        AssetType::Texture => AssetFallback::PlaceholderTexture,
        AssetType::Font => AssetFallback::DefaultFont,
        AssetType::Model => AssetFallback::HideModel,
        AssetType::Sound => AssetFallback::SilenceAudio,
        AssetType::UiLayout => AssetFallback::RejectUi,
    }
}

fn validate_resource_id(value: &str) -> Result<(), AssetError> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && value.contains(':')
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'_' | b'-' | b'.' | b':' | b'/')
        });
    if valid {
        Ok(())
    } else {
        Err(AssetError::InvalidResourceId)
    }
}

fn validate_relative_asset_path(value: &str) -> Result<(), AssetError> {
    let valid_chars = value.bytes().all(|byte| {
        byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || matches!(byte, b'_' | b'-' | b'.' | b'/')
    });
    let valid_segments = value
        .split('/')
        .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
    if !value.is_empty()
        && value.len() <= 256
        && !value.starts_with('/')
        && !value.contains("://")
        && valid_chars
        && valid_segments
    {
        Ok(())
    } else {
        Err(AssetError::InvalidAssetPath)
    }
}

fn validate_sha256(value: &str) -> Result<(), AssetError> {
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(AssetError::InvalidSha256)
    }
}

fn validate_version(value: &str) -> Result<(), AssetError> {
    let valid = !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+' | b'_'));
    if valid {
        Ok(())
    } else {
        Err(AssetError::InvalidVersion)
    }
}

fn validate_text(field: &'static str, value: &str, maximum: usize) -> Result<(), AssetError> {
    if !value.trim().is_empty() && value.len() <= maximum && !value.chars().any(char::is_control) {
        Ok(())
    } else {
        Err(AssetError::InvalidText { field })
    }
}

fn to_lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}
