//! Version service for asset versioning and state management
//!
//! Replaces hardcoded stubs in MCP handlers with actual business logic.

use adam_domain::asset::DependencySnapshot;
use adam_domain::asset::version::SemVer;
use adam_domain::{
    AssetDependencyRecord, AssetId, AssetRepository, AssetState, AssetVersion,
    AssetVersionRepository, DependencyRepository, DirtyQueueRepository, DirtyResolutionLog,
    DirtyResolutionLogRepository, EffectiveUpdateReason, RepositoryError,
};

use crate::services::state_propagator::StatePropagator;

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

/// Dependency declared during publish
#[derive(Debug, Clone)]
pub struct PublishDependency {
    pub upstream_asset_id: AssetId,
    pub version: String,
}

/// Command for publishing an asset version
#[derive(Debug, Clone)]
pub struct PublishAssetCommand {
    pub asset_id: AssetId,
    pub version: String,
    pub publisher: String,
    pub release_notes: String,
    pub dependencies: Vec<PublishDependency>,
    pub suggested_type: Option<String>,
}

/// One upstream dependency review outcome during manual clean
#[derive(Debug, Clone)]
pub struct ManualCleanResolution {
    pub upstream_asset_id: AssetId,
    pub from_version: String,
    pub to_version: String,
    pub review_result: String,
    pub comment: Option<String>,
}

/// Command for manually resolving dirty state
#[derive(Debug, Clone)]
pub struct ManualCleanCommand {
    pub asset_id: AssetId,
    pub asset_version: String,
    pub reviewed_by: String,
    pub resolutions: Vec<ManualCleanResolution>,
}

/// Service for asset versioning and state management
pub struct VersionService<
    A: AssetRepository,
    D: DirtyQueueRepository,
    V: AssetVersionRepository,
    DEP: DependencyRepository,
    L: DirtyResolutionLogRepository,
> {
    asset_repo: A,
    dirty_repo: D,
    version_repo: V,
    dependency_repo: DEP,
    dirty_log_repo: L,
}

impl<
    A: AssetRepository,
    D: DirtyQueueRepository,
    V: AssetVersionRepository,
    DEP: DependencyRepository,
    L: DirtyResolutionLogRepository,
