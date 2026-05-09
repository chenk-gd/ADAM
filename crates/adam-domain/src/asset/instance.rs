//! Asset instance domain model

use crate::asset::state::AssetState;
use crate::dependency::boundary::AssetLevel;
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
    pub current_state: AssetState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub idempotency_key: Option<String>,
}

impl AssetInstance {
    /// Create a new project-level asset
    pub fn new_project_level(
        name: impl Into<String>,
        asset_type_id: AssetTypeId,
        project_id: ProjectId,
        organization_id: OrganizationId,
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
            created_at: now,
            updated_at: now,
            idempotency_key: None,
        }
    }

    /// Check if the asset is archived
    pub fn is_archived(&self) -> bool {
        self.current_state.is_archived()
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

        let asset = AssetInstance::new_project_level("Test Asset", type_id, project_id, org_id);

        assert_eq!(asset.name, "Test Asset");
        assert_eq!(asset.level, AssetLevel::Project);
        assert_eq!(asset.project_id, Some(project_id));
        assert_eq!(asset.organization_id, org_id);
        assert_eq!(asset.current_state, AssetState::Clean);
    }

    #[test]
    fn test_new_organization_level_asset() {
        let org_id = OrganizationId::new();
        let type_id = AssetTypeId::new();

        let asset = AssetInstance::new_organization_level("Org Standard", type_id, org_id);

        assert_eq!(asset.name, "Org Standard");
        assert_eq!(asset.level, AssetLevel::Organization);
        assert_eq!(asset.project_id, None);
        assert_eq!(asset.current_state, AssetState::Clean);
    }

    #[test]
    fn test_is_archived() {
        let org_id = OrganizationId::new();
        let type_id = AssetTypeId::new();
        let mut asset = AssetInstance::new_organization_level("Test", type_id, org_id);

        assert!(!asset.is_archived());
        asset.current_state = AssetState::Archived;
        assert!(asset.is_archived());
    }
}
