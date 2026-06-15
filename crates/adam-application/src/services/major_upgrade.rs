//! Major upgrade rollback service
//!
//! Provides rollback functionality for major version upgrades using snapshots.

use std::sync::Arc;

use adam_domain::{
    AssetId, AssetRepository, AssetVersionRepository, DependencyRepository, RepositoryError, SemVer,
};

/// Error types for major upgrade operations
#[derive(Debug, thiserror::Error)]
pub enum MajorUpgradeError {
    /// Repository error
    #[error("Repository error: {0}")]
    Repository(#[from] RepositoryError),
    /// Asset not found
    #[error("Asset not found: {0}")]
    NotFound(String),
    /// Rollback not possible
    #[error("Rollback not possible: {0}")]
    RollbackNotPossible(String),
    /// Snapshot not found
    #[error("Snapshot not found: {0}")]
    SnapshotNotFound(String),
}

/// Snapshot of dependencies before upgrade
#[derive(Debug, Clone)]
pub struct DependencySnapshot {
    pub upstream_id: AssetId,
    pub effective_version: SemVer,
}

/// Major upgrade operation record
#[derive(Debug, Clone)]
pub struct MajorUpgradeOperation {
    pub id: uuid::Uuid,
    pub asset_id: AssetId,
    pub from_version: SemVer,
    pub to_version: SemVer,
    pub snapshots: Vec<DependencySnapshot>,
    pub status: UpgradeStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Status of upgrade operation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpgradeStatus {
    InProgress,
    Completed,
    RolledBack,
    Failed,
}

/// Repository for major upgrade operations
#[async_trait::async_trait]
pub trait MajorUpgradeRepository: Send + Sync {
    /// Create a new upgrade operation
    async fn create(&self, operation: &MajorUpgradeOperation) -> Result<(), RepositoryError>;

    /// Find operation by ID
    async fn find_by_id(
        &self,
        id: uuid::Uuid,
    ) -> Result<Option<MajorUpgradeOperation>, RepositoryError>;

    /// Update operation status
    async fn update_status(
        &self,
        id: uuid::Uuid,
        status: UpgradeStatus,
    ) -> Result<(), RepositoryError>;
}

/// Service for major upgrades with rollback support
pub struct MajorUpgradeService<AR, VR, DR, UR>
where
    AR: AssetRepository,
    VR: AssetVersionRepository,
    DR: DependencyRepository,
    UR: MajorUpgradeRepository,
{
    _asset_repo: Arc<AR>,
    _version_repo: Arc<VR>,
    dependency_repo: Arc<DR>,
    upgrade_repo: Arc<UR>,
}

impl<AR, VR, DR, UR> MajorUpgradeService<AR, VR, DR, UR>
where
    AR: AssetRepository,
    VR: AssetVersionRepository,
    DR: DependencyRepository,
    UR: MajorUpgradeRepository,
{
    /// Create a new MajorUpgradeService
    pub fn new(
        asset_repo: Arc<AR>,
        version_repo: Arc<VR>,
        dependency_repo: Arc<DR>,
        upgrade_repo: Arc<UR>,
    ) -> Self {
        Self {
            _asset_repo: asset_repo,
            _version_repo: version_repo,
            dependency_repo,
            upgrade_repo,
        }
    }

    /// Start a major upgrade with snapshot
    ///
    /// # Arguments
    /// * `asset_id` - ID of the asset to upgrade
    /// * `from_version` - Current version
    /// * `to_version` - Target version
    pub async fn start_upgrade(
        &self,
        asset_id: AssetId,
        from_version: SemVer,
        to_version: SemVer,
    ) -> Result<MajorUpgradeOperation, MajorUpgradeError> {
        // Get upstream dependencies
        let deps = self
            .dependency_repo
            .find_upstream_dependencies(&asset_id)
            .await?;

        // Create snapshots
        let snapshots: Vec<DependencySnapshot> = deps
            .into_iter()
            .map(|dep| DependencySnapshot {
                upstream_id: dep.target_id,
                effective_version: dep.effective_version.clone(),
            })
            .collect();

        // Create operation record
        let operation = MajorUpgradeOperation {
            id: uuid::Uuid::new_v4(),
            asset_id,
            from_version,
            to_version,
            snapshots,
            status: UpgradeStatus::InProgress,
            created_at: chrono::Utc::now(),
            completed_at: None,
        };

        self.upgrade_repo.create(&operation).await?;

        Ok(operation)
    }

    /// Complete an upgrade
    pub async fn complete_upgrade(
        &self,
        operation_id: uuid::Uuid,
    ) -> Result<(), MajorUpgradeError> {
        self.upgrade_repo
            .update_status(operation_id, UpgradeStatus::Completed)
            .await?;
        Ok(())
    }

    /// Rollback an upgrade
    ///
    /// Restores dependencies to their pre-upgrade state
    pub async fn rollback_upgrade(
        &self,
        operation_id: uuid::Uuid,
    ) -> Result<(), MajorUpgradeError> {
        // Find the operation
        let operation = self
            .upgrade_repo
            .find_by_id(operation_id)
            .await?
            .ok_or_else(|| MajorUpgradeError::NotFound(format!("Operation {operation_id}")))?;

        // Verify operation can be rolled back
        if operation.status != UpgradeStatus::Completed {
            return Err(MajorUpgradeError::RollbackNotPossible(
                "Only completed upgrades can be rolled back".to_string(),
            ));
        }

        // Restore dependencies from snapshots
        for snapshot in &operation.snapshots {
            // Update effective version back to pre-upgrade value
            self.dependency_repo
                .update_effective_version(
                    &operation.asset_id,
                    &snapshot.upstream_id,
                    snapshot.effective_version.to_string(),
                    "rollback".to_string(),
                    adam_domain::EffectiveUpdateReason::ManualClean,
                )
                .await?;
        }

        // Update operation status
        self.upgrade_repo
            .update_status(operation_id, UpgradeStatus::RolledBack)
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upgrade_status_equality() {
        assert_eq!(UpgradeStatus::InProgress, UpgradeStatus::InProgress);
        assert_ne!(UpgradeStatus::InProgress, UpgradeStatus::Completed);
    }
}
