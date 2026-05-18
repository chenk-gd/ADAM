//! Asset instance domain model

use crate::asset::state::{AssetState, StateError};
use crate::dependency::boundary::AssetLevel;
use crate::version::SemVer;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for assets
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AssetId(pub Uuid);

impl AssetId {
    /// Generate a new random AssetId
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create an AssetId from an existing UUID
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for AssetId {
    fn default() -> Self {
        Self::new()
    }
}

/// Unique identifier for asset types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AssetTypeId(pub Uuid);

impl AssetTypeId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create an AssetTypeId from an existing UUID
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for AssetTypeId {
    fn default() -> Self {
        Self::new()
    }
}

/// Unique identifier for projects
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectId(pub Uuid);

impl ProjectId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create a ProjectId from an existing UUID
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for ProjectId {
    fn default() -> Self {
        Self::new()
    }
}

/// Unique identifier for organizations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OrganizationId(pub Uuid);

impl OrganizationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create an OrganizationId from an existing UUID
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for OrganizationId {
    fn default() -> Self {
        Self::new()
    }
}

/// Asset instance representing an actual asset in the system
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetInstance {
    pub id: AssetId,
    pub name: String,
    pub asset_type_id: AssetTypeId,
    pub project_id: Option<ProjectId>,
    pub organization_id: OrganizationId,
    pub level: AssetLevel,
    pub(crate) current_state: AssetState,

    // 新增字段 (根据 spec 5.2.3)
    pub external_ref: String,                   // 外部系统引用地址
    pub source: String,                         // 来源：git/wiki/jira/manual
    pub metadata: serde_json::Value,            // 按类型 schema 的元数据
    pub assignees: Vec<String>,                 // 责任人列表
    pub(crate) publisher: Option<String>,       // 最新版本发布人
    pub(crate) current_version: SemVer,         // 当前发布的版本号 (CHANGED: from Option<String>)
    pub(crate) lock_version: i64,               // NEW: for optimistic locking

    pub created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
    pub(crate) idempotency_key: Option<String>,
}

impl AssetInstance {
    /// Create a new project-level asset
    pub fn new_project_level(
        name: impl Into<String>,
        asset_type_id: AssetTypeId,
        project_id: ProjectId,
        organization_id: OrganizationId,
        external_ref: impl Into<String>,
        source: impl Into<String>,
        metadata: serde_json::Value,
        initial_version: SemVer,  // NEW: required SemVer
    ) -> Self {
        let now = Utc::now();
        Self {
            id: AssetId::new(),
            name: name.into(),
            asset_type_id,
            project_id: Some(project_id),
            organization_id,
            level: AssetLevel::Project,
            current_state: AssetState::Clean,
            external_ref: external_ref.into(),
            source: source.into(),
            metadata,
            assignees: vec![],
            publisher: None,
            current_version: initial_version,  // CHANGED
            lock_version: 1,                  // NEW: initialize to 1
            created_at: now,
            updated_at: now,
            idempotency_key: None,
        }
    }

    /// Create a new organization-level asset
    pub fn new_organization_level(
        name: impl Into<String>,
        asset_type_id: AssetTypeId,
        organization_id: OrganizationId,
        external_ref: impl Into<String>,
        source: impl Into<String>,
        metadata: serde_json::Value,
        initial_version: SemVer,  // NEW: required SemVer
    ) -> Self {
        let now = Utc::now();
        Self {
            id: AssetId::new(),
            name: name.into(),
            asset_type_id,
            project_id: None,
            organization_id,
            level: AssetLevel::Organization,
            current_state: AssetState::Clean,
            external_ref: external_ref.into(),
            source: source.into(),
            metadata,
            assignees: vec![],
            publisher: None,
            current_version: initial_version,  // CHANGED
            lock_version: 1,                  // NEW: initialize to 1
            created_at: now,
            updated_at: now,
            idempotency_key: None,
        }
    }

    /// Check if the asset is archived
    pub fn is_archived(&self) -> bool {
        self.current_state.is_archived()
    }

    /// Get the current state
    pub fn state(&self) -> AssetState {
        self.current_state
    }

    /// Get the publisher
    pub fn publisher(&self) -> Option<&String> {
        self.publisher.as_ref()
    }

    /// Get the current version
    pub fn current_version(&self) -> &SemVer {
        &self.current_version
    }

    /// Get the lock version (for optimistic locking)
    pub fn lock_version(&self) -> i64 {
        self.lock_version
    }

    /// Get the updated_at timestamp
    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }

    /// Get the idempotency key
    pub fn idempotency_key(&self) -> Option<&String> {
        self.idempotency_key.as_ref()
    }

    /// Mark the asset as dirty (transition from Clean to Dirty)
    pub fn mark_dirty(&mut self) -> Result<(), StateError> {
        self.current_state.validate_transition(AssetState::Dirty)?;
        self.current_state = AssetState::Dirty;
        self.updated_at = Utc::now();
        Ok(())
    }

    /// Mark the asset as clean (transition from Dirty to Clean)
    pub fn mark_clean(&mut self) -> Result<(), StateError> {
        self.current_state.validate_transition(AssetState::Clean)?;
        self.current_state = AssetState::Clean;
        self.updated_at = Utc::now();
        Ok(())
    }

    /// Archive the asset (transition to Archived)
    pub fn archive(&mut self) -> Result<(), StateError> {
        self.current_state
            .validate_transition(AssetState::Archived)?;
        self.current_state = AssetState::Archived;
        self.updated_at = Utc::now();
        Ok(())
    }

