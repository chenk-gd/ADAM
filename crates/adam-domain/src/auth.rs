//! Authentication and authorization types for ADAM

use crate::{OrganizationId, ProjectId};
use std::collections::HashSet;

/// Authentication principal for users and AI agents
#[derive(Debug, Clone)]
pub struct AuthPrincipal {
    /// User/agent ID
    pub id: String,
    /// Organization ID (scope boundary)
    pub organization_id: OrganizationId,
    /// Project memberships (for project-level access)
    pub project_memberships: Vec<ProjectId>,
    /// Roles assigned to this principal
    pub roles: Vec<Role>,
}

impl AuthPrincipal {
    /// Check if principal is a member of the given project
    pub fn is_member_of(&self, project_id: &ProjectId) -> bool {
        self.project_memberships.contains(project_id)
    }

    /// Check if principal has a specific permission
    pub fn has_permission(&self, permission: Permission) -> bool {
        self.roles
            .iter()
            .any(|role| role.permissions().contains(&permission))
    }

    /// Check if principal has permission for a specific project
    /// OrgAdmin and SystemAdmin can bypass project membership checks within their org
    pub fn has_permission_for_project(&self, permission: Permission, project_id: &ProjectId) -> bool {
        // First check if has the permission
        if !self.has_permission(permission) {
            return false;
        }

        // OrgAdmin and SystemAdmin can access any project in their org
        let is_org_admin = self
            .roles
            .iter()
            .any(|r| matches!(r, Role::OrgAdmin | Role::SystemAdmin));

        if is_org_admin {
            return true;
        }

        // Otherwise must be a member of the project
        self.is_member_of(project_id)
    }

    /// Comprehensive permission check for organization and optional project
    /// Returns Ok(()) if allowed, Err(AuthorizationError) if denied
    pub fn can(
        &self,
        permission: Permission,
        resource_org: OrganizationId,
        resource_project: Option<ProjectId>,
    ) -> Result<(), AuthorizationError> {
        // 1. Organization boundary check
        if self.organization_id != resource_org {
            return Err(AuthorizationError::CrossOrganizationAccessDenied);
        }

        // 2. Permission check
        if !self.has_permission(permission) {
            return Err(AuthorizationError::PermissionDenied {
                required: permission,
            });
        }

        // 3. Project membership check for project-level resources
        // OrgAdmin and SystemAdmin can bypass project membership checks within their org
        let is_org_admin = self
            .roles
            .iter()
            .any(|r| matches!(r, Role::OrgAdmin | Role::SystemAdmin));

        if let Some(project_id) = resource_project {
            if !is_org_admin && !self.is_member_of(&project_id) {
                return Err(AuthorizationError::ProjectAccessDenied(project_id));
            }
        }

        Ok(())
    }
}

/// User/Actor roles
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// System administrator - all permissions
    SystemAdmin,
    /// Organization administrator
    OrgAdmin,
    /// Project administrator
    ProjectAdmin,
    /// Developer - can create and publish
    Developer,
    /// Reader - read-only access
    Reader,
    /// AI Agent - limited read + virtual context
    AiAgent,
}

/// Permissions for asset management
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Permission {
    /// Create asset types
    AssetTypeCreate,
    /// Read asset types
    AssetTypeRead,
    /// Update asset types
    AssetTypeUpdate,
    /// Create assets
    AssetCreate,
    /// Read assets
    AssetRead,
    /// Update assets
    AssetUpdate,
    /// Delete assets
    AssetDelete,
    /// Publish versions
    VersionPublish,
    /// Read versions
    VersionRead,
    /// Create dependencies
    DependencyCreate,
    /// Read dependencies
    DependencyRead,
    /// Delete dependencies
    DependencyDelete,
    /// Manual clean state
    StateManualClean,
    /// Archive assets
    StateArchive,
    /// Refresh state
    StateRefresh,
    /// Query assets (MCP tool)
    QueryAssets,
    /// Query impact analysis (MCP tool)
    QueryImpactAnalysis,
    /// Query virtual context (MCP tool)
    QueryVirtualContext,
}

