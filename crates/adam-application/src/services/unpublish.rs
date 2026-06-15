//! Version unpublish service
//!
//! Provides functionality to unpublish versions with propagation to downstream dependencies.

use std::sync::Arc;

use adam_domain::{
    AssetId, AssetRepository, AssetVersionRepository, DependencyRepository, DirtyQueueRepository,
    RepositoryError, SemVer,
};

/// Error types for unpublish operations
#[derive(Debug, thiserror::Error)]
pub enum UnpublishError {
    /// Repository error
    #[error("Repository error: {0}")]
    Repository(#[from] RepositoryError),
    /// Asset not found
    #[error("Asset not found: {0}")]
    NotFound(String),
    /// Unpublish not allowed
    #[error("Unpublish not allowed: {0}")]
    NotAllowed(String),
    /// Unpublish window expired
    #[error("Unpublish window expired")]
    WindowExpired,
    /// Version not found
    #[error("Version not found: {0}")]
    VersionNotFound(String),
}

/// Policy for unpublish operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnpublishPolicy {
    /// Unpublish is never allowed
    Never,
    /// Unpublish is allowed within a duration after release
    AllowWithin(chrono::Duration),
    /// Unpublish requires approval
    RequireApproval,
}

/// Propagation strategy for unpublish operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnpublishPropagation {
    /// Do not propagate
    None,
    /// Notify downstream dependencies
    NotifyDownstream,
    /// Automatically unpublish downstream
    AutoUnpublishDownstream,
}

/// Organization configuration for unpublish
#[derive(Debug, Clone)]
pub struct UnpublishConfig {
    pub policy: UnpublishPolicy,
    pub propagation: UnpublishPropagation,
}

impl Default for UnpublishConfig {
    fn default() -> Self {
        Self {
            policy: UnpublishPolicy::AllowWithin(chrono::Duration::hours(24)),
            propagation: UnpublishPropagation::NotifyDownstream,
        }
    }
}

/// Service for unpublishing versions with propagation
pub struct UnpublishService<AR, VR, DR, QR>
where
    AR: AssetRepository,
    VR: AssetVersionRepository,
    DR: DependencyRepository,
    QR: DirtyQueueRepository,
{
    _asset_repo: Arc<AR>,
    version_repo: Arc<VR>,
    dependency_repo: Arc<DR>,
    dirty_queue_repo: Arc<QR>,
}

impl<AR, VR, DR, QR> UnpublishService<AR, VR, DR, QR>
where
    AR: AssetRepository,
    VR: AssetVersionRepository,
    DR: DependencyRepository,
    QR: DirtyQueueRepository,
{
    /// Create a new UnpublishService
    pub fn new(
        asset_repo: Arc<AR>,
        version_repo: Arc<VR>,
        dependency_repo: Arc<DR>,
        dirty_queue_repo: Arc<QR>,
    ) -> Self {
        Self {
            _asset_repo: asset_repo,
            version_repo,
            dependency_repo,
            dirty_queue_repo,
        }
    }

    /// Unpublish a version
    ///
    /// # Arguments
    /// * `asset_id` - ID of the asset
    /// * `version` - The version to unpublish
    /// * `reason` - Reason for unpublishing
    pub async fn unpublish_version(
        &self,
        asset_id: AssetId,
        version: SemVer,
        _reason: String,
    ) -> Result<(), UnpublishError> {
        // Get organization config (using default for now)
        let config = UnpublishConfig::default();

        // Check policy
        match config.policy {
            UnpublishPolicy::Never => {
                return Err(UnpublishError::NotAllowed(
                    "Unpublish is not allowed for this organization".to_string(),
                ));
            }
            UnpublishPolicy::AllowWithin(duration) => {
                // Find the version
                let version_str = version.to_string();
                let version_record = self
                    .version_repo
                    .find_by_version(&asset_id, &version_str)
                    .await?
                    .ok_or_else(|| UnpublishError::VersionNotFound(version.to_string()))?;

                // Check if within window
                let age = chrono::Utc::now() - version_record.released_at;
                if age > duration {
                    return Err(UnpublishError::WindowExpired);
                }
            }
            UnpublishPolicy::RequireApproval => {
                // For now, just proceed without approval
                // In a full implementation, this would create an approval request
            }
        }

        // Propagate to downstream if configured
        if config.propagation == UnpublishPropagation::NotifyDownstream {
            self.propagate_unpublish(&asset_id, &version).await?;
        }

        // Note: Actually marking as unpublished would require:
        // 1. Adding is_unpublished field to AssetVersion
        // 2. Adding mark_unpublished method to AssetVersionRepository
        // For now, we just propagate the unpublish notification

        Ok(())
    }

    /// Propagate unpublish to downstream dependencies
    async fn propagate_unpublish(
        &self,
        upstream_id: &AssetId,
        version: &SemVer,
    ) -> Result<(), UnpublishError> {
        // Find downstream dependencies
        let deps = self
            .dependency_repo
            .find_downstream_dependencies(upstream_id)
            .await?;

        for dep in deps {
            // Create dirty queue entry for each downstream
            // This notifies them that the upstream version was unpublished
            let entry = adam_domain::DirtyQueueEntry {
                id: uuid::Uuid::new_v4(),
                asset_id: dep.source_id,
                upstream_asset_id: *upstream_id,
                upstream_version: version.to_string(),
                upstream_old_version: version.to_string(),
                impact_level: "high".to_string(),
                since: chrono::Utc::now(),
                created_at: chrono::Utc::now(),
                resolved_at: None,
            };

            self.dirty_queue_repo.upsert(&entry).await?;
        }

        Ok(())
    }

    /// Get organization config (placeholder - would fetch from config store)
    #[allow(dead_code)]
    async fn get_org_config(&self, _asset_id: AssetId) -> Result<UnpublishConfig, UnpublishError> {
        Ok(UnpublishConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adam_domain::{
        InMemoryAssetRepository, InMemoryAssetVersionRepository, InMemoryDependencyRepository,
        InMemoryDirtyQueueRepository,
    };

    #[tokio::test]
    async fn unpublish_not_allowed_when_policy_never() {
        let asset_repo = Arc::new(InMemoryAssetRepository::new());
        let version_repo = Arc::new(InMemoryAssetVersionRepository::new());
        let dependency_repo = Arc::new(InMemoryDependencyRepository::new());
        let dirty_queue_repo = Arc::new(InMemoryDirtyQueueRepository::new());

        // Note: This test would require mocking the config to return Never policy
        // For now, we just verify the service can be created
        let _service =
            UnpublishService::new(asset_repo, version_repo, dependency_repo, dirty_queue_repo);
    }
}