> VersionService<A, D, V, DEP, L>
{
    /// Create a new VersionService
    pub fn new(
        asset_repo: A,
        dirty_repo: D,
        version_repo: V,
        dependency_repo: DEP,
        dirty_log_repo: L,
    ) -> Self {
        Self {
            asset_repo,
            dirty_repo,
            version_repo,
            dependency_repo,
            dirty_log_repo,
        }
    }

    /// Publish a new version of an asset
    ///
    /// Creates a new AssetVersion, triggers dirty propagation to dependents,
    /// and updates the asset's current_version and publisher.
    pub async fn publish(
        &self,
        cmd: PublishAssetCommand,
    ) -> Result<AssetVersion, VersionServiceError> {
        // Validate version format
        let version = SemVer::parse(&cmd.version)
            .map_err(|e| VersionServiceError::InvalidVersion(format!("{}: {e}", cmd.version)))?;

        // Get asset
        let asset = self
            .asset_repo
            .find_by_id(&cmd.asset_id)
            .await
            .map_err(VersionServiceError::from)?
            .ok_or_else(|| VersionServiceError::NotFound(format!("{:?}", cmd.asset_id)))?;

        // Check asset is not archived
        if asset.state().is_archived() {
            return Err(VersionServiceError::InvalidState(
                "Cannot publish archived asset".to_string(),
            ));
        }

        let dependency_snapshots: Vec<DependencySnapshot> = cmd
            .dependencies
            .iter()
            .map(|dependency| DependencySnapshot {
                upstream_asset_id: dependency.upstream_asset_id,
                upstream_version: dependency.version.clone(),
            })
            .collect();

        let mut asset_version = AssetVersion::new(
            cmd.asset_id,
            version.to_string(),
            asset.metadata.clone(),
            dependency_snapshots,
            cmd.release_notes.clone(),
            cmd.publisher.clone(),
        );
        asset_version.suggested_type = cmd.suggested_type.clone();

        // Persist the asset version
        self.version_repo
            .create(&asset_version)
            .await
            .map_err(VersionServiceError::from)?;

        for dependency in &cmd.dependencies {
            let now = chrono::Utc::now();
            self.dependency_repo
                .create_dependency_record(&AssetDependencyRecord {
                    id: uuid::Uuid::new_v4(),
                    source_id: cmd.asset_id,
                    target_id: dependency.upstream_asset_id,
                    relationship: "depends_on".to_string(),
                    declared_version: dependency.version.clone(),
                    effective_version: dependency.version.clone(),
                    effective_updated_by: cmd.publisher.clone(),
                    effective_updated_at: now,
                    effective_reason: EffectiveUpdateReason::Publish,
                    created_at: now,
                })
                .await
                .map_err(VersionServiceError::from)?;
        }

        self.asset_repo
            .update_publication(
                &cmd.asset_id,
                asset_version.version_number.clone(),
                cmd.publisher.clone(),
                AssetState::Clean,
            )
            .await
            .map_err(VersionServiceError::from)?;

        let propagator = StatePropagator::new();
        propagator
            .on_asset_published(
                &cmd.asset_id,
                &asset_version.version_number,
                &self.asset_repo,
                &self.dependency_repo,
                &self.dirty_repo,
            )
            .await
            .map_err(|e| {
                VersionServiceError::Repository(RepositoryError::DatabaseError(e.to_string()))
            })?;

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
    pub async fn manual_clean(&self, cmd: ManualCleanCommand) -> Result<(), VersionServiceError> {
        SemVer::parse(&cmd.asset_version).map_err(|e| {
            VersionServiceError::InvalidVersion(format!("{}: {e}", cmd.asset_version))
        })?;

        // Get asset
        let asset = self
            .asset_repo
            .find_by_id(&cmd.asset_id)
            .await
            .map_err(VersionServiceError::from)?
            .ok_or_else(|| VersionServiceError::NotFound(format!("{:?}", cmd.asset_id)))?;

        // Check asset is in Dirty state
        if !asset.state().is_dirty() {
            return Err(VersionServiceError::InvalidState(
                "Asset must be in Dirty state to resolve".to_string(),
            ));
        }

        let unresolved = self
            .dirty_repo
            .find_unresolved_by_asset(&cmd.asset_id)
            .await
            .map_err(VersionServiceError::from)?;

        for resolution in &cmd.resolutions {
            let entry = unresolved
                .iter()
                .find(|entry| {
                    entry.upstream_asset_id == resolution.upstream_asset_id
                        && entry.upstream_old_version == resolution.from_version
                        && entry.upstream_version == resolution.to_version
                })
                .ok_or_else(|| {
                    VersionServiceError::InvalidState(format!(
                        "No unresolved dirty entry for upstream {:?} from {} to {}",
                        resolution.upstream_asset_id,
                        resolution.from_version,
                        resolution.to_version
                    ))
                })?;

            self.dependency_repo
                .update_effective_version(
                    &cmd.asset_id,
                    &resolution.upstream_asset_id,
                    resolution.to_version.clone(),
                    cmd.reviewed_by.clone(),
                    EffectiveUpdateReason::ManualClean,
                )
                .await
                .map_err(VersionServiceError::from)?;

            let reviewed_at = chrono::Utc::now();
            self.dirty_log_repo
                .insert(&DirtyResolutionLog {
                    id: uuid::Uuid::new_v4(),
                    asset_id: cmd.asset_id,
                    asset_version: cmd.asset_version.clone(),
                    upstream_asset_id: resolution.upstream_asset_id,
                    from_version: resolution.from_version.clone(),
                    to_version: resolution.to_version.clone(),
                    action: "manual_clean".to_string(),
                    review_result: resolution.review_result.clone(),
                    comment: resolution.comment.clone(),
                    reviewed_by: cmd.reviewed_by.clone(),
                    reviewed_at,
                })
                .await
                .map_err(VersionServiceError::from)?;

            self.dirty_repo
                .resolve(&entry.id)
                .await
                .map_err(VersionServiceError::from)?;
        }

        // Check if any unresolved entries remain
        let remaining = self
            .dirty_repo
            .find_unresolved_by_asset(&cmd.asset_id)
            .await
            .map_err(VersionServiceError::from)?;

        // Only mark as Clean if no unresolved entries remain
        if remaining.is_empty() {
            self.asset_repo
                .update_state(&cmd.asset_id, AssetState::Clean)
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
        InMemoryAssetRepository, InMemoryAssetVersionRepository, InMemoryDependencyRepository,
        InMemoryDirtyQueueRepository, InMemoryDirtyResolutionLogRepository, OrganizationId,
        ProjectId,
    };

    type TestVersionService = VersionService<
        InMemoryAssetRepository,
        InMemoryDirtyQueueRepository,
        InMemoryAssetVersionRepository,
        InMemoryDependencyRepository,
        InMemoryDirtyResolutionLogRepository,
    >;

    fn publish_command(asset_id: AssetId, version: &str, publisher: &str) -> PublishAssetCommand {
        PublishAssetCommand {
            asset_id,
            version: version.to_string(),
            publisher: publisher.to_string(),
            release_notes: String::new(),
            dependencies: Vec::new(),
            suggested_type: None,
        }
    }

    fn manual_clean_command(
        asset_id: AssetId,
        asset_version: &str,
        reviewed_by: &str,
        resolutions: Vec<ManualCleanResolution>,
    ) -> ManualCleanCommand {
        ManualCleanCommand {
            asset_id,
            asset_version: asset_version.to_string(),
            reviewed_by: reviewed_by.to_string(),
            resolutions,
        }
    }

    fn create_asset_command(
        name: &str,
        org_id: OrganizationId,
        project_id: ProjectId,
        type_id: AssetTypeId,
        metadata: serde_json::Value,
    ) -> CreateAssetCommand {
        CreateAssetCommand {
            name: name.to_string(),
            asset_type_id: type_id,
            project_id: Some(project_id),
            organization_id: org_id,
            level: AssetLevel::Project,
            external_ref: format!("https://example.com/asset/{name}"),
            source: "manual".to_string(),
            metadata,
            idempotency_key: None,
        }
    }

    #[tokio::test]
    async fn publish_creates_version_and_updates_asset() {
        let org_id = OrganizationId::new();
        let project_id = ProjectId::new();
        let type_id = AssetTypeId::new();

        let asset_repo = InMemoryAssetRepository::new();
        let dirty_repo = InMemoryDirtyQueueRepository::new();

        let cmd = create_asset_command(
            "Test Asset",
            org_id,
            project_id,
            type_id,
            serde_json::json!({"domain": "requirements"}),
        );
        let asset = asset_repo.create(&cmd).await.unwrap();

        let service = VersionService::new(
            asset_repo,
            dirty_repo,
            InMemoryAssetVersionRepository::new(),
            InMemoryDependencyRepository::new(),
            InMemoryDirtyResolutionLogRepository::new(),
        );

        let version = service
            .publish(PublishAssetCommand {
                asset_id: asset.id,
                version: "v1.0.0".to_string(),
                publisher: "test_user".to_string(),
                release_notes: "initial release".to_string(),
                dependencies: Vec::new(),
                suggested_type: Some("minor".to_string()),
            })
            .await
            .unwrap();

        assert_eq!(version.version_number, "1.0.0");
        assert_eq!(version.released_by, "test_user");
        assert_eq!(version.release_notes, "initial release");
        assert_eq!(version.suggested_type, Some("minor".to_string()));
        assert_eq!(
            version.metadata,
            serde_json::json!({"domain": "requirements"})
        );

        let updated = service
            .asset_repo
            .find_by_id(&asset.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.current_version().to_string(), "1.0.0");
        assert_eq!(updated.publisher().map(String::as_str), Some("test_user"));
        assert_eq!(updated.state(), AssetState::Clean);
    }

    #[tokio::test]
    async fn publish_persists_dependency_snapshot_and_effective_baseline() {
        let org_id = OrganizationId::new();
        let project_id = ProjectId::new();
        let type_id = AssetTypeId::new();

        let asset_repo = InMemoryAssetRepository::new();
        let dirty_repo = InMemoryDirtyQueueRepository::new();

        let upstream = asset_repo
            .create(&create_asset_command(
                "Upstream",
                org_id,
                project_id,
                type_id,
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        let downstream = asset_repo
            .create(&create_asset_command(
                "Downstream",
                org_id,
                project_id,
                type_id,
                serde_json::json!({}),
            ))
            .await
            .unwrap();

        let service = VersionService::new(
            asset_repo,
            dirty_repo,
            InMemoryAssetVersionRepository::new(),
            InMemoryDependencyRepository::new(),
            InMemoryDirtyResolutionLogRepository::new(),
        );

        let version = service
            .publish(PublishAssetCommand {
                asset_id: downstream.id,
                version: "1.2.0".to_string(),
                publisher: "publisher".to_string(),
                release_notes: "link dependency".to_string(),
                dependencies: vec![PublishDependency {
                    upstream_asset_id: upstream.id,
                    version: "1.0.0".to_string(),
                }],
                suggested_type: None,
            })
            .await
            .unwrap();

        assert_eq!(version.dependencies.len(), 1);
        assert_eq!(version.dependencies[0].upstream_asset_id, upstream.id);
        assert_eq!(version.dependencies[0].upstream_version, "1.0.0");

        let records = service
            .dependency_repo
            .find_upstream_dependencies(&downstream.id)
            .await
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].source_id, downstream.id);
        assert_eq!(records[0].target_id, upstream.id);
        assert_eq!(records[0].declared_version, "1.0.0");
        assert_eq!(records[0].effective_version, "1.0.0");
        assert_eq!(records[0].effective_updated_by, "publisher");
        assert_eq!(records[0].effective_reason, EffectiveUpdateReason::Publish);
    }

    #[tokio::test]
    async fn publish_marks_direct_downstream_assets_dirty() {
        let org_id = OrganizationId::new();
        let project_id = ProjectId::new();
        let type_id = AssetTypeId::new();

        let asset_repo = InMemoryAssetRepository::new();
        let upstream = asset_repo
            .create(&create_asset_command(
                "Upstream",
                org_id,
                project_id,
                type_id,
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        let downstream = asset_repo
            .create(&create_asset_command(
                "Downstream",
                org_id,
                project_id,
                type_id,
                serde_json::json!({}),
            ))
            .await
            .unwrap();

        let dependency_repo = InMemoryDependencyRepository::new();
        dependency_repo
            .create_dependency_record(&AssetDependencyRecord {
                id: uuid::Uuid::new_v4(),
                source_id: downstream.id,
                target_id: upstream.id,
                relationship: "depends_on".to_string(),
                declared_version: "1.0.0".to_string(),
                effective_version: "1.0.0".to_string(),
                effective_updated_by: "publisher".to_string(),
                effective_updated_at: chrono::Utc::now(),
                effective_reason: EffectiveUpdateReason::Publish,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();

        let service = VersionService::new(
            asset_repo,
            InMemoryDirtyQueueRepository::new(),
            InMemoryAssetVersionRepository::new(),
            dependency_repo,
            InMemoryDirtyResolutionLogRepository::new(),
        );

        service
            .publish(publish_command(upstream.id, "1.1.0", "publisher"))
            .await
            .unwrap();

        let downstream_asset = service
            .asset_repo
            .find_by_id(&downstream.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(downstream_asset.state(), AssetState::Dirty);

        let dirty_entries = service
            .dirty_repo
            .find_unresolved_by_asset(&downstream.id)
            .await
            .unwrap();
        assert_eq!(dirty_entries.len(), 1);
        assert_eq!(dirty_entries[0].upstream_asset_id, upstream.id);
        assert_eq!(dirty_entries[0].upstream_version, "1.1.0");
    }

    #[tokio::test]
    async fn publish_fails_for_archived_asset() {
        let org_id = OrganizationId::new();
        let project_id = ProjectId::new();
        let type_id = AssetTypeId::new();

        let asset_repo = InMemoryAssetRepository::new();
        let dirty_repo = InMemoryDirtyQueueRepository::new();

        let cmd = create_asset_command(
            "Test Asset",
            org_id,
            project_id,
            type_id,
            serde_json::json!({}),
        );
        let asset = asset_repo.create(&cmd).await.unwrap();

        let service = VersionService::new(
            asset_repo,
            dirty_repo,
            InMemoryAssetVersionRepository::new(),
            InMemoryDependencyRepository::new(),
            InMemoryDirtyResolutionLogRepository::new(),
        );

        // Archive the asset
        service
            .asset_repo
            .update_state(&asset.id, AssetState::Archived)
            .await
            .unwrap();

        let result = service
            .publish(publish_command(asset.id, "1.0.0", "test_user"))
            .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            VersionServiceError::InvalidState(_)
        ));
    }

    #[test]
    fn suggest_version_major() {
        let next = TestVersionService::suggest_version("1.2.3", ChangeType::Breaking).unwrap();
        assert_eq!(next, "2.0.0");
    }

    #[test]
    fn suggest_version_minor() {
        let next = TestVersionService::suggest_version("1.2.3", ChangeType::Feature).unwrap();
        assert_eq!(next, "1.3.0");
    }

    #[test]
    fn suggest_version_patch() {
        let next = TestVersionService::suggest_version("1.2.3", ChangeType::Bugfix).unwrap();
        assert_eq!(next, "1.2.4");
    }

    #[test]
    fn suggest_version_invalid_input() {
        let result = TestVersionService::suggest_version("not-a-version", ChangeType::Bugfix);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn manual_clean_resolves_dirty() {
        let org_id = OrganizationId::new();
        let project_id = ProjectId::new();
        let type_id = AssetTypeId::new();

        let asset_repo = InMemoryAssetRepository::new();
        let dirty_repo = InMemoryDirtyQueueRepository::new();

        let cmd = create_asset_command(
            "Test Asset",
            org_id,
            project_id,
            type_id,
            serde_json::json!({}),
        );
        let asset = asset_repo.create(&cmd).await.unwrap();

        // Make asset dirty
        asset_repo
            .update_state(&asset.id, AssetState::Dirty)
            .await
            .unwrap();

        let upstream_id = AssetId::new();
        let dependency_repo = InMemoryDependencyRepository::new();
        dependency_repo
            .create_dependency_record(&AssetDependencyRecord {
                id: uuid::Uuid::new_v4(),
                source_id: asset.id,
                target_id: upstream_id,
                relationship: "depends_on".to_string(),
                declared_version: "0.0.0".to_string(),
                effective_version: "0.0.0".to_string(),
                effective_updated_by: "publisher".to_string(),
                effective_updated_at: chrono::Utc::now(),
                effective_reason: EffectiveUpdateReason::Publish,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();

        // Add a dirty queue entry
        let entry = DirtyQueueEntry {
            id: uuid::Uuid::new_v4(),
            asset_id: asset.id,
            upstream_asset_id: upstream_id,
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
            dependency_repo,
            InMemoryDirtyResolutionLogRepository::new(),
        );

        // Resolve dirty
        service
            .manual_clean(manual_clean_command(
                asset.id,
                "1.0.1",
                "reviewer",
                vec![ManualCleanResolution {
                    upstream_asset_id: upstream_id,
                    from_version: "0.0.0".to_string(),
                    to_version: "1.0.0".to_string(),
                    review_result: "no_impact".to_string(),
                    comment: Some("仅文档更新".to_string()),
                }],
            ))
            .await
            .unwrap();

        // Verify asset is now clean
        let updated = service
            .asset_repo
            .find_by_id(&asset.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.state(), AssetState::Clean);

        let records = service
            .dependency_repo
            .find_upstream_dependencies(&asset.id)
            .await
            .unwrap();
        assert_eq!(records[0].effective_version, "1.0.0");
        assert_eq!(records[0].effective_updated_by, "reviewer");
        assert_eq!(
            records[0].effective_reason,
            EffectiveUpdateReason::ManualClean
        );

        let logs = service
            .dirty_log_repo
            .find_by_asset(&asset.id)
            .await
            .unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].upstream_asset_id, upstream_id);
        assert_eq!(logs[0].from_version, "0.0.0");
        assert_eq!(logs[0].to_version, "1.0.0");
        assert_eq!(logs[0].review_result, "no_impact");
    }

    #[tokio::test]
    async fn manual_clean_partial_resolution_keeps_asset_dirty() {
        let org_id = OrganizationId::new();
        let project_id = ProjectId::new();
        let type_id = AssetTypeId::new();

        let asset_repo = InMemoryAssetRepository::new();
        let dirty_repo = InMemoryDirtyQueueRepository::new();
        let dependency_repo = InMemoryDependencyRepository::new();

        let asset = asset_repo
            .create(&create_asset_command(
                "Test Asset",
                org_id,
                project_id,
                type_id,
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        asset_repo
            .update_state(&asset.id, AssetState::Dirty)
            .await
            .unwrap();

        let upstream_a = AssetId::new();
        let upstream_b = AssetId::new();
        for upstream_id in [upstream_a, upstream_b] {
            dependency_repo
                .create_dependency_record(&AssetDependencyRecord {
                    id: uuid::Uuid::new_v4(),
                    source_id: asset.id,
                    target_id: upstream_id,
                    relationship: "depends_on".to_string(),
                    declared_version: "1.0.0".to_string(),
                    effective_version: "1.0.0".to_string(),
                    effective_updated_by: "publisher".to_string(),
                    effective_updated_at: chrono::Utc::now(),
                    effective_reason: EffectiveUpdateReason::Publish,
                    created_at: chrono::Utc::now(),
                })
                .await
                .unwrap();
            dirty_repo
                .upsert(&DirtyQueueEntry {
                    id: uuid::Uuid::new_v4(),
                    asset_id: asset.id,
                    upstream_asset_id: upstream_id,
                    upstream_version: "1.1.0".to_string(),
                    upstream_old_version: "1.0.0".to_string(),
                    impact_level: "medium".to_string(),
                    since: chrono::Utc::now(),
                    created_at: chrono::Utc::now(),
                    resolved_at: None,
                })
                .await
                .unwrap();
        }

        let service = VersionService::new(
            asset_repo,
            dirty_repo,
            InMemoryAssetVersionRepository::new(),
            dependency_repo,
            InMemoryDirtyResolutionLogRepository::new(),
        );

        service
            .manual_clean(manual_clean_command(
                asset.id,
                "1.0.1",
                "reviewer",
                vec![ManualCleanResolution {
                    upstream_asset_id: upstream_a,
                    from_version: "1.0.0".to_string(),
                    to_version: "1.1.0".to_string(),
                    review_result: "accepted".to_string(),
                    comment: None,
                }],
            ))
            .await
            .unwrap();

        let updated = service
            .asset_repo
            .find_by_id(&asset.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.state(), AssetState::Dirty);

        let remaining = service
            .dirty_repo
            .find_unresolved_by_asset(&asset.id)
            .await
            .unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].upstream_asset_id, upstream_b);

        let logs = service
            .dirty_log_repo
            .find_by_asset(&asset.id)
            .await
            .unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].upstream_asset_id, upstream_a);
    }

    #[tokio::test]
    async fn manual_clean_does_not_dirty_downstream_assets() {
        let org_id = OrganizationId::new();
        let project_id = ProjectId::new();
        let type_id = AssetTypeId::new();

        let asset_repo = InMemoryAssetRepository::new();
        let dirty_repo = InMemoryDirtyQueueRepository::new();
        let dependency_repo = InMemoryDependencyRepository::new();

        let asset = asset_repo
            .create(&create_asset_command(
                "Current",
                org_id,
                project_id,
                type_id,
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        let downstream = asset_repo
            .create(&create_asset_command(
                "Downstream",
                org_id,
                project_id,
                type_id,
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        asset_repo
            .update_state(&asset.id, AssetState::Dirty)
            .await
            .unwrap();

        let upstream_id = AssetId::new();
        for (source_id, target_id) in [(asset.id, upstream_id), (downstream.id, asset.id)] {
            dependency_repo
                .create_dependency_record(&AssetDependencyRecord {
                    id: uuid::Uuid::new_v4(),
                    source_id,
                    target_id,
                    relationship: "depends_on".to_string(),
                    declared_version: "1.0.0".to_string(),
                    effective_version: "1.0.0".to_string(),
                    effective_updated_by: "publisher".to_string(),
                    effective_updated_at: chrono::Utc::now(),
                    effective_reason: EffectiveUpdateReason::Publish,
                    created_at: chrono::Utc::now(),
                })
                .await
                .unwrap();
        }
        dirty_repo
            .upsert(&DirtyQueueEntry {
                id: uuid::Uuid::new_v4(),
                asset_id: asset.id,
                upstream_asset_id: upstream_id,
                upstream_version: "1.1.0".to_string(),
                upstream_old_version: "1.0.0".to_string(),
                impact_level: "medium".to_string(),
                since: chrono::Utc::now(),
                created_at: chrono::Utc::now(),
                resolved_at: None,
            })
            .await
            .unwrap();

        let service = VersionService::new(
            asset_repo,
            dirty_repo,
            InMemoryAssetVersionRepository::new(),
            dependency_repo,
            InMemoryDirtyResolutionLogRepository::new(),
        );

        service
            .manual_clean(manual_clean_command(
                asset.id,
                "1.0.1",
                "reviewer",
                vec![ManualCleanResolution {
                    upstream_asset_id: upstream_id,
                    from_version: "1.0.0".to_string(),
                    to_version: "1.1.0".to_string(),
                    review_result: "accepted".to_string(),
                    comment: None,
                }],
            ))
            .await
            .unwrap();

        let downstream_asset = service
            .asset_repo
            .find_by_id(&downstream.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(downstream_asset.state(), AssetState::Clean);
        assert!(
            service
                .dirty_repo
                .find_unresolved_by_asset(&downstream.id)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn manual_clean_fails_for_non_dirty() {
        let org_id = OrganizationId::new();
        let project_id = ProjectId::new();
        let type_id = AssetTypeId::new();

        let asset_repo = InMemoryAssetRepository::new();
        let dirty_repo = InMemoryDirtyQueueRepository::new();

        let cmd = create_asset_command(
            "Test Asset",
            org_id,
            project_id,
            type_id,
            serde_json::json!({}),
        );
        let asset = asset_repo.create(&cmd).await.unwrap();

        let service = VersionService::new(
            asset_repo,
            dirty_repo,
            InMemoryAssetVersionRepository::new(),
            InMemoryDependencyRepository::new(),
            InMemoryDirtyResolutionLogRepository::new(),
        );

        // Try to clean a Clean asset
        let result = service
            .manual_clean(manual_clean_command(asset.id, "1.0.1", "reviewer", vec![]))
            .await;

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

        let cmd = create_asset_command(
            "Test Asset",
            org_id,
            project_id,
            type_id,
            serde_json::json!({}),
        );
        let asset = asset_repo.create(&cmd).await.unwrap();

        let service = VersionService::new(
            asset_repo,
            dirty_repo,
            InMemoryAssetVersionRepository::new(),
            InMemoryDependencyRepository::new(),
            InMemoryDirtyResolutionLogRepository::new(),
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
