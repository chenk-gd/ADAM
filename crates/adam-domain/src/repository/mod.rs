//! Repository traits for asset management

pub mod in_memory;

use crate::asset::asset_type::AssetType;
use crate::asset::instance::AssetTypeId;
use crate::asset::instance::{AssetId, AssetInstance};
use crate::asset::state::AssetState;
use async_trait::async_trait;
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

/// Repository trait for asset instances
#[async_trait]
pub trait AssetRepository: Send + Sync {
    /// Create a new asset
    async fn create(&self, cmd: &CreateAssetCommand) -> Result<AssetInstance, RepositoryError>;

    /// Find asset by ID
    async fn find_by_id(&self, id: &AssetId) -> Result<Option<AssetInstance>, RepositoryError>;

    /// Update asset state
    async fn update_state(&self, id: &AssetId, state: AssetState) -> Result<(), RepositoryError>;

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
}

/// Entry in the dirty queue for tracking upstream changes
#[derive(Debug, Clone)]
pub struct DirtyQueueEntry {
    pub id: uuid::Uuid,
    pub asset_id: AssetId,
    pub upstream_asset_id: AssetId,
    pub upstream_version: String,
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
