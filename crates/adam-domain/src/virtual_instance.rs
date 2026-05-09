//! Virtual asset instance for AI context queries

use crate::{AssetId, AssetTypeId, OrganizationId, ProjectId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Virtual instance ID
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VirtualInstanceId(pub Uuid);

impl std::fmt::Display for VirtualInstanceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl VirtualInstanceId {
    /// Create new random virtual instance ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create from existing UUID
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for VirtualInstanceId {
    fn default() -> Self {
        Self::new()
    }
}

/// Virtual asset instance - temporary context for AI queries
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualInstance {
    /// Unique virtual instance ID
    pub id: VirtualInstanceId,
    /// Target asset type for context generation
    pub target_type: AssetTypeId,
    /// Target asset type name for display
    pub target_type_name: String,
    /// Anchor asset IDs for context
    pub anchors: Vec<AssetId>,
    /// Project context
    pub project_id: ProjectId,
    /// Organization context
    pub organization_id: OrganizationId,
    /// Creator principal ID
    pub created_by: String,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Expiration timestamp (typically 1 hour from creation)
    pub expires_at: DateTime<Utc>,
    /// Context summary for display
    pub context_summary: String,
}

impl VirtualInstance {
    /// Create a new virtual instance
    pub fn new(
        target_type: AssetTypeId,
        target_type_name: impl Into<String>,
        anchors: Vec<AssetId>,
        project_id: ProjectId,
        organization_id: OrganizationId,
        created_by: String,
    ) -> Self {
        let now = Utc::now();
        let expires_at = now + chrono::Duration::hours(1);
        let target_type_name = target_type_name.into();

        let context_summary = format!(
            "Virtual {} asset with {} anchor(s) in project {}",
            target_type_name,
            anchors.len(),
            project_id.0
        );

        Self {
            id: VirtualInstanceId::new(),
            target_type,
            target_type_name,
            anchors,
            project_id,
            organization_id,
            created_by,
            created_at: now,
            expires_at,
            context_summary,
        }
    }

    /// Check if the virtual instance has expired
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    /// Check if principal is the creator
    pub fn is_creator(&self, principal_id: &str) -> bool {
        self.created_by == principal_id
    }
}

/// Repository for virtual instances
#[async_trait::async_trait]
pub trait VirtualInstanceRepository: Send + Sync {
    /// Find virtual instance by ID
    async fn find_by_id(
        &self,
        id: &VirtualInstanceId,
    ) -> Result<Option<VirtualInstance>, crate::RepositoryError>;

    /// Create new virtual instance
    async fn create(
        &self,
        instance: &VirtualInstance,
    ) -> Result<VirtualInstance, crate::RepositoryError>;

    /// Delete expired virtual instances
    async fn delete_expired(&self) -> Result<u64, crate::RepositoryError>;

    /// List virtual instances by project
    async fn find_by_project(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<VirtualInstance>, crate::RepositoryError>;
}

/// Error types for virtual instance operations
#[derive(Debug, thiserror::Error)]
pub enum VirtualInstanceError {
    #[error("Virtual instance not found: {0}")]
    NotFound(VirtualInstanceId),
    #[error("Virtual instance expired: {0}")]
    Expired(VirtualInstanceId),
    #[error("Access denied to virtual instance: {0}")]
    AccessDenied(VirtualInstanceId),
    #[error("Repository error: {0}")]
    Repository(#[from] crate::RepositoryError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_instance_creation() {
        let instance = VirtualInstance::new(
            AssetTypeId::new(),
            "code_commit",
            vec![AssetId::new()],
            ProjectId::new(),
            OrganizationId::new(),
            "test-user".to_string(),
        );

        assert!(!instance.context_summary.is_empty());
        assert!(!instance.is_expired());
        assert!(instance.is_creator("test-user"));
        assert!(!instance.is_creator("other-user"));
    }

    #[test]
    fn virtual_instance_expiration() {
        let instance = VirtualInstance {
            id: VirtualInstanceId::new(),
            target_type: AssetTypeId::new(),
            target_type_name: "test".to_string(),
            anchors: vec![],
            project_id: ProjectId::new(),
            organization_id: OrganizationId::new(),
            created_by: "test".to_string(),
            created_at: Utc::now() - chrono::Duration::hours(2),
            expires_at: Utc::now() - chrono::Duration::hours(1),
            context_summary: "test".to_string(),
        };

        assert!(instance.is_expired());
    }
}
