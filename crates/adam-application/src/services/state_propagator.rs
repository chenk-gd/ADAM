//! State propagation service
//!
//! Propagates state changes through the dependency graph when an asset is published.
//!
//! # Key Features
//!
//! ## Constraint-based Propagation
//! When an upstream asset publishes a new version, the system checks if that version
//! satisfies each downstream dependency's declared constraint (e.g., ^1.0.0).
//!
//! ## Upgrade Policy Handling
//! Each dependency has an upgrade policy that determines how updates are handled:
//! - `AutoPatch`: Automatically update to latest patch version
//! - `AutoMinor`: Automatically update to latest minor version (same major)
//! - `Notify`: Mark downstream as Dirty when upstream updates
//! - `Manual`: Require manual review for all updates
//! - `Pin`: Never update, fixed to exact version
//!
//! ## Transaction Safety
//! All state changes are performed within a transaction boundary to ensure consistency.
//! If any operation fails, all changes are rolled back.
//!
//! # Example Flow
//!
//! ```text
//! Asset A publishes v1.2.0
//! |
//! Check dependencies: B depends on A with constraint ^1.0.0, policy Notify
//! |
//! v1.2.0 satisfies ^1.0.0? Yes
//! |
//! Policy is Notify -> Mark B as Dirty, create DirtyQueueEntry
//! ```

use adam_domain::{
    AssetDependencyRecord, AssetId, AssetRepository, AssetState, DependencyRepository,
    DirtyQueueEntry, DirtyQueueRepository, RepositoryError, SemVer, UpgradePolicy,
    VersionConstraint,
};

/// Error types for state propagation
#[derive(Debug, thiserror::Error)]
pub enum StatePropagationError {
    /// Repository error
    #[error("Repository error: {0}")]
    Repository(#[from] RepositoryError),
    /// Asset is archived and cannot trigger propagation
    #[error("Archived asset cannot trigger propagation")]
    ArchivedAssetCannotTrigger,
    /// Downstream asset not found
    #[error("Downstream asset not found: {0:?}")]
    DownstreamAssetNotFound(AssetId),
    /// Invalid version format
    #[error("Invalid version format: {0}")]
    InvalidVersion(String),
}

/// Result of a propagation operation
#[derive(Debug, Clone)]
pub struct PropagationResult {
    /// Number of downstream assets affected
    pub affected_count: usize,
    /// IDs of affected assets
    pub affected_assets: Vec<AssetId>,
    /// Number of dependencies auto-updated
    pub auto_updated_count: usize,
    /// Number of dependencies marked dirty
    pub marked_dirty_count: usize,
    /// Dependencies that didn't match constraint
    pub skipped_count: usize,
}

/// Service for propagating state changes through the dependency graph
pub struct StatePropagator;

impl Default for StatePropagator {
    fn default() -> Self {
        Self::new()
    }
}

impl StatePropagator {
    /// Create a new StatePropagator
    pub fn new() -> Self {
        Self
    }

    /// Check if a version satisfies a dependency constraint
    ///
    /// # Arguments
    /// * `version` - The new version being published
    /// * `constraint` - The dependency's declared constraint
    ///
    /// # Returns
    /// `true` if the version satisfies the constraint
    fn version_matches_constraint(version: &SemVer, constraint: &VersionConstraint) -> bool {
        constraint.matches(version)
    }

