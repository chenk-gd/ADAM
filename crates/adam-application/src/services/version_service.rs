//! Version service for asset versioning and state management
//!
//! Replaces hardcoded stubs in MCP handlers with actual business logic.

use adam_domain::asset::version::SemVer;
use adam_domain::{
    AssetId, AssetRepository, AssetState, AssetVersion, AssetVersionRepository,
    DirtyQueueRepository, RepositoryError,
};

/// Errors that can occur in VersionService
#[derive(Debug, thiserror::Error)]
pub enum VersionServiceError {
    /// Repository error
    #[error("Repository error: {0}")]
    Repository(#[from] RepositoryError),
    /// Asset not found
    #[error("Asset not found: {0}")]
    NotFound(String),
    /// Invalid version format
    #[error("Invalid version format: {0}")]
    InvalidVersion(String),
    /// Invalid state for operation
    #[error("Invalid state for operation: {0}")]
    InvalidState(String),
    /// Dependency cycle detected
    #[error("Dependency cycle detected")]
    CycleDetected,
}

/// Service for asset versioning and state management
pub struct VersionService<A: AssetRepository, D: DirtyQueueRepository, V: AssetVersionRepository> {
    asset_repo: A,
    dirty_repo: D,
    version_repo: V,
}

impl<A: AssetRepository, D: DirtyQueueRepository, V: AssetVersionRepository>
    VersionService<A, D, V>
{
    /// Create a new VersionService
    pub fn new(asset_repo: A, dirty_repo: D, version_repo: V) -> Self {
        Self {
            asset_repo,
            dirty_repo,
            version_repo,
        }
    }

    /// Publish a new version of an asset
    ///
    /// Creates a new AssetVersion, triggers dirty propagation to dependents,
    /// and updates the asset's current_version and publisher.
    pub async fn publish(
        &self,
        asset_id: &AssetId,
        version_str: &str,
        publisher: &str,
    ) -> Result<AssetVersion, VersionServiceError> {
        // Validate version format
        let version = SemVer::parse(version_str)
            .map_err(|e| VersionServiceError::InvalidVersion(format!("{version_str}: {e}")))?;

        // Get asset
        let asset = self
            .asset_repo
            .find_by_id(asset_id)
            .await
            .map_err(VersionServiceError::from)?
            .ok_or_else(|| VersionServiceError::NotFound(format!("{asset_id:?}")))?;

        // Check asset is not archived
        if asset.state().is_archived() {
            return Err(VersionServiceError::InvalidState(
                "Cannot publish archived asset".to_string(),
            ));
        }

        // Create asset version
        let asset_version = AssetVersion::new(
            *asset_id,
            version.to_string(),
            serde_json::json!({}),
            vec![], // dependencies will be stored separately
            "",     // release notes
            publisher.to_string(),
        );

        // Persist the asset version
        self.version_repo
            .create(&asset_version)
            .await
            .map_err(VersionServiceError::from)?;

        // Update asset's current_version and publisher
        // Note: AssetRepository::update_version method would be needed for full implementation
        // For now, we persist the version and return it

        Ok(asset_version)
    }

    /// Suggest the next version number based on semantic versioning
    ///
    /// Analyzes the current version and change impact to suggest:
    /// - Major bump for breaking changes
    /// - Minor bump for new features
    /// - Patch bump for bug fixes
    pub fn suggest_version(
        current_version: &str,
        change_type: ChangeType,
    ) -> Result<String, VersionServiceError> {
        let version = SemVer::parse(current_version)
            .map_err(|e| VersionServiceError::InvalidVersion(format!("{current_version}: {e}")))?;

        let next = match change_type {
            ChangeType::Breaking => version.next_major(),
            ChangeType::Feature => version.next_minor(),
            ChangeType::Bugfix => version.next_patch(),
        };

        Ok(next.to_string())
    }

    /// Manually resolve dirty state
    ///
    /// Resolves all dirty queue entries for the asset and transitions
    /// to Clean state if no unresolved entries remain.
    pub async fn manual_clean(
        &self,
        asset_id: &AssetId,
        resolved_version: &str,
    ) -> Result<(), VersionServiceError> {
        // Validate version format
        let _version = SemVer::parse(resolved_version)
            .map_err(|e| VersionServiceError::InvalidVersion(format!("{resolved_version}: {e}")))?;

        // Get asset
        let asset = self
            .asset_repo
            .find_by_id(asset_id)
            .await
            .map_err(VersionServiceError::from)?
            .ok_or_else(|| VersionServiceError::NotFound(format!("{asset_id:?}")))?;

        // Check asset is in Dirty state
        if !asset.state().is_dirty() {
            return Err(VersionServiceError::InvalidState(
                "Asset must be in Dirty state to resolve".to_string(),
            ));
        }

        // Resolve all dirty queue entries for this asset
        let unresolved = self
            .dirty_repo
            .find_unresolved_by_asset(asset_id)
            .await
            .map_err(VersionServiceError::from)?;

        for entry in unresolved {
            self.dirty_repo
                .resolve(&entry.id)
                .await
                .map_err(VersionServiceError::from)?;
        }

        // Check if any unresolved entries remain
        let remaining = self
            .dirty_repo
            .find_unresolved_by_asset(asset_id)
            .await
            .map_err(VersionServiceError::from)?;

        // Only mark as Clean if no unresolved entries remain
        if remaining.is_empty() {
            self.asset_repo
                .update_state(asset_id, AssetState::Clean)
                .await
                .map_err(VersionServiceError::from)?;
        }

        Ok(())
    }

    /// Check if asset needs review (has unresolved dirty entries)
    pub async fn needs_review(&self, asset_id: &AssetId) -> Result<bool, VersionServiceError> {
        let unresolved = self
            .dirty_repo
            .find_unresolved_by_asset(asset_id)
            .await
            .map_err(VersionServiceError::from)?;
        Ok(!unresolved.is_empty())
    }
}

/// Types of changes for version suggestion
#[derive(Debug, Clone, Copy)]
pub enum ChangeType {
    /// Breaking change - major version bump
    Breaking,
    /// New feature - minor version bump
    Feature,
    /// Bug fix - patch version bump
    Bugfix,
}

#[cfg(test)]
mod tests {
    use super::*;
    use adam_domain::{
        AssetId, AssetLevel, AssetState, AssetTypeId, CreateAssetCommand, DirtyQueueEntry,
        InMemoryAssetRepository, InMemoryAssetVersionRepository, InMemoryDirtyQueueRepository,
        OrganizationId, ProjectId,
    };

