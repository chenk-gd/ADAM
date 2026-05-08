//! Integration tests for in-memory repositories

use adam_domain::repository::{
    AssetRepository, CreateAssetCommand, DirtyQueueEntry, DirtyQueueRepository, RepositoryError,
};
use adam_domain::{AssetId, AssetInstance, AssetState, AssetTypeId, OrganizationId, ProjectId};
use chrono::Utc;

mod in_memory {
    use super::*;
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory asset repository for testing
    pub struct InMemoryAssetRepository {
        data: Mutex<HashMap<AssetId, AssetInstance>>,
    }

    impl InMemoryAssetRepository {
        pub fn new() -> Self {
            Self {
                data: Mutex::new(HashMap::new()),
            }
        }

        pub fn with_data(assets: Vec<AssetInstance>) -> Self {
            let data: HashMap<AssetId, AssetInstance> =
                assets.into_iter().map(|a| (a.id, a)).collect();
            Self {
                data: Mutex::new(data),
            }
        }
    }

    #[async_trait]
    impl AssetRepository for InMemoryAssetRepository {
        async fn create(
            &self,
            cmd: &CreateAssetCommand,
        ) -> Result<AssetInstance, RepositoryError> {
            let mut data = self.data.lock().map_err(|e| {
                RepositoryError::DatabaseError(format!("Mutex poisoned: {e}"))
            })?;

            // Check idempotency key
            if let Some(ref key) = cmd.idempotency_key {
                if data.values().any(|a| a.idempotency_key.as_ref() == Some(key)) {
                    return Err(RepositoryError::DuplicateIdempotencyKey(key.clone()));
                }
            }

            let asset = AssetInstance {
                id: AssetId::new(),
                name: cmd.name.clone(),
                asset_type_id: cmd.asset_type_id,
                project_id: cmd.project_id,
                organization_id: cmd.organization_id,
                level: cmd.level,
                current_state: AssetState::Clean,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                idempotency_key: cmd.idempotency_key.clone(),
            };

            data.insert(asset.id, asset.clone());
            Ok(asset)
        }

        async fn find_by_id(
            &self,
            id: &AssetId,
        ) -> Result<Option<AssetInstance>, RepositoryError> {
            let data = self.data.lock().map_err(|e| {
                RepositoryError::DatabaseError(format!("Mutex poisoned: {e}"))
            })?;
            Ok(data.get(id).cloned())
        }

        async fn update_state(
            &self,
            id: &AssetId,
            state: AssetState,
        ) -> Result<(), RepositoryError> {
            let mut data = self.data.lock().map_err(|e| {
                RepositoryError::DatabaseError(format!("Mutex poisoned: {e}"))
            })?;

            if let Some(asset) = data.get_mut(id) {
                asset.current_state = state;
                asset.updated_at = Utc::now();
                Ok(())
            } else {
                Err(RepositoryError::NotFound(format!("{id:?}")))
            }
        }

        async fn find_by_project_id(
            &self,
            project_id: &ProjectId,
        ) -> Result<Vec<AssetInstance>, RepositoryError> {
            let data = self.data.lock().map_err(|e| {
                RepositoryError::DatabaseError(format!("Mutex poisoned: {e}"))
            })?;

            Ok(data
                .values()
                .filter(|a| a.project_id.as_ref() == Some(project_id))
                .cloned()
                .collect())
        }

        async fn find_by_organization_id(
            &self,
            org_id: &OrganizationId,
        ) -> Result<Vec<AssetInstance>, RepositoryError> {
            let data = self.data.lock().map_err(|e| {
                RepositoryError::DatabaseError(format!("Mutex poisoned: {e}"))
            })?;

            Ok(data
                .values()
                .filter(|a| a.organization_id == *org_id)
                .cloned()
                .collect())
        }
    }

    /// In-memory dirty queue repository for testing
    pub struct InMemoryDirtyQueueRepository {
        data: Mutex<Vec<DirtyQueueEntry>>,
    }