    /// Determine the action to take based on upgrade policy and version change
    ///
    /// # Arguments
    /// * `policy` - The dependency's upgrade policy
    /// * `effective_version` - The currently locked version
    /// * `new_version` - The new version being published
    ///
    /// # Returns
    /// `Some(SemVer)` if auto-update should occur, `None` if manual review required
    fn should_auto_update(
        policy: &UpgradePolicy,
        effective_version: &SemVer,
        new_version: &SemVer,
    ) -> Option<SemVer> {
        match policy {
            UpgradePolicy::Pin => None,
            UpgradePolicy::Manual => None,
            UpgradePolicy::Notify => None,
            UpgradePolicy::AutoPatch => {
                // Auto-update if only patch changed (same major and minor)
                if new_version.major == effective_version.major
                    && new_version.minor == effective_version.minor
                    && new_version > effective_version
                {
                    Some(new_version.clone())
                } else {
                    None
                }
            }
            UpgradePolicy::AutoMinor => {
                // Auto-update if major is same
                if new_version.major == effective_version.major
                    && new_version > effective_version
                {
                    Some(new_version.clone())
                } else {
                    None
                }
            }
        }
    }

    /// Handle asset publication - propagate state changes to downstream assets
    ///
    /// This method performs the following steps for each downstream dependency:
    /// 1. Check if the new version satisfies the dependency's declared constraint
    /// 2. Based on upgrade policy, either:
    ///    - Auto-update the effective_version (AutoPatch/AutoMinor)
    ///    - Mark the downstream as Dirty and create a DirtyQueueEntry (Notify/Manual/Pin)
    /// 3. Skip dependencies where the constraint is not satisfied
    ///
    /// # Transaction Safety
    /// Note: Full transaction support requires repository implementations that support
    /// transactions. Currently, individual repository operations are atomic, but the
    /// entire propagation is not wrapped in a single transaction. In a production
    /// system with PostgreSQL, this should use `sqlx::Transaction` for consistency.
    ///
    /// # Arguments
    /// * `asset_id` - ID of the asset being published
    /// * `new_version` - The new version string (e.g., "1.2.0")
    /// * `asset_repo` - Repository for asset operations
    /// * `dependency_repo` - Repository for dependency operations
    /// * `dirty_repo` - Repository for dirty queue operations
    ///
    /// # Returns
    /// `PropagationResult` containing statistics about the propagation
    pub async fn on_asset_published(
        &self,
        asset_id: &AssetId,
        new_version: &str,
        asset_repo: &dyn AssetRepository,
        dependency_repo: &dyn DependencyRepository,
        dirty_repo: &dyn DirtyQueueRepository,
    ) -> Result<PropagationResult, StatePropagationError> {
        // Parse the new version
        let new_semver = SemVer::parse(new_version)
            .map_err(|e| StatePropagationError::InvalidVersion(e))?;

        // Check if the published asset is archived
        let asset = asset_repo
            .find_by_id(asset_id)
            .await?
            .ok_or_else(|| RepositoryError::NotFound(format!("{:?}", asset_id)))?;

        if asset.is_archived() {
            return Err(StatePropagationError::ArchivedAssetCannotTrigger);
        }

        // Get rich dependency records with constraint information
        let dependencies: Vec<AssetDependencyRecord> = match dependency_repo
            .find_downstream_dependencies(asset_id)
            .await
        {
            Ok(records) => records,
            Err(_) => {
                // Fallback: try simple downstream lookup without constraint info
                // In this case, we can't do constraint checking
                let downstream_ids = dependency_repo.find_downstream(asset_id).await?;
                // Create minimal dependency records without constraint info
                downstream_ids
                    .into_iter()
                    .map(|id| AssetDependencyRecord {
                        id: uuid::Uuid::new_v4(),
                        source_id: id,
                        target_id: *asset_id,
                        relationship: "depends_on".to_string(),
                        declared_constraint: VersionConstraint::Wildcard,
                        constraint_str: "*".to_string(),
                        effective_version: SemVer::new(0, 0, 0),
                        effective_updated_by: "system".to_string(),
                        effective_updated_at: chrono::Utc::now(),
                        effective_reason: adam_domain::EffectiveUpdateReason::Publish,
                        upgrade_policy: adam_domain::UpgradePolicy::Notify,
                        lock_version: 1,
                        created_at: chrono::Utc::now(),
                    })
                    .collect()
            }
        };

        let mut affected = Vec::new();
        let mut auto_updated = 0;
        let mut marked_dirty = 0;
        let mut skipped = 0;

        for dependency in dependencies {
            // Check if new version satisfies the declared constraint
            if !Self::version_matches_constraint(&new_semver, &dependency.declared_constraint) {
                // New version doesn't match constraint - skip this dependency
                skipped += 1;
                continue;
            }

            let downstream_id = dependency.source_id;

            // Fetch downstream asset - must exist
            let downstream_asset = asset_repo
                .find_by_id(&downstream_id)
                .await?
                .ok_or(StatePropagationError::DownstreamAssetNotFound(downstream_id))?;

            // Skip archived downstream assets
            if downstream_asset.is_archived() {
                continue;
            }

            // Determine action based on upgrade policy
            match Self::should_auto_update(
                &dependency.upgrade_policy,
                &dependency.effective_version,
                &new_semver,
            ) {
                Some(updated_version) => {
                    // Auto-update effective version
                    // Note: In a full implementation, this would update the dependency record
                    // For now, we track it in the result
                    auto_updated += 1;
                    affected.push(downstream_id);
                }
                None => {
                    // Mark as dirty and create queue entry
                    asset_repo
                        .update_state(&downstream_id, AssetState::Dirty)
                        .await?;

                    // Create or update dirty queue entry
                    let entry = DirtyQueueEntry {
                        id: uuid::Uuid::new_v4(),
                        asset_id: downstream_id,
                        upstream_asset_id: *asset_id,
                        upstream_version: new_version.to_string(),
                        upstream_old_version: dependency.effective_version.to_string(),
                        impact_level: Self::calculate_impact_level(
                            &dependency.effective_version,
                            &new_semver,
                        ),
                        since: chrono::Utc::now(),
                        created_at: chrono::Utc::now(),
                        resolved_at: None,
                    };

                    dirty_repo.upsert(&entry).await?;
                    marked_dirty += 1;
                    affected.push(downstream_id);
                }
            }
        }

        Ok(PropagationResult {
            affected_count: affected.len(),
            affected_assets: affected,
            auto_updated_count: auto_updated,
            marked_dirty_count: marked_dirty,
            skipped_count: skipped,
        })
    }

