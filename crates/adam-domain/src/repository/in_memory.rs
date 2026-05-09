//! In-memory repository implementations for testing

use crate::{
    AssetId, AssetInstance, AssetRepository, AssetState, CreateAssetCommand, DirtyQueueEntry,
    DirtyQueueRepository, OrganizationId, ProjectId, RepositoryError, VirtualInstance,
    VirtualInstanceId, VirtualInstanceRepository,
};
use async_trait::async_trait;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Mutex;

/// In-memory asset repository for testing
pub struct InMemoryAssetRepository {
    pub data: Mutex<HashMap<AssetId, AssetInstance>>,
}

impl InMemoryAssetRepository {
    /// Create a new empty repository
    pub fn new() -> Self {
        Self {
            data: Mutex::new(HashMap::new()),
        }
    }

    /// Create a repository with pre-populated data
    pub fn with_data(assets: Vec<AssetInstance>) -> Self {
        let map: HashMap<AssetId, AssetInstance> = assets.into_iter().map(|a| (a.id, a)).collect();
        Self {
            data: Mutex::new(map),
        }
    }
}

impl Default for InMemoryAssetRepository {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AssetRepository for InMemoryAssetRepository {
    async fn create(&self, cmd: &CreateAssetCommand) -> Result<AssetInstance, RepositoryError> {
        let mut data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;

        // Check idempotency key
        if let Some(ref key) = cmd.idempotency_key {
            if data
                .values()
                .any(|a| a.idempotency_key.as_ref() == Some(key))
            {
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
            external_ref: cmd.external_ref.clone(),
            source: cmd.source.clone(),
            metadata: cmd.metadata.clone(),
            assignees: vec![],
            publisher: None,
            current_version: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            idempotency_key: cmd.idempotency_key.clone(),
        };

        data.insert(asset.id, asset.clone());
        Ok(asset)
    }

    async fn find_by_id(&self, id: &AssetId) -> Result<Option<AssetInstance>, RepositoryError> {
        let data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;
        Ok(data.get(id).cloned())
    }

    async fn update_state(&self, id: &AssetId, state: AssetState) -> Result<(), RepositoryError> {
        let mut data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;

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
        let data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;

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
        let data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;

        Ok(data
            .values()
            .filter(|a| a.organization_id == *org_id)
            .cloned()
            .collect())
    }
}

/// In-memory dirty queue repository for testing
pub struct InMemoryDirtyQueueRepository {
    pub data: Mutex<Vec<DirtyQueueEntry>>,
}

impl InMemoryDirtyQueueRepository {
    /// Create a new empty repository
    pub fn new() -> Self {
        Self {
            data: Mutex::new(Vec::new()),
        }
    }
}

impl Default for InMemoryDirtyQueueRepository {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DirtyQueueRepository for InMemoryDirtyQueueRepository {
    async fn upsert(&self, entry: &DirtyQueueEntry) -> Result<(), RepositoryError> {
        let mut data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;

        // Check for existing unresolved entry with same asset/upstream combo
        if let Some(existing) = data.iter_mut().find(|e| {
            e.asset_id == entry.asset_id
                && e.upstream_asset_id == entry.upstream_asset_id
                && e.resolved_at.is_none()
        }) {
            // Update existing entry
            existing
                .upstream_version
                .clone_from(&entry.upstream_version);
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
        let data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;

        Ok(data
            .iter()
            .filter(|e| e.asset_id == *asset_id && e.resolved_at.is_none())
            .cloned()
            .collect())
    }

    async fn resolve(&self, entry_id: &uuid::Uuid) -> Result<(), RepositoryError> {
        let mut data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;

        if let Some(entry) = data.iter_mut().find(|e| e.id == *entry_id) {
            entry.resolved_at = Some(Utc::now());
            Ok(())
        } else {
            Err(RepositoryError::NotFound(format!("{entry_id}")))
        }
    }

    async fn find_all_unresolved(&self) -> Result<Vec<DirtyQueueEntry>, RepositoryError> {
        let data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;

        Ok(data
            .iter()
            .filter(|e| e.resolved_at.is_none())
            .cloned()
            .collect())
    }
}

/// In-memory virtual instance repository for testing
pub struct InMemoryVirtualInstanceRepository {
    data: Mutex<HashMap<VirtualInstanceId, VirtualInstance>>,
}

impl InMemoryVirtualInstanceRepository {
    /// Create a new empty repository
    pub fn new() -> Self {
        Self {
            data: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryVirtualInstanceRepository {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl VirtualInstanceRepository for InMemoryVirtualInstanceRepository {
    async fn find_by_id(
        &self,
        id: &VirtualInstanceId,
    ) -> Result<Option<VirtualInstance>, RepositoryError> {
        let data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;
        Ok(data.get(id).cloned())
    }

    async fn create(&self, instance: &VirtualInstance) -> Result<VirtualInstance, RepositoryError> {
        let mut data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;
        data.insert(instance.id, instance.clone());
        Ok(instance.clone())
    }

    async fn delete_expired(&self) -> Result<u64, RepositoryError> {
        let mut data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;
        let now = Utc::now();
        let expired_keys: Vec<VirtualInstanceId> = data
            .iter()
            .filter(|(_, v)| v.expires_at < now)
            .map(|(k, _)| *k)
            .collect();
        let count = expired_keys.len();
        for key in expired_keys {
            data.remove(&key);
        }
        Ok(count as u64)
    }

    async fn find_by_project(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<VirtualInstance>, RepositoryError> {
        let data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;
        Ok(data
            .values()
            .filter(|v| v.project_id == *project_id)
            .cloned()
            .collect())
    }
}