    /// Set the publisher
    pub fn set_publisher(&mut self, publisher: impl Into<String>) {
        self.publisher = Some(publisher.into());
        self.updated_at = Utc::now();
    }

    /// Set the current version
    pub fn set_current_version(&mut self, version: SemVer) {
        self.current_version = version;
        self.updated_at = Utc::now();
    }

    /// Increment lock version (for optimistic locking)
    pub fn increment_lock_version(&mut self) {
        self.lock_version += 1;
    }

    /// Set the idempotency key
    pub fn set_idempotency_key(&mut self, key: impl Into<String>) {
        self.idempotency_key = Some(key.into());
    }

    /// Create a new AssetInstance with all fields (for repository use)
    #[doc(hidden)]
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_fields(
        id: AssetId,
        name: String,
        asset_type_id: AssetTypeId,
        project_id: Option<ProjectId>,
        organization_id: OrganizationId,
        level: AssetLevel,
        current_state: AssetState,
        external_ref: String,
        source: String,
        metadata: serde_json::Value,
        assignees: Vec<String>,
        publisher: Option<String>,
        current_version: SemVer,  // CHANGED
        lock_version: i64,        // NEW
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
        idempotency_key: Option<String>,
    ) -> Self {
        Self {
            id,
            name,
            asset_type_id,
            project_id,
            organization_id,
            level,
            current_state,
            external_ref,
            source,
            metadata,
            assignees,
            publisher,
            current_version,
            lock_version,
            created_at,
            updated_at,
            idempotency_key,
        }
    }

    /// Update state (for repository use - only accessible within domain crate)
    #[doc(hidden)]
    #[allow(dead_code)]
    pub(crate) fn update_state_internal(&mut self, state: AssetState) {
        self.current_state = state;
        self.updated_at = Utc::now();
    }

    /// Update the name
    pub fn update_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
        self.updated_at = Utc::now();
    }

    /// Update the assignees
    pub fn update_assignees(&mut self, assignees: Vec<String>) {
        self.assignees = assignees;
        self.updated_at = Utc::now();
    }

    /// Update the metadata
    pub fn update_metadata(&mut self, metadata: serde_json::Value) {
        self.metadata = metadata;
        self.updated_at = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_asset_id_new_is_unique() {
        let id1 = AssetId::new();
        let id2 = AssetId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_new_project_level_asset() {
        let org_id = OrganizationId::new();
        let project_id = ProjectId::new();
        let type_id = AssetTypeId::new();

        let asset = AssetInstance::new_project_level(
            "Test Asset",
            type_id,
            project_id,
            org_id,
            "https://example.com/asset/1",
            "manual",
            serde_json::json!({"title": "Test"}),
            SemVer::new(1, 0, 0),  // NEW: initial version
        );

        assert_eq!(asset.name, "Test Asset");
        assert_eq!(asset.level, AssetLevel::Project);
        assert_eq!(asset.project_id, Some(project_id));
        assert_eq!(asset.organization_id, org_id);
        assert_eq!(asset.state(), AssetState::Clean);
        assert_eq!(asset.external_ref, "https://example.com/asset/1");
        assert_eq!(asset.source, "manual");
        assert!(asset.assignees.is_empty());
        assert_eq!(asset.current_version, SemVer::new(1, 0, 0));
        assert_eq!(asset.lock_version, 1);
    }

    #[test]
    fn test_new_organization_level_asset() {
        let org_id = OrganizationId::new();
        let type_id = AssetTypeId::new();

        let asset = AssetInstance::new_organization_level(
            "Org Standard",
            type_id,
            org_id,
            "https://wiki.example.com/standards",
            "wiki",
            serde_json::json!({"category": "standard"}),
            SemVer::new(0, 1, 0),  // NEW: initial version
        );

        assert_eq!(asset.name, "Org Standard");
        assert_eq!(asset.level, AssetLevel::Organization);
        assert_eq!(asset.project_id, None);
        assert_eq!(asset.state(), AssetState::Clean);
        assert_eq!(asset.source, "wiki");
        assert_eq!(asset.current_version, SemVer::new(0, 1, 0));
        assert_eq!(asset.lock_version, 1);
    }

    #[test]
    fn test_is_archived() {
        let org_id = OrganizationId::new();
        let type_id = AssetTypeId::new();
        let mut asset = AssetInstance::new_organization_level(
            "Test",
            type_id,
            org_id,
            "https://example.com/asset",
            "manual",
            serde_json::json!({}),
            SemVer::new(1, 0, 0),  // NEW: initial version
        );

        assert!(!asset.is_archived());
        asset.archive().unwrap();
        assert!(asset.is_archived());
    }

    #[test]
    fn test_set_current_version() {
        let org_id = OrganizationId::new();
        let type_id = AssetTypeId::new();
        let mut asset = AssetInstance::new_organization_level(
            "Test",
            type_id,
            org_id,
            "https://example.com/asset",
            "manual",
            serde_json::json!({}),
            SemVer::new(1, 0, 0),
        );

        asset.set_current_version(SemVer::new(2, 0, 0));
        assert_eq!(asset.current_version(), &SemVer::new(2, 0, 0));
    }

    #[test]
    fn test_lock_version_increment() {
        let org_id = OrganizationId::new();
        let type_id = AssetTypeId::new();
        let mut asset = AssetInstance::new_organization_level(
            "Test",
            type_id,
            org_id,
            "https://example.com/asset",
            "manual",
            serde_json::json!({}),
            SemVer::new(1, 0, 0),
        );

        assert_eq!(asset.lock_version(), 1);
        asset.increment_lock_version();
        assert_eq!(asset.lock_version(), 2);
    }
}