    impl InMemoryDirtyQueueRepository {
        pub fn new() -> Self {
            Self {
                data: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl DirtyQueueRepository for InMemoryDirtyQueueRepository {
        async fn upsert(&self, entry: &DirtyQueueEntry) -> Result<(), RepositoryError> {
            let mut data = self.data.lock().map_err(|e| {
                RepositoryError::DatabaseError(format!("Mutex poisoned: {e}"))
            })?;

            // Check for existing unresolved entry with same asset/upstream combo
            if let Some(existing) = data
                .iter_mut()
                .find(|e| e.asset_id == entry.asset_id && e.upstream_asset_id == entry.upstream_asset_id && e.resolved_at.is_none())
            {
                // Update existing entry
                existing.upstream_version.clone_from(&entry.upstream_version);
            } else {
                // Insert new entry
                data.push(entry.clone());
            }

            Ok(())
        }

        async fn find_unresolved_by_asset(
            &self,
            asset_id: &AssetId,
        ) -> Result<Vec<DirtyQueueEntry>, RepositoryError> {
            let data = self.data.lock().map_err(|e| {
                RepositoryError::DatabaseError(format!("Mutex poisoned: {e}"))
            })?;

            Ok(data
                .iter()
                .filter(|e| e.asset_id == *asset_id && e.resolved_at.is_none())
                .cloned()
                .collect())
        }

        async fn resolve(&self, entry_id: &uuid::Uuid) -> Result<(), RepositoryError> {
            let mut data = self.data.lock().map_err(|e| {
                RepositoryError::DatabaseError(format!("Mutex poisoned: {e}"))
            })?;

            if let Some(entry) = data.iter_mut().find(|e| e.id == *entry_id) {
                entry.resolved_at = Some(Utc::now());
                Ok(())
            } else {
                Err(RepositoryError::NotFound(format!("{entry_id}")))
            }
        }

        async fn find_all_unresolved(&self) -> Result<Vec<DirtyQueueEntry>, RepositoryError> {
            let data = self.data.lock().map_err(|e| {
                RepositoryError::DatabaseError(format!("Mutex poisoned: {e}"))
            })?;

            Ok(data
                .iter()
                .filter(|e| e.resolved_at.is_none())
                .cloned()
                .collect())
        }
    }
}

use in_memory::{InMemoryAssetRepository, InMemoryDirtyQueueRepository};

#[tokio::test]
async fn memory_repo_creates_asset() {
    let repo = InMemoryAssetRepository::new();
    let org_id = OrganizationId::new();
    let project_id = ProjectId::new();
    let type_id = AssetTypeId::new();

    let cmd = CreateAssetCommand {
        name: "Test Asset".to_string(),
        asset_type_id: type_id,
        project_id: Some(project_id),
        organization_id: org_id,
        level: adam_domain::AssetLevel::Project,
        idempotency_key: None,
    };

    let asset = repo.create(&cmd).await.unwrap();
    assert_eq!(asset.name, "Test Asset");
    assert_eq!(asset.current_state, AssetState::Clean);

    // Verify it can be found
    let found = repo.find_by_id(&asset.id).await.unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().name, "Test Asset");
}

#[tokio::test]
async fn memory_repo_enforces_idempotency() {
    let repo = InMemoryAssetRepository::new();
    let org_id = OrganizationId::new();
    let type_id = AssetTypeId::new();

    let cmd = CreateAssetCommand {
        name: "Test Asset".to_string(),
        asset_type_id: type_id,
        project_id: None,
        organization_id: org_id,
        level: adam_domain::AssetLevel::Organization,
        idempotency_key: Some("git:org1:proj1:abc123".to_string()),
    };

    let asset = repo.create(&cmd).await.unwrap();
    let result = repo.create(&cmd).await;

    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        RepositoryError::DuplicateIdempotencyKey(_)
    ));

    // Verify first asset still exists
    let found = repo.find_by_id(&asset.id).await.unwrap().unwrap();
    assert_eq!(found.name, "Test Asset");
}

#[tokio::test]
async fn memory_repo_update_state() {
    let repo = InMemoryAssetRepository::new();
    let org_id = OrganizationId::new();
    let project_id = ProjectId::new();
    let type_id = AssetTypeId::new();

    let cmd = CreateAssetCommand {
        name: "Test Asset".to_string(),
        asset_type_id: type_id,
        project_id: Some(project_id),
        organization_id: org_id,
        level: adam_domain::AssetLevel::Project,
        idempotency_key: None,
    };

    let asset = repo.create(&cmd).await.unwrap();
    assert_eq!(asset.current_state, AssetState::Clean);

    repo.update_state(&asset.id, AssetState::Dirty).await.unwrap();

    let updated = repo.find_by_id(&asset.id).await.unwrap().unwrap();
    assert_eq!(updated.current_state, AssetState::Dirty);
}

