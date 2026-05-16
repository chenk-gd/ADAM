//! Dependency boundary validation module
//!
//! Enforces BR-008: Organization boundary and asset level constraints

use crate::asset::instance::{OrganizationId, ProjectId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Asset level enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AssetLevel {
    /// Project-level asset belongs to a specific project
    Project,
    /// Organization-level asset is shared across projects
    Organization,
}

/// Errors that can occur during dependency validation
#[derive(Debug, Error, PartialEq)]
pub enum DependencyError {
    /// Project-level asset cannot depend on organization-level asset
    #[error("Project-level asset cannot depend on organization-level asset")]
    ProjectCannotDependOnOrganization,
    /// Organization-level asset cannot depend on project-level asset
    #[error("Organization-level asset can only depend on organization-level assets")]
    OrganizationCannotDependOnProject,
    /// Cross-project dependency is not allowed
    #[error("Cross-project dependency not allowed")]
    CrossProjectDependency,
    /// Cross-organization dependency is not allowed
    #[error("Cross-organization dependency not allowed")]
    CrossOrganizationDependency,
    /// Project-level asset must have a project ID
    #[error("Project-level asset must have a project ID")]
    MissingProjectId,
    /// Organization-level asset should not have a project ID
    #[error("Organization-level asset should not have a project ID")]
    OrganizationAssetHasProjectId,
}

/// Context for validating dependency boundaries
#[derive(Debug, Clone)]
pub struct DependencyBoundaryContext {
    /// Level of the source asset (dependent)
    pub source_level: AssetLevel,
    /// Level of the target asset (dependency)
    pub target_level: AssetLevel,
    /// Project ID of the source asset (if project-level)
    pub source_project_id: Option<ProjectId>,
    /// Project ID of the target asset (if project-level)
    pub target_project_id: Option<ProjectId>,
    /// Organization ID of the source asset
    pub source_org_id: OrganizationId,
    /// Organization ID of the target asset
    pub target_org_id: OrganizationId,
}

impl DependencyBoundaryContext {
    /// Validate the dependency boundary constraints
    ///
    /// # Validation Rules (BR-008)
    ///
    /// 1. Cross-organization dependencies are prohibited
    /// 2. Project-level assets can only depend on assets within the same project
    /// 3. Organization-level assets can only depend on other organization-level assets
    /// 4. Project-level assets cannot depend on organization-level assets
    pub fn validate(&self) -> Result<(), DependencyError> {
        // Rule 1: Cross-organization check
        if self.source_org_id != self.target_org_id {
            return Err(DependencyError::CrossOrganizationDependency);
        }

        // Validate project ID consistency
        match self.source_level {
            AssetLevel::Project => {
                if self.source_project_id.is_none() {
                    return Err(DependencyError::MissingProjectId);
                }
            }
            AssetLevel::Organization => {
                if self.source_project_id.is_some() {
                    return Err(DependencyError::OrganizationAssetHasProjectId);
                }
            }
        }

        // Level-based validation
        match (self.source_level, self.target_level) {
            // Rule 4: Project -> Organization is prohibited
            (AssetLevel::Project, AssetLevel::Organization) => {
                Err(DependencyError::ProjectCannotDependOnOrganization)
            }
            // Rule 3: Organization -> Project is prohibited
            (AssetLevel::Organization, AssetLevel::Project) => {
                Err(DependencyError::OrganizationCannotDependOnProject)
            }
            // Project -> Project: must be same project
            (AssetLevel::Project, AssetLevel::Project) => {
                if self.source_project_id != self.target_project_id {
                    Err(DependencyError::CrossProjectDependency)
                } else {
                    Ok(())
                }
            }
            // Organization -> Organization: already checked cross-org
            (AssetLevel::Organization, AssetLevel::Organization) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(
        source_level: AssetLevel,
        target_level: AssetLevel,
        source_project: Option<ProjectId>,
        target_project: Option<ProjectId>,
        source_org: OrganizationId,
        target_org: OrganizationId,
    ) -> DependencyBoundaryContext {
        DependencyBoundaryContext {
            source_level,
            target_level,
            source_project_id: source_project,
            target_project_id: target_project,
            source_org_id: source_org,
            target_org_id: target_org,
        }
    }

    #[test]
    fn test_same_project_is_valid() {
        let org = OrganizationId::new();
        let project = ProjectId::new();
        let context = ctx(
            AssetLevel::Project,
            AssetLevel::Project,
            Some(project),
            Some(project),
            org,
            org,
        );
        assert!(context.validate().is_ok());
    }

    #[test]
    fn test_cross_project_is_invalid() {
        let org = OrganizationId::new();
        let project_a = ProjectId::new();
        let project_b = ProjectId::new();
        let context = ctx(
            AssetLevel::Project,
            AssetLevel::Project,
            Some(project_a),
            Some(project_b),
            org,
            org,
        );
        assert_eq!(
            context.validate(),
            Err(DependencyError::CrossProjectDependency)
        );
    }

    #[test]
    fn test_project_cannot_depend_on_organization() {
        let org = OrganizationId::new();
        let context = ctx(
            AssetLevel::Project,
            AssetLevel::Organization,
            Some(ProjectId::new()),
            None,
            org,
            org,
        );
        assert_eq!(
            context.validate(),
            Err(DependencyError::ProjectCannotDependOnOrganization)
        );
    }

    #[test]
    fn test_organization_cannot_depend_on_project() {
        let org = OrganizationId::new();
        let context = ctx(
            AssetLevel::Organization,
            AssetLevel::Project,
            None,
            Some(ProjectId::new()),
            org,
            org,
        );
        assert_eq!(
            context.validate(),
            Err(DependencyError::OrganizationCannotDependOnProject)
        );
    }

    #[test]
    fn test_cross_organization_is_invalid() {
        let org_a = OrganizationId::new();
        let org_b = OrganizationId::new();
        let context = ctx(
            AssetLevel::Organization,
            AssetLevel::Organization,
            None,
            None,
            org_a,
            org_b,
        );
        assert_eq!(
            context.validate(),
            Err(DependencyError::CrossOrganizationDependency)
        );
    }

    #[test]
    fn test_same_organization_org_level_is_valid() {
        let org = OrganizationId::new();
        let context = ctx(
            AssetLevel::Organization,
            AssetLevel::Organization,
            None,
            None,
            org,
            org,
        );
        assert!(context.validate().is_ok());
    }
}