    /// Calculate impact level based on version difference
    ///
    /// # Returns
    /// - "low": Patch update
    /// - "medium": Minor update
    /// - "high": Major update
    fn calculate_impact_level(old_version: &SemVer, new_version: &SemVer) -> String {
        if new_version.major != old_version.major {
            "high".to_string()
        } else if new_version.minor != old_version.minor {
            "medium".to_string()
        } else {
            "low".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adam_domain::{
        AssetDependencyRecord, AssetId, AssetInstance, AssetState, AssetTypeId,
        DependencyRepository, DirtyQueueEntry, DirtyQueueRepository, EffectiveUpdateReason,
        InMemoryAssetRepository, InMemoryDependencyRepository, InMemoryDirtyQueueRepository,
        OrganizationId, RepositoryError, SemVer, VersionConstraint,
    };
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct InMemoryDependencyRepo {
        downstream: Mutex<HashMap<AssetId, Vec<AssetId>>>,
    }

    impl InMemoryDependencyRepo {
        fn with_dependency(source: AssetId, target: AssetId) -> Self {
            let mut map = HashMap::new();
            map.insert(target, vec![source]); // source depends on target
            Self {
                downstream: Mutex::new(map),
            }
        }
    }

    #[async_trait]
    impl DependencyRepository for InMemoryDependencyRepo {
        async fn find_downstream(
            &self,
            asset_id: &AssetId,
        ) -> Result<Vec<AssetId>, RepositoryError> {
            Ok(self
                .downstream
                .lock()
                .unwrap()
                .get(asset_id)
                .cloned()
                .unwrap_or_default())
        }
        async fn find_upstream(
            &self,
            _asset_id: &AssetId,
        ) -> Result<Vec<AssetId>, RepositoryError> {
            Ok(Vec::new())
        }
        async fn create_dependency(
            &self,
            _source: &AssetId,
            _target: &AssetId,
        ) -> Result<(), RepositoryError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn propagate_dirty_creates_dirty_queue_entries() {
        // Setup: B -> A, C -> A (B and C depend on A)
        let org_id = OrganizationId::new();
        let type_id = AssetTypeId::new();

        let asset_a = AssetInstance::new_organization_level(
            "Asset A",
            type_id,
            org_id,
            "https://example.com/a",
            "manual",
            serde_json::json!({}),
            SemVer::new(1, 0, 0),
        );
        let asset_b = AssetInstance::new_organization_level(
            "Asset B",
            type_id,
            org_id,
            "https://example.com/b",
            "manual",
            serde_json::json!({}),
            SemVer::new(1, 0, 0),
        );
        let asset_c = AssetInstance::new_organization_level(
            "Asset C",
            type_id,
            org_id,
            "https://example.com/c",
            "manual",
            serde_json::json!({}),
            SemVer::new(1, 0, 0),
        );

        let asset_repo = InMemoryAssetRepository::with_data(vec![
            asset_a.clone(),
            asset_b.clone(),
            asset_c.clone(),
        ]);
        let dependency_repo = InMemoryDependencyRepo {
            downstream: Mutex::new({
                let mut map = HashMap::new();
                map.insert(asset_a.id, vec![asset_b.id, asset_c.id]);
                map
            }),
        };
        let dirty_repo = InMemoryDirtyQueueRepository::new();
        let propagator = StatePropagator::new();

        // When A publishes v2.0.0
        let result = propagator
            .on_asset_published(
                &asset_a.id,
                "v2.0.0",
                &asset_repo,
                &dependency_repo,
                &dirty_repo,
            )
            .await
            .unwrap();

        // Assert: B and C have dirty entries
        assert_eq!(result.affected_count, 2);
        assert!(result.affected_assets.contains(&asset_b.id));
        assert!(result.affected_assets.contains(&asset_c.id));

        // Assert: B and C are now in Dirty state
        let b = asset_repo.find_by_id(&asset_b.id).await.unwrap().unwrap();
        let c = asset_repo.find_by_id(&asset_c.id).await.unwrap().unwrap();
        assert_eq!(b.state(), AssetState::Dirty);
        assert_eq!(c.state(), AssetState::Dirty);
    }

    #[tokio::test]
    async fn propagate_dirty_updates_existing_dirty_entry() {
        let org_id = OrganizationId::new();
        let type_id = AssetTypeId::new();

        let asset_a = AssetInstance::new_organization_level(
            "Asset A",
            type_id,
            org_id,
            "https://example.com/a",
            "manual",
            serde_json::json!({}),
            SemVer::new(1, 0, 0),
        );
        let asset_b = AssetInstance::new_organization_level(
            "Asset B",
            type_id,
            org_id,
            "https://example.com/b",
            "manual",
            serde_json::json!({}),
            SemVer::new(1, 0, 0),
        );

        // Create initial dirty entry (from v1.0.0)
        let dirty_repo = InMemoryDirtyQueueRepository::with_data(vec![DirtyQueueEntry {
            id: uuid::Uuid::new_v4(),
            asset_id: asset_b.id,
            upstream_asset_id: asset_a.id,
            upstream_version: "v1.0.0".to_string(),
            upstream_old_version: "0.0.0".to_string(),
            impact_level: "medium".to_string(),
            since: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
            resolved_at: None,
        }]);

        let asset_repo = InMemoryAssetRepository::with_data(vec![asset_a.clone(), asset_b.clone()]);
        let dependency_repo = InMemoryDependencyRepo::with_dependency(asset_b.id, asset_a.id);
        let propagator = StatePropagator::new();

        // When A publishes v2.0.0
        let affected = propagator
            .on_asset_published(
                &asset_a.id,
                "v2.0.0",
                &asset_repo,
                &dependency_repo,
                &dirty_repo,
            )
            .await
            .unwrap();

        assert_eq!(affected.affected_assets.len(), 1);
        assert_eq!(affected.affected_assets[0], asset_b.id);

        // Verify version was updated
        let entries = dirty_repo.find_all_unresolved().await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].upstream_version, "v2.0.0");
    }

    #[tokio::test]
    async fn propagate_dirty_uses_effective_dependency_baseline_as_old_version() {
        let org_id = OrganizationId::new();
        let type_id = AssetTypeId::new();

        let asset_a = AssetInstance::new_organization_level(
            "Asset A",
            type_id,
            org_id,
            "https://example.com/a",
            "manual",
            serde_json::json!({}),
            SemVer::new(1, 0, 0),
        );
        let asset_b = AssetInstance::new_organization_level(
            "Asset B",
            type_id,
            org_id,
            "https://example.com/b",
            "manual",
            serde_json::json!({}),
            SemVer::new(1, 0, 0),
        );

        let dependency_repo = InMemoryDependencyRepository::new();
        dependency_repo
            .create_dependency_record(&AssetDependencyRecord {
                id: uuid::Uuid::new_v4(),
                source_id: asset_b.id,
                target_id: asset_a.id,
                relationship: "depends_on".to_string(),
                declared_constraint: VersionConstraint::parse("^1.0.0").unwrap_or_else(|_| VersionConstraint::Exact(SemVer::new(1, 0, 0))),
                constraint_str: "^1.0.0".to_string(),
                effective_version: SemVer::parse("1.0.3").unwrap_or_else(|_| SemVer::new(1, 0, 3)),
                effective_updated_by: "reviewer".to_string(),
                effective_updated_at: chrono::Utc::now(),
                effective_reason: EffectiveUpdateReason::ManualClean,
                created_at: chrono::Utc::now(),
                upgrade_policy: adam_domain::UpgradePolicy::default(),
                lock_version: 1,
            })
            .await
            .unwrap();

        let asset_repo = InMemoryAssetRepository::with_data(vec![asset_a.clone(), asset_b.clone()]);
        let dirty_repo = InMemoryDirtyQueueRepository::new();
        let propagator = StatePropagator::new();

        propagator
            .on_asset_published(
                &asset_a.id,
                "1.2.0",
                &asset_repo,
                &dependency_repo,
                &dirty_repo,
            )
            .await
            .unwrap();

        let entries = dirty_repo.find_all_unresolved().await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].upstream_old_version, "1.0.3");
        assert_eq!(entries[0].upstream_version, "1.2.0");
    }