impl Role {
    /// Get permissions for this role
    pub fn permissions(&self) -> HashSet<Permission> {
        match self {
            Role::SystemAdmin => [
                Permission::AssetTypeCreate,
                Permission::AssetTypeRead,
                Permission::AssetTypeUpdate,
                Permission::AssetCreate,
                Permission::AssetRead,
                Permission::AssetUpdate,
                Permission::AssetDelete,
                Permission::VersionPublish,
                Permission::VersionRead,
                Permission::DependencyCreate,
                Permission::DependencyRead,
                Permission::DependencyDelete,
                Permission::StateManualClean,
                Permission::StateArchive,
                Permission::StateRefresh,
                Permission::QueryAssets,
                Permission::QueryImpactAnalysis,
                Permission::QueryVirtualContext,
            ]
            .into_iter()
            .collect(),
            Role::OrgAdmin => [
                Permission::AssetTypeCreate,
                Permission::AssetTypeRead,
                Permission::AssetTypeUpdate,
                Permission::AssetCreate,
                Permission::AssetRead,
                Permission::AssetUpdate,
                Permission::AssetDelete,
                Permission::VersionPublish,
                Permission::VersionRead,
                Permission::DependencyCreate,
                Permission::DependencyRead,
                Permission::DependencyDelete,
                Permission::StateManualClean,
                Permission::StateArchive,
                Permission::StateRefresh,
                Permission::QueryAssets,
                Permission::QueryImpactAnalysis,
                Permission::QueryVirtualContext,
            ]
            .into_iter()
            .collect(),
            Role::ProjectAdmin => [
                Permission::AssetRead,
                Permission::AssetCreate,
                Permission::AssetUpdate,
                Permission::VersionPublish,
                Permission::VersionRead,
                Permission::DependencyCreate,
                Permission::DependencyRead,
                Permission::StateManualClean,
                Permission::StateRefresh,
                Permission::QueryAssets,
                Permission::QueryImpactAnalysis,
            ]
            .into_iter()
            .collect(),
            Role::Developer => [
                Permission::AssetRead,
                Permission::AssetCreate,
                Permission::VersionPublish,
                Permission::VersionRead,
                Permission::DependencyCreate,
                Permission::DependencyRead,
                Permission::StateManualClean,
                Permission::StateRefresh,
                Permission::QueryAssets,
                Permission::QueryImpactAnalysis,
                Permission::QueryVirtualContext,
            ]
            .into_iter()
            .collect(),
            Role::Reader => [
                Permission::AssetTypeRead,
                Permission::AssetRead,
                Permission::VersionRead,
                Permission::DependencyRead,
                Permission::QueryAssets,
                Permission::QueryImpactAnalysis,
            ]
            .into_iter()
            .collect(),
            Role::AiAgent => [
                Permission::AssetRead,
                Permission::VersionRead,
                Permission::DependencyRead,
                Permission::QueryAssets,
                Permission::QueryImpactAnalysis,
                Permission::QueryVirtualContext,
            ]
            .into_iter()
            .collect(),
        }
    }
}

/// Authorization errors
#[derive(Debug, thiserror::Error)]
pub enum AuthorizationError {
    #[error("Cross-organization access denied")]
    CrossOrganizationAccessDenied,
    #[error("Project access denied for project {0:?}")]
    ProjectAccessDenied(ProjectId),
    #[error("Permission denied: {required:?} required")]
    PermissionDenied { required: Permission },
    #[error("Project not found: {0:?}")]
    ProjectNotFound(ProjectId),
}

/// Authorization service for permission checks
pub struct AuthorizationService;

impl AuthorizationService {
    /// Check if principal has permission for resource
    pub fn check(
        principal: &AuthPrincipal,
        permission: Permission,
        resource_org: OrganizationId,
        resource_project: Option<ProjectId>,
    ) -> Result<(), AuthorizationError> {
        // 1. Organization boundary check
        if principal.organization_id != resource_org {
            return Err(AuthorizationError::CrossOrganizationAccessDenied);
        }

        // 2. Permission check
        let has_permission = principal
            .roles
            .iter()
            .any(|role| role.permissions().contains(&permission));

        if !has_permission {
            return Err(AuthorizationError::PermissionDenied {
                required: permission,
            });
        }

        // 3. Project membership check for project-level resources
        // OrgAdmin and SystemAdmin can bypass project membership checks within their org
        let is_org_admin = principal
            .roles
            .iter()
            .any(|r| matches!(r, Role::OrgAdmin | Role::SystemAdmin));

        if let Some(project_id) = resource_project {
            if !is_org_admin && !principal.project_memberships.contains(&project_id) {
                return Err(AuthorizationError::ProjectAccessDenied(project_id));
            }
        }

        Ok(())
    }

