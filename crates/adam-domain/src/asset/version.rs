//! Asset version and publish history

use crate::asset::instance::AssetId;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

/// Unique identifier for asset versions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AssetVersionId(pub Uuid);

impl AssetVersionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AssetVersionId {
    fn default() -> Self {
        Self::new()
    }
}

/// Semantic version type
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SemVer {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl SemVer {
    pub fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    pub fn parse(version: &str) -> Result<Self, String> {
        let version = version.trim_start_matches('v');
        let parts: Vec<&str> = version.split('.').collect();
        if parts.len() != 3 {
            return Err("Invalid semver format".to_string());
        }
        Ok(Self {
            major: parts[0].parse::<u64>().map_err(|e| e.to_string())?,
            minor: parts[1].parse::<u64>().map_err(|e| e.to_string())?,
            patch: parts[2].parse::<u64>().map_err(|e| e.to_string())?,
        })
    }

    /// Get next major version (resets minor and patch to 0)
    pub fn next_major(&self) -> Self {
        Self {
            major: self.major + 1,
            minor: 0,
            patch: 0,
        }
    }

    /// Get next minor version (resets patch to 0)
    pub fn next_minor(&self) -> Self {
        Self {
            major: self.major,
            minor: self.minor + 1,
            patch: 0,
        }
    }

    /// Get next patch version
    pub fn next_patch(&self) -> Self {
        Self {
            major: self.major,
            minor: self.minor,
            patch: self.patch + 1,
        }
    }
}

impl std::fmt::Display for SemVer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Use normalized format (without "v" prefix) for consistent storage
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Dependency snapshot at publish time
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencySnapshot {
    pub upstream_asset_id: AssetId,
    pub upstream_version: String,
}

/// Asset version - publish history
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetVersion {
    pub id: AssetVersionId,
    pub asset_id: AssetId,
    pub version_number: String,
    pub metadata: serde_json::Value,
    pub dependencies: Vec<DependencySnapshot>, // FR-012: 发布依赖快照
    pub release_notes: String,
    pub suggested_type: Option<String>, // major/minor/patch
    pub released_by: String,
    pub released_at: chrono::DateTime<chrono::Utc>,
}

impl AssetVersion {
    pub fn new(
        asset_id: AssetId,
        version_number: impl Into<String>,
        metadata: serde_json::Value,
        dependencies: Vec<DependencySnapshot>,
        release_notes: impl Into<String>,
        released_by: impl Into<String>,
    ) -> Self {
        Self {
            id: AssetVersionId::new(),
            asset_id,
            version_number: version_number.into(),
            metadata,
            dependencies,
            release_notes: release_notes.into(),
            suggested_type: None,
            released_by: released_by.into(),
            released_at: chrono::Utc::now(),
        }
    }
}

/// Repository trait for asset versions
#[async_trait::async_trait]
pub trait AssetVersionRepository: Send + Sync {
    /// Create a new version
    async fn create(&self, version: &AssetVersion) -> Result<(), crate::RepositoryError>;

    /// Find versions by asset ID
    async fn find_by_asset(
        &self,
        asset_id: &AssetId,
    ) -> Result<Vec<AssetVersion>, crate::RepositoryError>;

    /// Find specific version by asset + version number
    async fn find_by_version(
        &self,
        asset_id: &AssetId,
        version: &str,
    ) -> Result<Option<AssetVersion>, crate::RepositoryError>;

    /// Get latest version for an asset
    async fn find_latest(
        &self,
        asset_id: &AssetId,
    ) -> Result<Option<AssetVersion>, crate::RepositoryError>;
}

#[async_trait::async_trait]
impl<T: AssetVersionRepository + ?Sized> AssetVersionRepository for Arc<T> {
    async fn create(&self, version: &AssetVersion) -> Result<(), crate::RepositoryError> {
        self.as_ref().create(version).await
    }

    async fn find_by_asset(
        &self,
        asset_id: &AssetId,
    ) -> Result<Vec<AssetVersion>, crate::RepositoryError> {
        self.as_ref().find_by_asset(asset_id).await
    }

    async fn find_by_version(
        &self,
        asset_id: &AssetId,
        version: &str,
    ) -> Result<Option<AssetVersion>, crate::RepositoryError> {
        self.as_ref().find_by_version(asset_id, version).await
    }

    async fn find_latest(
        &self,
        asset_id: &AssetId,
    ) -> Result<Option<AssetVersion>, crate::RepositoryError> {
        self.as_ref().find_latest(asset_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_semver_parse_and_display() {
        let semver = SemVer::new(1, 2, 3);
        assert_eq!(semver.to_string(), "1.2.3");

        let parsed = SemVer::parse("v1.2.3").unwrap();
        assert_eq!(parsed.major, 1);
        assert_eq!(parsed.minor, 2);
        assert_eq!(parsed.patch, 3);
    }

    #[test]
    fn test_asset_version_creation() {
        let asset_id = AssetId::new();
        let deps = vec![DependencySnapshot {
            upstream_asset_id: AssetId::new(),
            upstream_version: "v1.0.0".to_string(),
        }];

        let version = AssetVersion::new(
            asset_id,
            "v1.0.0",
            json!({"key": "value"}),
            deps,
            "Initial release",
            "user@example.com",
        );

        assert_eq!(version.version_number, "v1.0.0");
        assert_eq!(version.dependencies.len(), 1);
        assert_eq!(version.release_notes, "Initial release");
    }
}
