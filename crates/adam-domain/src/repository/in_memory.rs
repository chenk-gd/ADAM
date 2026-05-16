//! In-memory repository implementations for testing

use crate::asset::asset_type::AssetType;
use crate::asset::instance::{AssetId, AssetInstance, AssetTypeId};
use crate::asset::state::AssetState;
use crate::asset::version::{AssetVersion, AssetVersionId, AssetVersionRepository};
use crate::repository::{
    AssetDependencyRecord, AssetRepository, AssetTypeRepository, CreateAssetCommand,
    DependencyRepository, DirtyResolutionLog, DirtyResolutionLogRepository, EffectiveUpdateReason,
    RepositoryError, UpdateAssetCommand,
};
use crate::{
    DirtyQueueEntry, DirtyQueueRepository, OrganizationId, ProjectId, VirtualInstance,
    VirtualInstanceId, VirtualInstanceRepository,
};
use async_trait::async_trait;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Mutex;

/// In-memory asset repository for testing
pub struct InMemoryAssetRepository {
    data: Mutex<HashMap<AssetId, AssetInstance>>,
}

impl InMemoryAssetRepository {
    /// Create a new empty repository
    pub fn new() -> Self {
        Self {
            data: Mutex::new(HashMap::new()),
        }
    }

    /// Create a repository with pre-populated data (for testing)
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
            // Validate state transition
            if !asset.current_state.can_transition_to(state) {
                return Err(RepositoryError::InvalidStateTransition(format!(
                    "Cannot transition from {:?} to {:?}",
                    asset.current_state, state
                )));
            }
            asset.current_state = state;
            asset.updated_at = Utc::now();
            Ok(())
        } else {
            Err(RepositoryError::NotFound(format!("{id:?}")))
        }
    }

    async fn update_publication(
        &self,
        id: &AssetId,
        current_version: String,
        publisher: String,
        state: AssetState,
    ) -> Result<(), RepositoryError> {
        let mut data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;

        if let Some(asset) = data.get_mut(id) {
            if !asset.current_state.can_transition_to(state) {
                return Err(RepositoryError::InvalidStateTransition(format!(
                    "Cannot transition from {:?} to {:?}",
                    asset.current_state, state
                )));
            }
            asset.current_version = Some(current_version);
            asset.publisher = Some(publisher);
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

    async fn update(
        &self,
        id: &AssetId,
        cmd: &UpdateAssetCommand,
    ) -> Result<AssetInstance, RepositoryError> {
        let mut data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;

        if let Some(asset) = data.get_mut(id) {
            if let Some(name) = &cmd.name {
                asset.update_name(name.clone());
            }
            if let Some(assignees) = &cmd.assignees {
                asset.update_assignees(assignees.clone());
            }
            if let Some(metadata) = &cmd.metadata {
                asset.update_metadata(metadata.clone());
            }
            Ok(asset.clone())
        } else {
            Err(RepositoryError::NotFound(format!("{id:?}")))
        }
    }

    async fn delete(&self, id: &AssetId) -> Result<(), RepositoryError> {
        let mut data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;

        if data.remove(id).is_some() {
            Ok(())
        } else {
            Err(RepositoryError::NotFound(format!("{id:?}")))
        }
    }
}

/// In-memory dirty queue repository for testing
pub struct InMemoryDirtyQueueRepository {
    data: Mutex<Vec<DirtyQueueEntry>>,
}

impl InMemoryDirtyQueueRepository {
    /// Create a new empty repository
    pub fn new() -> Self {
        Self {
            data: Mutex::new(Vec::new()),
        }
    }

    /// Create a repository with pre-populated data (for testing)
    pub fn with_data(entries: Vec<DirtyQueueEntry>) -> Self {
        Self {
            data: Mutex::new(entries),
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
            // Update existing entry with new fields
            existing
                .upstream_version
                .clone_from(&entry.upstream_version);
            existing
                .upstream_old_version
                .clone_from(&entry.upstream_old_version);
            existing.impact_level.clone_from(&entry.impact_level);
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

/// In-memory asset type repository for testing
pub struct InMemoryAssetTypeRepository {
    data: Mutex<HashMap<AssetTypeId, AssetType>>,
}

impl InMemoryAssetTypeRepository {
    /// Create a new empty repository
    pub fn new() -> Self {
        Self {
            data: Mutex::new(HashMap::new()),
        }
    }

    /// Create a repository with pre-populated data (for testing)
    pub fn with_data(types: Vec<AssetType>) -> Self {
        let map: HashMap<AssetTypeId, AssetType> = types.into_iter().map(|t| (t.id, t)).collect();
        Self {
            data: Mutex::new(map),
        }
    }
}

impl Default for InMemoryAssetTypeRepository {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AssetTypeRepository for InMemoryAssetTypeRepository {
    async fn create(&self, asset_type: &AssetType) -> Result<AssetType, RepositoryError> {
        let mut data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;
        data.insert(asset_type.id, asset_type.clone());
        Ok(asset_type.clone())
    }

    async fn find_by_id(&self, id: &AssetTypeId) -> Result<Option<AssetType>, RepositoryError> {
        let data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;
        Ok(data.get(id).cloned())
    }

    async fn find_by_name(&self, name: &str) -> Result<Option<AssetType>, RepositoryError> {
        let data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;
        Ok(data.values().find(|at| at.name == name).cloned())
    }

    async fn list_all(&self) -> Result<Vec<AssetType>, RepositoryError> {
        let data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;
        Ok(data.values().cloned().collect())
    }
}

/// In-memory dependency repository for testing
pub struct InMemoryDependencyRepository {
    /// Stores upstream dependencies: asset_id -> list of assets it depends on
    upstream: Mutex<HashMap<AssetId, Vec<AssetId>>>,
    /// Stores downstream dependencies: asset_id -> list of assets that depend on it
    downstream: Mutex<HashMap<AssetId, Vec<AssetId>>>,
    /// Full dependency records keyed by (source, target)
    records: Mutex<HashMap<(AssetId, AssetId), AssetDependencyRecord>>,
}

impl InMemoryDependencyRepository {
    /// Create a new empty repository
    pub fn new() -> Self {
        Self {
            upstream: Mutex::new(HashMap::new()),
            downstream: Mutex::new(HashMap::new()),
            records: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryDependencyRepository {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DependencyRepository for InMemoryDependencyRepository {
    async fn find_downstream(&self, asset_id: &AssetId) -> Result<Vec<AssetId>, RepositoryError> {
        let downstream = self.downstream.lock().map_err(|_| {
            RepositoryError::DatabaseError("Failed to lock downstream map".to_string())
        })?;
        Ok(downstream.get(asset_id).cloned().unwrap_or_default())
    }

    async fn find_upstream(&self, asset_id: &AssetId) -> Result<Vec<AssetId>, RepositoryError> {
        let upstream = self.upstream.lock().map_err(|_| {
            RepositoryError::DatabaseError("Failed to lock upstream map".to_string())
        })?;
        Ok(upstream.get(asset_id).cloned().unwrap_or_default())
    }

    async fn create_dependency(
        &self,
        source_id: &AssetId,
        target_id: &AssetId,
    ) -> Result<(), RepositoryError> {
        let now = Utc::now();
        let record = AssetDependencyRecord {
            id: uuid::Uuid::new_v4(),
            source_id: *source_id,
            target_id: *target_id,
            relationship: "depends_on".to_string(),
            declared_version: "0.0.0".to_string(),
            effective_version: "0.0.0".to_string(),
            effective_updated_by: "system".to_string(),
            effective_updated_at: now,
            effective_reason: EffectiveUpdateReason::Publish,
            created_at: now,
        };
        self.create_dependency_record(&record).await
    }

    async fn create_dependency_record(
        &self,
        record: &AssetDependencyRecord,
    ) -> Result<(), RepositoryError> {
        // source depends on target (source -> target)
        let mut upstream = self.upstream.lock().map_err(|_| {
            RepositoryError::DatabaseError("Failed to lock upstream map".to_string())
        })?;
        let upstreams = upstream.entry(record.source_id).or_default();
        if !upstreams.contains(&record.target_id) {
            upstreams.push(record.target_id);
        }

        let mut downstream = self.downstream.lock().map_err(|_| {
            RepositoryError::DatabaseError("Failed to lock downstream map".to_string())
        })?;
        let downstreams = downstream.entry(record.target_id).or_default();
        if !downstreams.contains(&record.source_id) {
            downstreams.push(record.source_id);
        }

        let mut records = self.records.lock().map_err(|_| {
            RepositoryError::DatabaseError("Failed to lock dependency records map".to_string())
        })?;
        records.insert((record.source_id, record.target_id), record.clone());

        Ok(())
    }

    async fn find_downstream_dependencies(
        &self,
        asset_id: &AssetId,
    ) -> Result<Vec<AssetDependencyRecord>, RepositoryError> {
        let records = self.records.lock().map_err(|_| {
            RepositoryError::DatabaseError("Failed to lock dependency records map".to_string())
        })?;
        Ok(records
            .values()
            .filter(|record| record.target_id == *asset_id)
            .cloned()
            .collect())
    }

    async fn find_upstream_dependencies(
        &self,
        asset_id: &AssetId,
    ) -> Result<Vec<AssetDependencyRecord>, RepositoryError> {
        let records = self.records.lock().map_err(|_| {
            RepositoryError::DatabaseError("Failed to lock dependency records map".to_string())
        })?;
        Ok(records
            .values()
            .filter(|record| record.source_id == *asset_id)
            .cloned()
            .collect())
    }

    async fn update_effective_version(
        &self,
        source_id: &AssetId,
        target_id: &AssetId,
        effective_version: String,
        updated_by: String,
        reason: EffectiveUpdateReason,
    ) -> Result<(), RepositoryError> {
        let mut records = self.records.lock().map_err(|_| {
            RepositoryError::DatabaseError("Failed to lock dependency records map".to_string())
        })?;
        let record = records
            .get_mut(&(*source_id, *target_id))
            .ok_or_else(|| RepositoryError::NotFound(format!("{source_id:?}->{target_id:?}")))?;
        record.effective_version = effective_version;
        record.effective_updated_by = updated_by;
        record.effective_updated_at = Utc::now();
        record.effective_reason = reason;
        Ok(())
    }
}

/// In-memory dirty resolution log repository for testing
pub struct InMemoryDirtyResolutionLogRepository {
    data: Mutex<Vec<DirtyResolutionLog>>,
}

impl InMemoryDirtyResolutionLogRepository {
    /// Create a new empty repository
    pub fn new() -> Self {
        Self {
            data: Mutex::new(Vec::new()),
        }
    }
}

impl Default for InMemoryDirtyResolutionLogRepository {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DirtyResolutionLogRepository for InMemoryDirtyResolutionLogRepository {
    async fn insert(&self, log: &DirtyResolutionLog) -> Result<(), RepositoryError> {
        let mut data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;
        data.push(log.clone());
        Ok(())
    }

    async fn find_by_asset(
        &self,
        asset_id: &AssetId,
    ) -> Result<Vec<DirtyResolutionLog>, RepositoryError> {
        let data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;
        Ok(data
            .iter()
            .filter(|log| log.asset_id == *asset_id)
            .cloned()
            .collect())
    }
}

/// In-memory asset version repository for testing
pub struct InMemoryAssetVersionRepository {
    data: Mutex<HashMap<AssetVersionId, AssetVersion>>,
}

impl InMemoryAssetVersionRepository {
    /// Create a new empty repository
    pub fn new() -> Self {
        Self {
            data: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryAssetVersionRepository {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AssetVersionRepository for InMemoryAssetVersionRepository {
    async fn create(&self, version: &AssetVersion) -> Result<(), RepositoryError> {
        let mut data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;
        data.insert(version.id, version.clone());
        Ok(())
    }

    async fn find_by_asset(
        &self,
        asset_id: &AssetId,
    ) -> Result<Vec<AssetVersion>, RepositoryError> {
        let data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;
        Ok(data
            .values()
            .filter(|v| v.asset_id == *asset_id)
            .cloned()
            .collect())
    }

    async fn find_by_version(
        &self,
        asset_id: &AssetId,
        version: &str,
    ) -> Result<Option<AssetVersion>, RepositoryError> {
        let data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;
        Ok(data
            .values()
            .find(|v| v.asset_id == *asset_id && v.version_number == version)
            .cloned())
    }

    async fn find_latest(
        &self,
        asset_id: &AssetId,
    ) -> Result<Option<AssetVersion>, RepositoryError> {
        let data = self
            .data
            .lock()
            .map_err(|e| RepositoryError::DatabaseError(format!("Mutex poisoned: {e}")))?;
        Ok(data
            .values()
            .filter(|v| v.asset_id == *asset_id)
            .max_by_key(|v| &v.released_at)
            .cloned())
    }
}