    /// Check if principal can access specific asset
    pub fn check_asset_access(
        principal: &AuthPrincipal,
        asset_org: OrganizationId,
        asset_project: Option<ProjectId>,
    ) -> Result<(), AuthorizationError> {
        // Organization boundary
        if principal.organization_id != asset_org {
            return Err(AuthorizationError::CrossOrganizationAccessDenied);
        }

        // Project membership for project-level assets
        // OrgAdmin and SystemAdmin can bypass within their organization
        let is_org_admin = principal
            .roles
            .iter()
            .any(|r| matches!(r, Role::OrgAdmin | Role::SystemAdmin));

        if let Some(project_id) = asset_project {
            if !is_org_admin && !principal.project_memberships.contains(&project_id) {
                return Err(AuthorizationError::ProjectAccessDenied(project_id));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_principal(roles: Vec<Role>) -> AuthPrincipal {
        AuthPrincipal {
            id: "test-user".to_string(),
            organization_id: OrganizationId::new(),
            project_memberships: vec![ProjectId::new()],
            roles,
        }
    }

    #[test]
    fn auth_principal_can_check_project_membership() {
        let project_1 = ProjectId::new();
        let project_2 = ProjectId::new();
        let principal = AuthPrincipal {
            id: "user-123".to_string(),
            organization_id: OrganizationId::new(),
            roles: vec![Role::Developer],
            project_memberships: vec![project_1.clone()],
        };

        // Check membership
        assert!(principal.is_member_of(&project_1));
        assert!(!principal.is_member_of(&project_2));
    }

    #[test]
    fn developer_has_project_permissions_in_member_projects() {
        let project_1 = ProjectId::new();
        let project_2 = ProjectId::new();
        let principal = AuthPrincipal {
            id: "user-123".to_string(),
            organization_id: OrganizationId::new(),
            roles: vec![Role::Developer],
            project_memberships: vec![project_1.clone()],
        };

        // Developer has AssetCreate permission in member project
        assert!(principal.has_permission_for_project(Permission::AssetCreate, &project_1));

        // Developer does NOT have permission in non-member project (no OrgAdmin bypass)
        assert!(!principal.has_permission_for_project(Permission::AssetCreate, &project_2));

        // Developer always has read permission
        assert!(principal.has_permission(Permission::AssetRead));
    }

    #[test]
    fn org_admin_can_access_any_project_in_org() {
        let project_1 = ProjectId::new();
        let project_2 = ProjectId::new();
        let principal = AuthPrincipal {
            id: "admin".to_string(),
            organization_id: OrganizationId::new(),
            roles: vec![Role::OrgAdmin],
            project_memberships: vec![], // Not a member of any project
        };

        // OrgAdmin can access any project without being a member
        assert!(principal.has_permission_for_project(Permission::AssetCreate, &project_1));
        assert!(principal.has_permission_for_project(Permission::AssetCreate, &project_2));
        assert!(principal.has_permission_for_project(Permission::AssetDelete, &project_1));
    }

    #[test]
    fn system_admin_can_access_same_org_without_project_membership() {
        let org_id = OrganizationId::new();
        let principal = AuthPrincipal {
            id: "admin".to_string(),
            organization_id: org_id.clone(),
            roles: vec![Role::SystemAdmin],
            project_memberships: vec![], // Not a member of any project
        };

        // SystemAdmin can access any project in same org
        assert!(principal
            .can(
                Permission::AssetCreate,
                org_id.clone(),
                Some(ProjectId::new())
            )
            .is_ok());

        // SystemAdmin can delete assets
        assert!(principal
            .can(
                Permission::AssetDelete,
                org_id.clone(),
                Some(ProjectId::new())
            )
            .is_ok());
    }

    #[test]
    fn system_admin_cannot_cross_organization_boundary() {
        let org_1 = OrganizationId::new();
        let org_2 = OrganizationId::new();
        let principal = AuthPrincipal {
            id: "admin".to_string(),
            organization_id: org_1,
            roles: vec![Role::SystemAdmin],
            project_memberships: vec![],
        };

        // SystemAdmin cannot access different org
        let result = principal.can(Permission::AssetRead, org_2, None);
        assert!(matches!(
            result,
            Err(AuthorizationError::CrossOrganizationAccessDenied)
        ));
    }

    #[test]
    fn system_admin_has_all_permissions() {
        let principal = test_principal(vec![Role::SystemAdmin]);
        assert!(
            AuthorizationService::check(
                &principal,
                Permission::QueryAssets,
                principal.organization_id,
                Some(principal.project_memberships[0]),
            )
            .is_ok()
        );
    }

    #[test]
    fn ai_agent_has_query_virtual_context() {
        let principal = test_principal(vec![Role::AiAgent]);
        assert!(
            AuthorizationService::check(
                &principal,
                Permission::QueryVirtualContext,
                principal.organization_id,
                Some(principal.project_memberships[0]),
            )
            .is_ok()
        );
    }

    #[test]
    fn ai_agent_cannot_create_assets() {
        let principal = test_principal(vec![Role::AiAgent]);
        assert!(
            AuthorizationService::check(
                &principal,
                Permission::AssetCreate,
                principal.organization_id,
                Some(principal.project_memberships[0]),
            )
            .is_err()
        );
    }

    #[test]
    fn cross_organization_access_denied() {
        let principal = test_principal(vec![Role::SystemAdmin]);
        let other_org = OrganizationId::new();
        assert!(matches!(
            AuthorizationService::check(&principal, Permission::QueryAssets, other_org, None,)
                .unwrap_err(),
            AuthorizationError::CrossOrganizationAccessDenied
        ));
    }

    #[test]
    fn project_access_denied() {
        let principal = test_principal(vec![Role::Developer]);
        let other_project = ProjectId::new();
        assert!(matches!(
            AuthorizationService::check(
                &principal,
                Permission::QueryAssets,
                principal.organization_id,
                Some(other_project),
            )
            .unwrap_err(),
            AuthorizationError::ProjectAccessDenied(_)
        ));
    }
}