#[tokio::test]
async fn memory_repo_update_state_not_found() {
    let repo = InMemoryAssetRepository::new();
    let fake_id = AssetId::new();

    let result = repo.update_state(&fake_id, AssetState::Dirty).await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), RepositoryError::NotFound(_)));
}

#[tokio::test]
async fn dirty_queue_upsert_inserts_new_entry() {
    let repo = InMemoryDirtyQueueRepository::new();
    let asset_id = AssetId::new();
    let upstream_id = AssetId::new();

    let entry = DirtyQueueEntry {
        id: uuid::Uuid::new_v4(),
        asset_id,
        upstream_asset_id: upstream_id,
        upstream_version: "1.0.0".to_string(),
        created_at: Utc::now(),
        resolved_at: None,
    };

    repo.upsert(&entry).await.unwrap();

    let unresolved = repo.find_unresolved_by_asset(&asset_id).await.unwrap();
    assert_eq!(unresolved.len(), 1);
    assert_eq!(unresolved[0].upstream_version, "1.0.0");
}

#[tokio::test]
async fn dirty_queue_upsert_updates_existing_entry() {
    let repo = InMemoryDirtyQueueRepository::new();
    let asset_id = AssetId::new();
    let upstream_id = AssetId::new();

    let entry1 = DirtyQueueEntry {
        id: uuid::Uuid::new_v4(),
        asset_id,
        upstream_asset_id: upstream_id,
        upstream_version: "1.0.0".to_string(),
        created_at: Utc::now(),
        resolved_at: None,
    };

    repo.upsert(&entry1).await.unwrap();

    let entry2 = DirtyQueueEntry {
        id: uuid::Uuid::new_v4(),
        asset_id,
        upstream_asset_id: upstream_id,
        upstream_version: "2.0.0".to_string(),
        created_at: Utc::now(),
        resolved_at: None,
    };

    repo.upsert(&entry2).await.unwrap();

    let unresolved = repo.find_unresolved_by_asset(&asset_id).await.unwrap();
    assert_eq!(unresolved.len(), 1);
    assert_eq!(unresolved[0].upstream_version, "2.0.0");
}

#[tokio::test]
async fn dirty_queue_resolve_marks_entry_resolved() {
    let repo = InMemoryDirtyQueueRepository::new();
    let asset_id = AssetId::new();
    let upstream_id = AssetId::new();

    let entry = DirtyQueueEntry {
        id: uuid::Uuid::new_v4(),
        asset_id,
        upstream_asset_id: upstream_id,
        upstream_version: "1.0.0".to_string(),
        created_at: Utc::now(),
        resolved_at: None,
    };

    repo.upsert(&entry).await.unwrap();

    // Verify entry exists
    let unresolved_before = repo.find_unresolved_by_asset(&asset_id).await.unwrap();
    assert_eq!(unresolved_before.len(), 1);

    // Resolve the entry
    repo.resolve(&entry.id).await.unwrap();

    // Verify entry is resolved
    let unresolved_after = repo.find_unresolved_by_asset(&asset_id).await.unwrap();
    assert!(unresolved_after.is_empty());

    // Verify it's still in all_unresolved
    let all_unresolved = repo.find_all_unresolved().await.unwrap();
    assert!(all_unresolved.is_empty());
}

#[tokio::test]
async fn dirty_queue_allows_multiple_unresolved_for_different_assets() {
    let repo = InMemoryDirtyQueueRepository::new();
    let asset_id1 = AssetId::new();
    let asset_id2 = AssetId::new();
    let upstream_id = AssetId::new();

    let entry1 = DirtyQueueEntry {
        id: uuid::Uuid::new_v4(),
        asset_id: asset_id1,
        upstream_asset_id: upstream_id,
        upstream_version: "1.0.0".to_string(),
        created_at: Utc::now(),
        resolved_at: None,
    };

    let entry2 = DirtyQueueEntry {
        id: uuid::Uuid::new_v4(),
        asset_id: asset_id2,
        upstream_asset_id: upstream_id,
        upstream_version: "1.0.0".to_string(),
        created_at: Utc::now(),
        resolved_at: None,
    };

    repo.upsert(&entry1).await.unwrap();
    repo.upsert(&entry2).await.unwrap();

    let all_unresolved = repo.find_all_unresolved().await.unwrap();
    assert_eq!(all_unresolved.len(), 2);
}
