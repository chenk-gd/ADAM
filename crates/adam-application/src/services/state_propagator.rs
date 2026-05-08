//! State propagation service

use adam_domain::{
    AssetId, AssetInstance, AssetRepository, AssetState, AssetTypeId, CreateAssetCommand,
    DependencyRepository, DirtyQueueEntry, DirtyQueueRepository, OrganizationId, ProjectId,
    RepositoryError,
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
}

/// Service for propagating state changes through the dependency graph
pub struct StatePropagator;

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

        // Find all downstream assets (assets that depend on this one)
        let downstream = dependency_repo.find_downstream(asset_id).await?;
        let mut affected = Vec::new();

        for downstream_id in downstream {
            let downstream_asset = asset_repo.find_by_id(&downstream_id).await?;

            // Skip archived downstream assets
            if let Some(downstream) = downstream_asset {
                if downstream.is_archived() {
                    continue;
                }
            }

            // Create or update dirty queue entry
            let entry = DirtyQueueEntry {
                id: uuid::Uuid::new_v4(),
                asset_id: downstream_id,
                upstream_asset_id: *asset_id,
                upstream_version: new_version.to_string(),
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
        AssetId, AssetInstance, AssetRepository, AssetState, AssetTypeId, CreateAssetCommand,
        DependencyRepository, DirtyQueueEntry, DirtyQueueRepository, OrganizationId, ProjectId,
        RepositoryError,
    };
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct InMemoryAssetRepo {
        data: Mutex<HashMap<AssetId, AssetInstance>>,
    }

    impl InMemoryAssetRepo {
        fn new() -> Self {
            Self {
                data: Mutex::new(HashMap::new()),
            }
        }

        fn with_data(assets: Vec<AssetInstance>) -> Self {
            let map: HashMap<_, _> = assets.into_iter().map(|a| (a.id, a)).collect();
            Self { data: Mutex::new(map) }
        }
    }

    #[async_trait]
    impl AssetRepository for InMemoryAssetRepo {
        async fn create(
            &self,
            _cmd: &CreateAssetCommand,
        ) -> Result<AssetInstance, RepositoryError> {
            unimplemented!()
        }
        async fn find_by_id(
            &self,
            id: &AssetId,
        ) -> Result<Option<AssetInstance>, RepositoryError> {
            Ok(self.data.lock().unwrap().get(id).cloned())
        }
        async fn update_state(&self, _id: &AssetId, _state: AssetState) -> Result<(), RepositoryError> {
            Ok(())
        }
        async fn find_by_project_id(
            &self,
            _id: &ProjectId,
        ) -> Result<Vec<AssetInstance>, RepositoryError> {
            Ok(Vec::new())
        }
        async fn find_by_organization_id(
            &self,
            _id: &OrganizationId,
        ) -> Result<Vec<AssetInstance>, RepositoryError> {
            Ok(Vec::new())
        }
    }

    struct InMemoryDependencyRepo {
        downstream: Mutex<HashMap<AssetId, Vec<AssetId>>>,
    }

    impl InMemoryDependencyRepo {
        fn new() -> Self {
            Self {
                downstream: Mutex::new(HashMap::new()),
            }
        }

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
        async fn find_upstream(&self, _asset_id: &AssetId) -> Result<Vec<AssetId>, RepositoryError> {
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

    struct InMemoryDirtyQueueRepo {
        data: Mutex<Vec<DirtyQueueEntry>>,
    }

    impl InMemoryDirtyQueueRepo {
        fn new() -> Self {
            Self {
                data: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl DirtyQueueRepository for InMemoryDirtyQueueRepo {
        async fn upsert(&self, entry: &DirtyQueueEntry) -> Result<(), RepositoryError> {
            let mut data = self.data.lock().unwrap();
            // Check for existing unresolved entry
            if let Some(existing) = data.iter_mut().find(|e| {
                e.asset_id == entry.asset_id
                    && e.upstream_asset_id == entry.upstream_asset_id
                    && e.resolved_at.is_none()
            }) {
                existing.upstream_version.clone_from(&entry.upstream_version);
            } else {
                data.push(entry.clone());
            }
            Ok(())
        }

        async fn find_unresolved_by_asset(
            &self,
            _asset_id: &AssetId,
        ) -> Result<Vec<DirtyQueueEntry>, RepositoryError> {
            Ok(Vec::new())
        }
        async fn resolve(&self, _entry_id: &uuid::Uuid) -> Result<(), RepositoryError> {
            Ok(())
        }
        async fn find_all_unresolved(&self) -> Result<Vec<DirtyQueueEntry>, RepositoryError> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn propagate_dirty_creates_dirty_queue_entries() {
        // Setup: B -> A, C -> A (B and C depend on A)
        let org_id = OrganizationId::new();
        let type_id = AssetTypeId::new();

        let asset_a = AssetInstance::new_organization_level("Asset A", type_id, org_id);
        let asset_b = AssetInstance::new_organization_level("Asset B", type_id, org_id);
        let asset_c = AssetInstance::new_organization_level("Asset C", type_id, org_id);

        let asset_repo = InMemoryAssetRepo::with_data(vec![asset_a.clone(), asset_b.clone(), asset_c.clone()]);
        let dependency_repo = InMemoryDependencyRepo {
            downstream: Mutex::new({
                let mut map = HashMap::new();
                map.insert(asset_a.id, vec![asset_b.id, asset_c.id]);
                map
            }),
        };
        let dirty_repo = InMemoryDirtyQueueRepo::new();
        let propagator = StatePropagator::new();

        // When A publishes v2.0.0
        let affected = propagator
            .on_asset_published(&asset_a.id, "v2.0.0", &asset_repo, &dependency_repo, &dirty_repo)
            .await
            .unwrap();

        // Assert: B and C have dirty entries
        assert_eq!(affected.len(), 2);
        assert!(affected.contains(&asset_b.id));
        assert!(affected.contains(&asset_c.id));
    }

    #[tokio::test]
    async fn propagate_dirty_updates_existing_dirty_entry() {
        let org_id = OrganizationId::new();
        let type_id = AssetTypeId::new();

        let asset_a = AssetInstance::new_organization_level("Asset A", type_id, org_id);
        let asset_b = AssetInstance::new_organization_level("Asset B", type_id, org_id);

        // Create initial dirty entry (from v1.0.0)
        let dirty_repo = InMemoryDirtyQueueRepo {
            data: Mutex::new(vec![DirtyQueueEntry {
                id: uuid::Uuid::new_v4(),
                asset_id: asset_b.id,
                upstream_asset_id: asset_a.id,
                upstream_version: "v1.0.0".to_string(),
                created_at: chrono::Utc::now(),
                resolved_at: None,
            }]),
        };

        let asset_repo = InMemoryAssetRepo::with_data(vec![asset_a.clone(), asset_b.clone()]);
        let dependency_repo = InMemoryDependencyRepo::with_dependency(asset_b.id, asset_a.id);
        let propagator = StatePropagator::new();

        // When A publishes v2.0.0
        let affected = propagator
            .on_asset_published(&asset_a.id, "v2.0.0", &asset_repo, &dependency_repo, &dirty_repo)
            .await
            .unwrap();

        assert_eq!(affected.len(), 1);
        assert_eq!(affected[0], asset_b.id);

        // Verify version was updated
        let entries = dirty_repo.data.lock().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].upstream_version, "v2.0.0");
    }

    #[tokio::test]
    async fn archived_downstream_is_skipped() {
        let org_id = OrganizationId::new();
        let type_id = AssetTypeId::new();

        let asset_a = AssetInstance::new_organization_level("Asset A", type_id, org_id);
        // Create B as archived
        let mut asset_b = AssetInstance::new_organization_level("Asset B", type_id, org_id);
        asset_b.current_state = AssetState::Archived;

        let asset_c = AssetInstance::new_organization_level("Asset C", type_id, org_id);

        let asset_repo =
            InMemoryAssetRepo::with_data(vec![asset_a.clone(), asset_b.clone(), asset_c.clone()]);
        let dependency_repo = InMemoryDependencyRepo {
            downstream: Mutex::new({
                let mut map = HashMap::new();
                map.insert(asset_a.id, vec![asset_b.id, asset_c.id]);
                map
            }),
        };
        let dirty_repo = InMemoryDirtyQueueRepo::new();
        let propagator = StatePropagator::new();

        // When A publishes
        let affected = propagator
            .on_asset_published(&asset_a.id, "v2.0.0", &asset_repo, &dependency_repo, &dirty_repo)
            .await
            .unwrap();

        // Only C should be affected
        assert_eq!(affected.len(), 1);
        assert_eq!(affected[0], asset_c.id);
    }

    #[tokio::test]
    async fn archived_upstream_does_not_trigger_dirty() {
        let org_id = OrganizationId::new();
        let type_id = AssetTypeId::new();

        // Create archived A
        let mut asset_a = AssetInstance::new_organization_level("Asset A", type_id, org_id);
        asset_a.current_state = AssetState::Archived;

        let asset_b = AssetInstance::new_organization_level("Asset B", type_id, org_id);

        let asset_repo = InMemoryAssetRepo::with_data(vec![asset_a.clone(), asset_b.clone()]);
        let dependency_repo = InMemoryDependencyRepo::with_dependency(asset_b.id, asset_a.id);
        let dirty_repo = InMemoryDirtyQueueRepo::new();
        let propagator = StatePropagator::new();

        // When archived A tries to publish
        let result = propagator
            .on_asset_published(&asset_a.id, "v2.0.0", &asset_repo, &dependency_repo, &dirty_repo)
            .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            StatePropagationError::ArchivedAssetCannotTrigger
        ));

        // Verify dirty queue is empty
        let entries = dirty_repo.data.lock().unwrap();
        assert!(entries.is_empty());
    }
}
