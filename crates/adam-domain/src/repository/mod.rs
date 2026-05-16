//! Repository traits for asset management

pub mod in_memory;

use crate::asset::asset_type::AssetType;
use crate::asset::instance::AssetTypeId;
use crate::asset::instance::{AssetId, AssetInstance};
use crate::asset::state::AssetState;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;

/// Errors that can occur in repository operations
#[derive(Debug, Error)]
pub enum RepositoryError {
    /// Asset not found
    #[error("Asset not found: {0}")]
    NotFound(String),
    /// Duplicate idempotency key
    #[error("Duplicate idempotency key: {0}")]
    DuplicateIdempotencyKey(String),
    /// Database error
    #[error("Database error: {0}")]
    DatabaseError(String),
    /// Invalid state transition
    #[error("Invalid state transition: {0}")]
    InvalidStateTransition(String),
}

/// Command for creating a new asset
#[derive(Debug, Clone)]
pub struct CreateAssetCommand {
    pub name: String,
    pub asset_type_id: crate::asset::instance::AssetTypeId,
    pub project_id: Option<crate::asset::instance::ProjectId>,
    pub organization_id: crate::asset::instance::OrganizationId,
    pub level: crate::dependency::boundary::AssetLevel,
    pub external_ref: String,
    pub source: String,
    pub metadata: serde_json::Value,
    pub idempotency_key: Option<String>,
}

/// Command for updating an asset
#[derive(Debug, Clone)]
pub struct UpdateAssetCommand {
    pub name: Option<String>,
    pub assignees: Option<Vec<String>>,
    pub metadata: Option<serde_json::Value>,
}

/// Repository trait for asset instances
#[async_trait]
pub trait AssetRepository: Send + Sync {
    /// Create a new asset
    async fn create(&self, cmd: &CreateAssetCommand) -> Result<AssetInstance, RepositoryError>;

    /// Find asset by ID
    async fn find_by_id(&self, id: &AssetId) -> Result<Option<AssetInstance>, RepositoryError>;

    /// Update asset
    async fn update(
        &self,
        id: &AssetId,
        cmd: &UpdateAssetCommand,
    ) -> Result<AssetInstance, RepositoryError>;

    /// Update asset state
    async fn update_state(&self, id: &AssetId, state: AssetState) -> Result<(), RepositoryError>;

    /// Update fields changed by a successful publish operation
    async fn update_publication(
        &self,
        id: &AssetId,
        current_version: String,
        publisher: String,
        state: AssetState,
    ) -> Result<(), RepositoryError>;

    /// Find assets by project ID
    async fn find_by_project_id(
        &self,
        project_id: &crate::asset::instance::ProjectId,
    ) -> Result<Vec<AssetInstance>, RepositoryError>;

    /// Find assets by organization ID
    async fn find_by_organization_id(
        &self,
        org_id: &crate::asset::instance::OrganizationId,
    ) -> Result<Vec<AssetInstance>, RepositoryError>;

    /// Delete asset by ID
    async fn delete(&self, id: &AssetId) -> Result<(), RepositoryError>;
}

#[async_trait]
impl<T: AssetRepository + ?Sized> AssetRepository for Arc<T> {
    async fn create(&self, cmd: &CreateAssetCommand) -> Result<AssetInstance, RepositoryError> {
        self.as_ref().create(cmd).await
    }

    async fn find_by_id(&self, id: &AssetId) -> Result<Option<AssetInstance>, RepositoryError> {
        self.as_ref().find_by_id(id).await
    }

    async fn update(
        &self,
        id: &AssetId,
        cmd: &UpdateAssetCommand,
    ) -> Result<AssetInstance, RepositoryError> {
        self.as_ref().update(id, cmd).await
    }

    async fn update_state(&self, id: &AssetId, state: AssetState) -> Result<(), RepositoryError> {
        self.as_ref().update_state(id, state).await
    }

    async fn update_publication(
        &self,
        id: &AssetId,
        current_version: String,
        publisher: String,
        state: AssetState,
    ) -> Result<(), RepositoryError> {
        self.as_ref()
            .update_publication(id, current_version, publisher, state)
            .await
    }

    async fn find_by_project_id(
        &self,
        project_id: &crate::asset::instance::ProjectId,
    ) -> Result<Vec<AssetInstance>, RepositoryError> {
        self.as_ref().find_by_project_id(project_id).await
    }

    async fn find_by_organization_id(
        &self,
        org_id: &crate::asset::instance::OrganizationId,
    ) -> Result<Vec<AssetInstance>, RepositoryError> {
        self.as_ref().find_by_organization_id(org_id).await
    }