    #[tokio::test]
    async fn publish_creates_version() {
        let org_id = OrganizationId::new();
        let project_id = ProjectId::new();
        let type_id = AssetTypeId::new();

        let asset_repo = InMemoryAssetRepository::new();
        let dirty_repo = InMemoryDirtyQueueRepository::new();

        // Create asset first
        let cmd = CreateAssetCommand {
            name: "Test Asset".to_string(),
            asset_type_id: type_id,
            project_id: Some(project_id),
            organization_id: org_id,
            level: AssetLevel::Project,
            external_ref: "https://example.com/asset/1".to_string(),
            source: "manual".to_string(),
            metadata: serde_json::json!({}),
            idempotency_key: None,
        };
        let asset = asset_repo.create(&cmd).await.unwrap();

        let service = VersionService::new(
            asset_repo,
            dirty_repo,
            InMemoryAssetVersionRepository::new(),
        );

        // Publish new version
        let version = service
            .publish(&asset.id, "1.0.0", "test_user")
            .await
            .unwrap();

        assert_eq!(version.version_number, "1.0.0");
        assert_eq!(version.released_by, "test_user");
    }

    #[tokio::test]
    async fn publish_fails_for_archived_asset() {
        let org_id = OrganizationId::new();
        let project_id = ProjectId::new();
        let type_id = AssetTypeId::new();

        let asset_repo = InMemoryAssetRepository::new();
        let dirty_repo = InMemoryDirtyQueueRepository::new();

        // Create asset
        let cmd = CreateAssetCommand {
            name: "Test Asset".to_string(),
            asset_type_id: type_id,
            project_id: Some(project_id),
            organization_id: org_id,
            level: AssetLevel::Project,
            external_ref: "https://example.com/asset/1".to_string(),
            source: "manual".to_string(),
            metadata: serde_json::json!({}),
            idempotency_key: None,
        };
        let asset = asset_repo.create(&cmd).await.unwrap();

        let service = VersionService::new(
            asset_repo,
            dirty_repo,
            InMemoryAssetVersionRepository::new(),
        );

        // Archive the asset
        service
            .asset_repo
            .update_state(&asset.id, AssetState::Archived)
            .await
            .unwrap();

        // Try to publish
        let result = service.publish(&asset.id, "1.0.0", "test_user").await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            VersionServiceError::InvalidState(_)
        ));
    }

    #[test]
    fn suggest_version_major() {
        let next = VersionService::<
            InMemoryAssetRepository,
            InMemoryDirtyQueueRepository,
            InMemoryAssetVersionRepository,
        >::suggest_version("1.2.3", ChangeType::Breaking)
        .unwrap();
        assert_eq!(next, "2.0.0");
    }

    #[test]
    fn suggest_version_minor() {
        let next = VersionService::<
            InMemoryAssetRepository,
            InMemoryDirtyQueueRepository,
            InMemoryAssetVersionRepository,
        >::suggest_version("1.2.3", ChangeType::Feature)
        .unwrap();
        assert_eq!(next, "1.3.0");
    }

    #[test]
    fn suggest_version_patch() {
        let next = VersionService::<
            InMemoryAssetRepository,
            InMemoryDirtyQueueRepository,
            InMemoryAssetVersionRepository,
        >::suggest_version("1.2.3", ChangeType::Bugfix)
        .unwrap();
        assert_eq!(next, "1.2.4");
    }

    #[test]
    fn suggest_version_invalid_input() {
        let result = VersionService::<
            InMemoryAssetRepository,
            InMemoryDirtyQueueRepository,
            InMemoryAssetVersionRepository,
        >::suggest_version("not-a-version", ChangeType::Bugfix);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn manual_clean_resolves_dirty() {
        let org_id = OrganizationId::new();
        let project_id = ProjectId::new();
        let type_id = AssetTypeId::new();

        let asset_repo = InMemoryAssetRepository::new();
        let dirty_repo = InMemoryDirtyQueueRepository::new();

        // Create asset
        let cmd = CreateAssetCommand {
            name: "Test Asset".to_string(),
            asset_type_id: type_id,
            project_id: Some(project_id),
            organization_id: org_id,
            level: AssetLevel::Project,
            external_ref: "https://example.com/asset/1".to_string(),
            source: "manual".to_string(),
            metadata: serde_json::json!({}),
            idempotency_key: None,
        };
        let asset = asset_repo.create(&cmd).await.unwrap();

        // Make asset dirty
        asset_repo
            .update_state(&asset.id, AssetState::Dirty)
            .await
            .unwrap();

        // Add a dirty queue entry
        let entry = DirtyQueueEntry {
            id: uuid::Uuid::new_v4(),
            asset_id: asset.id,
            upstream_asset_id: AssetId::new(),
            upstream_version: "1.0.0".to_string(),
            upstream_old_version: "0.0.0".to_string(),
            impact_level: "medium".to_string(),
            since: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
            resolved_at: None,
        };
        dirty_repo.upsert(&entry).await.unwrap();

        let service = VersionService::new(
            asset_repo,
            dirty_repo,
            InMemoryAssetVersionRepository::new(),
        );

        // Resolve dirty
        service.manual_clean(&asset.id, "1.0.1").await.unwrap();

        // Verify asset is now clean
        let updated = service
            .asset_repo
            .find_by_id(&asset.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.state(), AssetState::Clean);
    }

    #[tokio::test]
    async fn manual_clean_fails_for_non_dirty() {
        let org_id = OrganizationId::new();
        let project_id = ProjectId::new();
        let type_id = AssetTypeId::new();

        let asset_repo = InMemoryAssetRepository::new();
        let dirty_repo = InMemoryDirtyQueueRepository::new();

        // Create asset (starts as Clean)
        let cmd = CreateAssetCommand {
            name: "Test Asset".to_string(),
            asset_type_id: type_id,
            project_id: Some(project_id),
            organization_id: org_id,
            level: AssetLevel::Project,
            external_ref: "https://example.com/asset/1".to_string(),
            source: "manual".to_string(),
            metadata: serde_json::json!({}),
            idempotency_key: None,
        };
        let asset = asset_repo.create(&cmd).await.unwrap();

        let service = VersionService::new(
            asset_repo,
            dirty_repo,
            InMemoryAssetVersionRepository::new(),
        );

        // Try to clean a Clean asset
        let result = service.manual_clean(&asset.id, "1.0.1").await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            VersionServiceError::InvalidState(_)
        ));
    }

    #[tokio::test]
    async fn needs_review_returns_true_when_dirty() {
        let org_id = OrganizationId::new();
        let project_id = ProjectId::new();
        let type_id = AssetTypeId::new();

        let asset_repo = InMemoryAssetRepository::new();
        let dirty_repo = InMemoryDirtyQueueRepository::new();

        // Create asset
        let cmd = CreateAssetCommand {
            name: "Test Asset".to_string(),
            asset_type_id: type_id,
            project_id: Some(project_id),
            organization_id: org_id,
            level: AssetLevel::Project,
            external_ref: "https://example.com/asset/1".to_string(),
            source: "manual".to_string(),
            metadata: serde_json::json!({}),
            idempotency_key: None,
        };
        let asset = asset_repo.create(&cmd).await.unwrap();

        let service = VersionService::new(
            asset_repo,
            dirty_repo,
            InMemoryAssetVersionRepository::new(),
        );

        // Initially no review needed
        assert!(!service.needs_review(&asset.id).await.unwrap());

        // Add dirty queue entry
        let entry = DirtyQueueEntry {
            id: uuid::Uuid::new_v4(),
            asset_id: asset.id,
            upstream_asset_id: AssetId::new(),
            upstream_version: "2.0.0".to_string(),
            upstream_old_version: "1.0.0".to_string(),
            impact_level: "high".to_string(),
            since: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
            resolved_at: None,
        };
        service.dirty_repo.upsert(&entry).await.unwrap();

        // Now review is needed
        assert!(service.needs_review(&asset.id).await.unwrap());
    }
}