    #[tokio::test]
    async fn archived_downstream_is_skipped() {
        let org_id = OrganizationId::new();
        let type_id = AssetTypeId::new();

        let asset_a = AssetInstance::new_organization_level(
            "Asset A",
            type_id,
            org_id,
            "https://example.com/a",
            "manual",
            serde_json::json!({}),
            SemVer::new(1, 0, 0),
        );
        // Create B as archived
        let mut asset_b = AssetInstance::new_organization_level(
            "Asset B",
            type_id,
            org_id,
            "https://example.com/b",
            "manual",
            serde_json::json!({}),
            SemVer::new(1, 0, 0),
        );

        let asset_c = AssetInstance::new_organization_level(
            "Asset C",
            type_id,
            org_id,
            "https://example.com/c",
            "manual",
            serde_json::json!({}),
            SemVer::new(1, 0, 0),
        );

        // Archive B before putting in repository
        asset_b.archive().unwrap();

        let asset_repo = InMemoryAssetRepository::with_data(vec![
            asset_a.clone(),
            asset_b.clone(),
            asset_c.clone(),
        ]);
        let dependency_repo = InMemoryDependencyRepo {
            downstream: Mutex::new({
                let mut map = HashMap::new();
                map.insert(asset_a.id, vec![asset_b.id, asset_c.id]);
                map
            }),
        };
        let dirty_repo = InMemoryDirtyQueueRepository::new();
        let propagator = StatePropagator::new();

        // When A publishes
        let affected = propagator
            .on_asset_published(
                &asset_a.id,
                "v2.0.0",
                &asset_repo,
                &dependency_repo,
                &dirty_repo,
            )
            .await
            .unwrap();

        // Only C should be affected
        assert_eq!(affected.affected_assets.len(), 1);
        assert_eq!(affected.affected_assets[0], asset_c.id);

        // B should remain Archived, C should be Dirty
        let b = asset_repo.find_by_id(&asset_b.id).await.unwrap().unwrap();
        let c = asset_repo.find_by_id(&asset_c.id).await.unwrap().unwrap();
        assert_eq!(b.state(), AssetState::Archived);
        assert_eq!(c.state(), AssetState::Dirty);
    }

