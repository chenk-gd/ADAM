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
    /// Validation error
    #[error("Validation error: {0}")]
    ValidationError(String),
    /// Concurrent modification - lock version mismatch
    #[error("Concurrent modification: expected lock version {expected}, actual {actual}")]
    ConcurrentModification { expected: i64, actual: i64 },
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

    /// Update fields changed by a successful publish operation with CAS (Compare-And-Swap) optimistic locking
    ///
    /// # CAS Semantics
    /// This method performs an atomic update only if the asset's lock_version matches
    /// the expected value. This prevents lost updates in concurrent scenarios.
    ///
    /// # Arguments
    /// * `id` - Asset ID to update
    /// * `current_version` - New current version string
    /// * `publisher` - Who published the version
    /// * `state` - New asset state
    /// * `expected_lock_version` - Expected lock_version for CAS check
    ///
    /// # Returns
    /// * `Ok(new_lock_version)` - Update successful, returns the new lock_version
    /// * `Err(RepositoryError::ConcurrentModification)` - CAS check failed, asset was modified
    async fn update_publication_cas(
        &self,
        id: &AssetId,
        current_version: String,
        publisher: String,
        state: AssetState,
        expected_lock_version: i64,
    ) -> Result<i64, RepositoryError>;

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

    async fn update_publication_cas(
        &self,
        id: &AssetId,
        current_version: String,
        publisher: String,
        state: AssetState,
        expected_lock_version: i64,
    ) -> Result<i64, RepositoryError> {
        self.as_ref()
            .update_publication_cas(id, current_version, publisher, state, expected_lock_version)
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

/// Upgrade policy for dependency updates
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpgradePolicy {
    /// Automatically update patch versions
    AutoPatch,
    /// Automatically update minor versions
    AutoMinor,
    /// Notify but don't auto-update
    Notify,
    /// Manual review required
    Manual,
    /// Pin to specific version
    Pin,
}

impl Default for UpgradePolicy {
    fn default() -> Self {
        UpgradePolicy::Notify
    }
}

impl UpgradePolicy {
    /// Stable storage representation
    pub fn as_str(self) -> &'static str {
        match self {
            UpgradePolicy::AutoPatch => "auto_patch",
            UpgradePolicy::AutoMinor => "auto_minor",
            UpgradePolicy::Notify => "notify",
            UpgradePolicy::Manual => "manual",
            UpgradePolicy::Pin => "pin",
        }
    }
}

/// Full dependency record with declared constraint and effective version
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetDependencyRecord {
    pub id: uuid::Uuid,
    pub source_id: AssetId,
    pub target_id: AssetId,
    pub relationship: String,
    pub declared_constraint: crate::version::VersionConstraint,  // CHANGED: from String
    pub constraint_str: String,                                   // NEW: for DB storage
    pub effective_version: crate::version::SemVer,              // CHANGED: from String
    pub effective_updated_by: String,
    pub effective_updated_at: chrono::DateTime<chrono::Utc>,
    pub effective_reason: EffectiveUpdateReason,
    pub upgrade_policy: UpgradePolicy,                          // NEW
    pub lock_version: i64,                                      // NEW: for optimistic locking
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

/// Transaction trait for atomic operations
///
/// Implementations should provide database-level transaction support
/// to ensure ACID properties across multiple repository operations.
#[async_trait]
pub trait Transaction: Send + Sync {
    /// Commit the transaction
    async fn commit(self) -> Result<(), RepositoryError>;

    /// Rollback the transaction
    async fn rollback(self) -> Result<(), RepositoryError>;
}

/// Unit of work for transactional repository operations
///
/// This trait allows repositories to participate in transactions
/// by providing methods that execute within a transaction context.
#[async_trait]
pub trait UnitOfWork: Send + Sync {
    /// Execute a closure within a transaction
    ///
    /// The closure receives a transactional context that can be used
    /// to perform repository operations atomically.
    ///
    /// # Type Parameters
    /// * `F` - The closure type that performs transactional operations
    /// * `T` - The return type of the closure
    /// * `E` - The error type (must be convertible from RepositoryError)
    ///
    /// # Arguments
    /// * `operation` - Closure that receives transaction context and returns Result
    ///
    /// # Returns
    /// * `Ok(T)` - Transaction committed successfully, returns closure result
    /// * `Err(E)` - Transaction rolled back, returns closure error
    async fn transaction<F, T, E>(&self, operation: F) -> Result<T, E>
    where
        F: for<'a> FnOnce(&'a mut TransactionContext)
                -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, E>> + Send + 'a>>
            + Send
            + 'async_trait,
        E: From<RepositoryError> + Send,
        T: Send;
}

/// Transaction context for repository operations within a transaction
///
/// This struct holds the transaction handle and provides access to
/// transactional versions of repositories.
pub struct TransactionContext {
    /// Transaction-specific asset repository
    pub asset_repo: Box<dyn AssetRepository>,
    /// Transaction-specific dependency repository
    pub dependency_repo: Box<dyn DependencyRepository>,
    /// Transaction-specific dirty queue repository
    pub dirty_queue_repo: Box<dyn DirtyQueueRepository>,
}

/// Builder for transaction context
///
/// Allows flexible construction of transaction contexts with
/// different repository combinations.
pub struct TransactionContextBuilder {
    asset_repo: Option<Box<dyn AssetRepository>>,
    dependency_repo: Option<Box<dyn DependencyRepository>>,
    dirty_queue_repo: Option<Box<dyn DirtyQueueRepository>>,
}

impl TransactionContextBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            asset_repo: None,
            dependency_repo: None,
            dirty_queue_repo: None,
        }
    }

    /// Set the asset repository
    pub fn with_asset_repo(mut self, repo: Box<dyn AssetRepository>) -> Self {
        self.asset_repo = Some(repo);
        self
    }

    /// Set the dependency repository
    pub fn with_dependency_repo(mut self, repo: Box<dyn DependencyRepository>) -> Self {
        self.dependency_repo = Some(repo);
        self
    }

    /// Set the dirty queue repository
    pub fn with_dirty_queue_repo(mut self, repo: Box<dyn DirtyQueueRepository>) -> Self {
        self.dirty_queue_repo = Some(repo);
        self
    }

    /// Build the transaction context
    ///
    /// # Panics
    /// Panics if required repositories are not set
    pub fn build(self) -> TransactionContext {
        TransactionContext {
            asset_repo: self.asset_repo.expect("Asset repository required"),
            dependency_repo: self.dependency_repo.expect("Dependency repository required"),
            dirty_queue_repo: self.dirty_queue_repo.expect("Dirty queue repository required"),
        }
    }
}

impl Default for TransactionContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}