    async fn delete(&self, id: &AssetId) -> Result<(), RepositoryError> {
        self.as_ref().delete(id).await
    }
}

/// Reason why the current effective dependency baseline was updated
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectiveUpdateReason {
    /// Updated by a publish operation
    Publish,
    /// Updated by manual clean review
    ManualClean,
}

impl EffectiveUpdateReason {
    /// Stable storage representation
    pub fn as_str(self) -> &'static str {
        match self {
            EffectiveUpdateReason::Publish => "publish",
            EffectiveUpdateReason::ManualClean => "manual_clean",
        }
    }
}

/// Full dependency record with declared and effective versions
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetDependencyRecord {
    pub id: uuid::Uuid,
    pub source_id: AssetId,
    pub target_id: AssetId,
    pub relationship: String,
    pub declared_version: String,
    pub effective_version: String,
    pub effective_updated_by: String,
    pub effective_updated_at: chrono::DateTime<chrono::Utc>,
    pub effective_reason: EffectiveUpdateReason,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Entry in the dirty queue for tracking upstream changes
#[derive(Debug, Clone)]
pub struct DirtyQueueEntry {
    pub id: uuid::Uuid,
    pub asset_id: AssetId,
    pub upstream_asset_id: AssetId,
    pub upstream_version: String,
    /// Previous version of upstream asset before this change
    pub upstream_old_version: String,
    /// Impact level: "low", "medium", "high", "critical"
    pub impact_level: String,
    /// When the dirty state was triggered
    pub since: chrono::DateTime<chrono::Utc>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub resolved_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Repository trait for dirty queue operations
#[async_trait]
pub trait DirtyQueueRepository: Send + Sync {
    /// Insert or update a dirty queue entry
    async fn upsert(&self, entry: &DirtyQueueEntry) -> Result<(), RepositoryError>;

    /// Find unresolved entries by asset ID
    async fn find_unresolved_by_asset(
        &self,
        asset_id: &AssetId,
    ) -> Result<Vec<DirtyQueueEntry>, RepositoryError>;

    /// Mark an entry as resolved
    async fn resolve(&self, entry_id: &uuid::Uuid) -> Result<(), RepositoryError>;

    /// Find all unresolved entries
    async fn find_all_unresolved(&self) -> Result<Vec<DirtyQueueEntry>, RepositoryError>;
}

#[async_trait]
impl<T: DirtyQueueRepository + ?Sized> DirtyQueueRepository for Arc<T> {
    async fn upsert(&self, entry: &DirtyQueueEntry) -> Result<(), RepositoryError> {
        self.as_ref().upsert(entry).await
    }

    async fn find_unresolved_by_asset(
        &self,
        asset_id: &AssetId,
    ) -> Result<Vec<DirtyQueueEntry>, RepositoryError> {
        self.as_ref().find_unresolved_by_asset(asset_id).await
    }

    async fn resolve(&self, entry_id: &uuid::Uuid) -> Result<(), RepositoryError> {
        self.as_ref().resolve(entry_id).await
    }

    async fn find_all_unresolved(&self) -> Result<Vec<DirtyQueueEntry>, RepositoryError> {
        self.as_ref().find_all_unresolved().await
    }
}

/// Repository trait for dependency operations
#[async_trait]
pub trait DependencyRepository: Send + Sync {
    /// Find all downstream assets (assets that depend on the given asset)
    async fn find_downstream(&self, asset_id: &AssetId) -> Result<Vec<AssetId>, RepositoryError>;

    /// Find all upstream assets (assets that the given asset depends on)
    async fn find_upstream(&self, asset_id: &AssetId) -> Result<Vec<AssetId>, RepositoryError>;

    /// Create a dependency relationship
    async fn create_dependency(
        &self,
        source_id: &AssetId,
        target_id: &AssetId,
    ) -> Result<(), RepositoryError>;

    /// Create or replace a rich dependency record
    async fn create_dependency_record(
        &self,
        _record: &AssetDependencyRecord,
    ) -> Result<(), RepositoryError> {
        Err(RepositoryError::DatabaseError(
            "create_dependency_record is not implemented".to_string(),
        ))
    }

    /// Find rich downstream dependency records where `asset_id` is the upstream target
    async fn find_downstream_dependencies(
        &self,
        _asset_id: &AssetId,
    ) -> Result<Vec<AssetDependencyRecord>, RepositoryError> {
        Err(RepositoryError::DatabaseError(
            "find_downstream_dependencies is not implemented".to_string(),
        ))
    }

    /// Find rich upstream dependency records where `asset_id` is the downstream source
    async fn find_upstream_dependencies(
        &self,
        _asset_id: &AssetId,
    ) -> Result<Vec<AssetDependencyRecord>, RepositoryError> {
        Err(RepositoryError::DatabaseError(
            "find_upstream_dependencies is not implemented".to_string(),
        ))
    }

    /// Update only the current effective baseline for an existing dependency
    async fn update_effective_version(
        &self,
        _source_id: &AssetId,
        _target_id: &AssetId,
        _effective_version: String,
        _updated_by: String,
        _reason: EffectiveUpdateReason,
    ) -> Result<(), RepositoryError> {
        Err(RepositoryError::DatabaseError(
            "update_effective_version is not implemented".to_string(),
        ))
    }
}

#[async_trait]
impl<T: DependencyRepository + ?Sized> DependencyRepository for Arc<T> {
    async fn find_downstream(&self, asset_id: &AssetId) -> Result<Vec<AssetId>, RepositoryError> {
        self.as_ref().find_downstream(asset_id).await
    }

    async fn find_upstream(&self, asset_id: &AssetId) -> Result<Vec<AssetId>, RepositoryError> {
        self.as_ref().find_upstream(asset_id).await
    }

    async fn create_dependency(
        &self,
        source_id: &AssetId,
        target_id: &AssetId,
    ) -> Result<(), RepositoryError> {
        self.as_ref().create_dependency(source_id, target_id).await
    }

    async fn create_dependency_record(
        &self,
        record: &AssetDependencyRecord,
    ) -> Result<(), RepositoryError> {
        self.as_ref().create_dependency_record(record).await
    }

    async fn find_downstream_dependencies(
        &self,
        asset_id: &AssetId,
    ) -> Result<Vec<AssetDependencyRecord>, RepositoryError> {
        self.as_ref().find_downstream_dependencies(asset_id).await
    }

    async fn find_upstream_dependencies(
        &self,
        asset_id: &AssetId,
    ) -> Result<Vec<AssetDependencyRecord>, RepositoryError> {
        self.as_ref().find_upstream_dependencies(asset_id).await
    }

    async fn update_effective_version(
        &self,
        source_id: &AssetId,
        target_id: &AssetId,
        effective_version: String,
        updated_by: String,
        reason: EffectiveUpdateReason,
    ) -> Result<(), RepositoryError> {
        self.as_ref()
            .update_effective_version(source_id, target_id, effective_version, updated_by, reason)
            .await
    }
}

/// Dirty resolution audit log
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirtyResolutionLog {
    pub id: uuid::Uuid,
    pub asset_id: AssetId,
    pub asset_version: String,
    pub upstream_asset_id: AssetId,
    pub from_version: String,
    pub to_version: String,
    pub action: String,
    pub review_result: String,
    pub comment: Option<String>,
    pub reviewed_by: String,
    pub reviewed_at: chrono::DateTime<chrono::Utc>,
}

/// Repository trait for dirty resolution audit logs
#[async_trait]
pub trait DirtyResolutionLogRepository: Send + Sync {
    /// Insert a dirty resolution log
    async fn insert(&self, log: &DirtyResolutionLog) -> Result<(), RepositoryError>;

    /// Find logs by asset ID
    async fn find_by_asset(
        &self,
        asset_id: &AssetId,
    ) -> Result<Vec<DirtyResolutionLog>, RepositoryError>;
}

#[async_trait]
impl<T: DirtyResolutionLogRepository + ?Sized> DirtyResolutionLogRepository for Arc<T> {
    async fn insert(&self, log: &DirtyResolutionLog) -> Result<(), RepositoryError> {
        self.as_ref().insert(log).await
    }

    async fn find_by_asset(
        &self,
        asset_id: &AssetId,
    ) -> Result<Vec<DirtyResolutionLog>, RepositoryError> {
        self.as_ref().find_by_asset(asset_id).await
    }
}

/// Repository trait for asset types
#[async_trait]
pub trait AssetTypeRepository: Send + Sync {
    /// Create a new asset type
    async fn create(&self, asset_type: &AssetType) -> Result<AssetType, RepositoryError>;

    /// Find by ID
    async fn find_by_id(&self, id: &AssetTypeId) -> Result<Option<AssetType>, RepositoryError>;

    /// Find by name
    async fn find_by_name(&self, name: &str) -> Result<Option<AssetType>, RepositoryError>;

    /// List all asset types
    async fn list_all(&self) -> Result<Vec<AssetType>, RepositoryError>;
}
