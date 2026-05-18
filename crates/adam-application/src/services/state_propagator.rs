//! State propagation service

use adam_domain::{
    AssetId, AssetRepository, AssetState, DependencyRepository, DirtyQueueEntry,
    DirtyQueueRepository, RepositoryError, SemVer,
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

    /// Handle asset publication - propagate dirty state to downstream assets
    pub async fn on_asset_published(
        &self,
        asset_id: &AssetId,
        new_version: &str,
        asset_repo: &dyn AssetRepository,
        dependency_repo: &dyn DependencyRepository,
        dirty_repo: &dyn DirtyQueueRepository,
    ) -> Result<Vec<AssetId>, StatePropagationError> {
        // Check if the published asset is archived
        let asset = asset_repo
            .find_by_id(asset_id)
            .await?
            .ok_or_else(|| RepositoryError::NotFound(format!("{asset_id:?}")))?;

        if asset.is_archived() {
            return Err(StatePropagationError::ArchivedAssetCannotTrigger);
        }

        // Prefer rich dependency records so Dirty entries preserve the reviewed baseline.
        let downstream: Vec<(AssetId, Option<String>)> =
            match dependency_repo.find_downstream_dependencies(asset_id).await {
                Ok(records) => records
                    .into_iter()
                    .map(|record| (record.source_id, Some(record.effective_version.to_string())))
                    .collect(),
                Err(_) => dependency_repo
                    .find_downstream(asset_id)
                    .await?
                    .into_iter()
                    .map(|downstream_id| (downstream_id, None))
                    .collect(),
            };
        let mut affected = Vec::new();

        for (downstream_id, effective_version) in downstream {
            // Fetch downstream asset - must exist
            let downstream_asset = asset_repo.find_by_id(&downstream_id).await?.ok_or(
                StatePropagationError::DownstreamAssetNotFound(downstream_id),
            )?;

            // Skip archived downstream assets
            if downstream_asset.is_archived() {
                continue;
            }

            // Update downstream asset state to Dirty
            asset_repo
                .update_state(&downstream_id, AssetState::Dirty)
                .await?;

            // Create or update dirty queue entry
            let entry = DirtyQueueEntry {
                id: uuid::Uuid::new_v4(),
                asset_id: downstream_id,
                upstream_asset_id: *asset_id,
                upstream_version: new_version.to_string(),
                upstream_old_version: effective_version.unwrap_or_else(|| {
                    downstream_asset
                        .current_version()
                        .to_string()
                }),
                impact_level: "medium".to_string(),
                since: chrono::Utc::now(),
                created_at: chrono::Utc::now(),
                resolved_at: None,
            };

            dirty_repo.upsert(&entry).await?;
            affected.push(downstream_id);
        }

        Ok(affected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adam_domain::{
        AssetDependencyRecord, AssetId, AssetInstance, AssetState, AssetTypeId,
        DependencyRepository, DirtyQueueEntry, DirtyQueueRepository, EffectiveUpdateReason,
        InMemoryAssetRepository, InMemoryDependencyRepository, InMemoryDirtyQueueRepository,
        OrganizationId, RepositoryError,
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

        // Assert: B and C have dirty entries
        assert_eq!(affected.len(), 2);
        assert!(affected.contains(&asset_b.id));
        assert!(affected.contains(&asset_c.id));

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

        assert_eq!(affected.len(), 1);
        assert_eq!(affected[0], asset_b.id);

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
                declared_constraint: "1.0.0".to_string(),
                effective_version: "1.0.3".to_string(),
                effective_updated_by: "reviewer".to_string(),
                effective_updated_at: chrono::Utc::now(),
                effective_reason: EffectiveUpdateReason::ManualClean,
                created_at: chrono::Utc::now(),
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
        assert_eq!(affected.len(), 1);
        assert_eq!(affected[0], asset_c.id);

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