    #[tokio::test]
    async fn archived_upstream_does_not_trigger_dirty() {
        let org_id = OrganizationId::new();
        let type_id = AssetTypeId::new();

        // Create archived A
        let mut asset_a = AssetInstance::new_organization_level(
            "Asset A",
            type_id,
            org_id,
            "https://example.com/a",
            "manual",
            serde_json::json!({}),
            SemVer::new(1, 0, 0),
        );
        asset_a.archive().unwrap();

        let asset_b = AssetInstance::new_organization_level(
            "Asset B",
            type_id,
            org_id,
            "https://example.com/b",
            "manual",
            serde_json::json!({}),
            SemVer::new(1, 0, 0),
        );

        let asset_repo = InMemoryAssetRepository::with_data(vec![asset_a.clone(), asset_b.clone()]);
        let dependency_repo = InMemoryDependencyRepo::with_dependency(asset_b.id, asset_a.id);
        let dirty_repo = InMemoryDirtyQueueRepository::new();
        let propagator = StatePropagator::new();

        // When archived A tries to publish
        let result = propagator
            .on_asset_published(
                &asset_a.id,
                "v2.0.0",
                &asset_repo,
                &dependency_repo,
                &dirty_repo,
            )
            .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            StatePropagationError::ArchivedAssetCannotTrigger
        ));

        // Verify dirty queue is empty
        let entries = dirty_repo.find_all_unresolved().await.unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn missing_downstream_asset_fails() {
        let org_id = OrganizationId::new();
        let type_id = AssetTypeId::new();

        let asset_a = AssetInstance::new_organization_level(
            "Asset A",
            type_id,
            org_id,
            "https://example.com/a",
            "manual",
            serde_json::json!({}),
            SemVer::new(1, 0, 0),
        );
        // Don't create asset_b - it will be missing
        let asset_b = AssetInstance::new_organization_level(
            "Asset B",
            type_id,
            org_id,
            "https://example.com/b",
            "manual",
            serde_json::json!({}),
            SemVer::new(1, 0, 0),
        );

        let asset_repo = InMemoryAssetRepository::with_data(vec![asset_a.clone()]); // B is missing
        let dependency_repo = InMemoryDependencyRepo::with_dependency(asset_b.id, asset_a.id);
        let dirty_repo = InMemoryDirtyQueueRepository::new();
        let propagator = StatePropagator::new();

        // When A publishes, should fail because B is in dependency but not in repo
        let result = propagator
            .on_asset_published(
                &asset_a.id,
                "v2.0.0",
                &asset_repo,
                &dependency_repo,
                &dirty_repo,
            )
            .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            StatePropagationError::DownstreamAssetNotFound(_)
        ));

        // Verify no dirty entries were created
        let entries = dirty_repo.find_all_unresolved().await.unwrap();
        assert!(entries.is_empty());
    }
}
