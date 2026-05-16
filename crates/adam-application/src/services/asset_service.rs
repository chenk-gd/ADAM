//! Asset service for business logic
//!
//! Extracts business logic from REST handlers for Clean Architecture compliance.

use adam_domain::{
    AssetId, AssetInstance, AssetLevel, AssetRepository, CreateAssetCommand, DependencyRepository,
    ProjectId, RepositoryError,
};

/// Errors that can occur in AssetService
#[derive(Debug, thiserror::Error)]
pub enum AssetServiceError {
    /// Repository error
    #[error("Repository error: {0}")]
    Repository(#[from] RepositoryError),
    /// Cross-project dependency not allowed
    #[error("Cross-project dependencies are not allowed")]
    CrossProjectDependencyNotAllowed,
    /// Project cannot depend on organization-level asset
    #[error("Project-level assets cannot depend on organization-level assets")]
    ProjectDependsOnOrgNotAllowed,
    /// Invalid level configuration
    #[error("Invalid level configuration: {0}")]
    InvalidLevel(String),
}

/// Service for asset business logic
pub struct AssetService<R: AssetRepository, D: DependencyRepository> {
    asset_repo: R,
    dependency_repo: D,
}

impl<R: AssetRepository, D: DependencyRepository> AssetService<R, D> {
    /// Create a new AssetService
    pub fn new(asset_repo: R, dependency_repo: D) -> Self {
        Self {
            asset_repo,
            dependency_repo,
        }
    }

    /// Create a new asset with validation
    pub async fn create(
        &self,
        cmd: &CreateAssetCommand,
        dependencies: Option<&[AssetId]>,
    ) -> Result<AssetInstance, AssetServiceError> {
        // Validate level configuration
        Self::validate_level(cmd.level, cmd.project_id)?;

        // Validate dependencies if provided
        if let Some(deps) = dependencies {
            for dep_id in deps {
                self.validate_dependency(cmd.project_id, cmd.level, dep_id)
                    .await?;
            }
        }

        // Create the asset
        let asset = self.asset_repo.create(cmd).await?;

        // Create dependency relationships if provided
        if let Some(deps) = dependencies {
            for dep_id in deps {
                self.dependency_repo
                    .create_dependency(&asset.id, dep_id)
                    .await
                    .map_err(AssetServiceError::from)?;
            }
        }

        Ok(asset)
    }

    /// Validate dependency rules
    async fn validate_dependency(
        &self,
        source_project_id: Option<ProjectId>,
        source_level: AssetLevel,
        target_id: &AssetId,
    ) -> Result<(), AssetServiceError> {
        let target = self
            .asset_repo
            .find_by_id(target_id)
            .await?
            .ok_or_else(|| RepositoryError::NotFound(format!("{target_id:?}")))?;

        // Rule 1: Cross-project dependencies are not allowed
        // Project-level assets can only depend on assets in the same project
        if source_level == AssetLevel::Project {
            if target.level == AssetLevel::Project {
                // Both are project-level, must be in same project
                if source_project_id != target.project_id {
                    return Err(AssetServiceError::CrossProjectDependencyNotAllowed);
                }
            } else {
                // Project-level asset cannot depend on org-level asset
                return Err(AssetServiceError::ProjectDependsOnOrgNotAllowed);
            }
        }

        // Rule 2: Org-level assets can depend on org-level assets (no project restriction)
        // This is automatically valid since org-level assets have no project_id

        Ok(())
    }

    /// Validate level configuration
    fn validate_level(
        level: AssetLevel,
        project_id: Option<ProjectId>,
    ) -> Result<(), AssetServiceError> {
        match level {
            AssetLevel::Project => {
                if project_id.is_none() {
                    return Err(AssetServiceError::InvalidLevel(
                        "Project-level assets must have a project_id".to_string(),
                    ));
                }
            }
            AssetLevel::Organization => {
                if project_id.is_some() {
                    return Err(AssetServiceError::InvalidLevel(
                        "Organization-level assets cannot have a project_id".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Find asset by ID
    pub async fn find_by_id(
        &self,
        id: &AssetId,
    ) -> Result<Option<AssetInstance>, AssetServiceError> {
        self.asset_repo
            .find_by_id(id)
            .await
            .map_err(AssetServiceError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adam_domain::{
        AssetId, AssetLevel, AssetTypeId, DependencyRepository, InMemoryAssetRepository,
        OrganizationId, ProjectId, RepositoryError,
    };
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::Mutex;

    // Mock DependencyRepository for testing
    struct MockDependencyRepo {
        data: Mutex<HashMap<AssetId, Vec<AssetId>>>, // target -> sources
    }

    impl MockDependencyRepo {
        fn new() -> Self {
            Self {
                data: Mutex::new(HashMap::new()),
            }
        }
    }

    impl Default for MockDependencyRepo {
        fn default() -> Self {
            Self::new()
        }
    }

    #[async_trait]
    impl DependencyRepository for MockDependencyRepo {
        async fn find_downstream(
            &self,
            _asset_id: &AssetId,
        ) -> Result<Vec<AssetId>, RepositoryError> {
            Ok(vec![])
        }

        async fn find_upstream(
            &self,
            _asset_id: &AssetId,
        ) -> Result<Vec<AssetId>, RepositoryError> {
            Ok(vec![])
        }

        async fn create_dependency(
            &self,
            source: &AssetId,
            target: &AssetId,
        ) -> Result<(), RepositoryError> {
            let mut data = self.data.lock().unwrap();
            data.entry(*target).or_default().push(*source);
            Ok(())
        }
    }

    #[tokio::test]
    async fn create_asset_succeeds() {
        let org_id = OrganizationId::new();
        let project_id = ProjectId::new();
        let type_id = AssetTypeId::new();

        let asset_repo = InMemoryAssetRepository::new();
        let dep_repo = MockDependencyRepo::new();
        let service = AssetService::new(asset_repo, dep_repo);

        let cmd = CreateAssetCommand {
            name: "Test Asset".to_string(),
            asset_type_id: type_id,
            project_id: Some(project_id),
            organization_id: org_id,
            level: AssetLevel::Project,
            external_ref: "https://example.com/asset/1".to_string(),
            source: "manual".to_string(),
            metadata: serde_json::json!({}),
            idempotency_key: None,
        };

        let asset = service.create(&cmd, None).await.unwrap();
        assert_eq!(asset.name, "Test Asset");
        assert_eq!(asset.level, AssetLevel::Project);
    }

    #[tokio::test]
    async fn cross_project_dependency_fails() {
        let org_id = OrganizationId::new();
        let project_id1 = ProjectId::new();
        let project_id2 = ProjectId::new();
        let type_id = AssetTypeId::new();

        let asset_repo = InMemoryAssetRepository::new();
        let dep_repo = MockDependencyRepo::new();
        let service = AssetService::new(asset_repo, dep_repo);

        // Create first asset in project 1
        let cmd1 = CreateAssetCommand {
            name: "Asset 1".to_string(),
            asset_type_id: type_id,
            project_id: Some(project_id1),
            organization_id: org_id,
            level: AssetLevel::Project,
            external_ref: "https://example.com/asset/1".to_string(),
            source: "manual".to_string(),
            metadata: serde_json::json!({}),
            idempotency_key: None,
        };
        let asset1 = service.create(&cmd1, None).await.unwrap();

        // Try to create asset in project 2 that depends on asset1 (in project 1)
        let cmd2 = CreateAssetCommand {
            name: "Asset 2".to_string(),
            asset_type_id: type_id,
            project_id: Some(project_id2),
            organization_id: org_id,
            level: AssetLevel::Project,
            external_ref: "https://example.com/asset/2".to_string(),
            source: "manual".to_string(),
            metadata: serde_json::json!({}),
            idempotency_key: None,
        };
        let result = service.create(&cmd2, Some(&[asset1.id])).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            AssetServiceError::CrossProjectDependencyNotAllowed
        ));
    }

    #[tokio::test]
    async fn project_depends_on_org_fails() {
        let org_id = OrganizationId::new();
        let project_id = ProjectId::new();
        let type_id = AssetTypeId::new();

        let asset_repo = InMemoryAssetRepository::new();
        let dep_repo = MockDependencyRepo::new();
        let service = AssetService::new(asset_repo, dep_repo);

        // Create org-level asset
        let cmd_org = CreateAssetCommand {
            name: "Org Asset".to_string(),
            asset_type_id: type_id,
            project_id: None,
            organization_id: org_id,
            level: AssetLevel::Organization,
            external_ref: "https://example.com/org-asset".to_string(),
            source: "manual".to_string(),
            metadata: serde_json::json!({}),
            idempotency_key: None,
        };
        let org_asset = service.create(&cmd_org, None).await.unwrap();

        // Try to create project-level asset that depends on org-level asset
        let cmd_proj = CreateAssetCommand {
            name: "Project Asset".to_string(),
            asset_type_id: type_id,
            project_id: Some(project_id),
            organization_id: org_id,
            level: AssetLevel::Project,
            external_ref: "https://example.com/proj-asset".to_string(),
            source: "manual".to_string(),
            metadata: serde_json::json!({}),
            idempotency_key: None,
        };
        let result = service.create(&cmd_proj, Some(&[org_asset.id])).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            AssetServiceError::ProjectDependsOnOrgNotAllowed
        ));
    }

    #[tokio::test]
    async fn org_depends_on_org_succeeds() {
        let org_id = OrganizationId::new();
        let type_id = AssetTypeId::new();

        let asset_repo = InMemoryAssetRepository::new();
        let dep_repo = MockDependencyRepo::new();
        let service = AssetService::new(asset_repo, dep_repo);

        // Create first org-level asset
        let cmd1 = CreateAssetCommand {
            name: "Org Asset 1".to_string(),
            asset_type_id: type_id,
            project_id: None,
            organization_id: org_id,
            level: AssetLevel::Organization,
            external_ref: "https://example.com/org1".to_string(),
            source: "manual".to_string(),
            metadata: serde_json::json!({}),
            idempotency_key: None,
        };
        let asset1 = service.create(&cmd1, None).await.unwrap();

        // Create second org-level asset that depends on first
        let cmd2 = CreateAssetCommand {
            name: "Org Asset 2".to_string(),
            asset_type_id: type_id,
            project_id: None,
            organization_id: org_id,
            level: AssetLevel::Organization,
            external_ref: "https://example.com/org2".to_string(),
            source: "manual".to_string(),
            metadata: serde_json::json!({}),
            idempotency_key: None,
        };
        let asset2 = service.create(&cmd2, Some(&[asset1.id])).await.unwrap();

        assert_eq!(asset2.level, AssetLevel::Organization);
    }

    #[tokio::test]
    async fn project_level_requires_project_id() {
        let org_id = OrganizationId::new();
        let type_id = AssetTypeId::new();

        let asset_repo = InMemoryAssetRepository::new();
        let dep_repo = MockDependencyRepo::new();
        let service = AssetService::new(asset_repo, dep_repo);

        let cmd = CreateAssetCommand {
            name: "Test Asset".to_string(),
            asset_type_id: type_id,
            project_id: None, // Missing project_id
            organization_id: org_id,
            level: AssetLevel::Project,
            external_ref: "https://example.com/asset/1".to_string(),
            source: "manual".to_string(),
            metadata: serde_json::json!({}),
            idempotency_key: None,
        };

        let result = service.create(&cmd, None).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            AssetServiceError::InvalidLevel(_)
        ));
    }

    #[tokio::test]
    async fn org_level_cannot_have_project_id() {
        let org_id = OrganizationId::new();
        let project_id = ProjectId::new();
        let type_id = AssetTypeId::new();

        let asset_repo = InMemoryAssetRepository::new();
        let dep_repo = MockDependencyRepo::new();
        let service = AssetService::new(asset_repo, dep_repo);

        let cmd = CreateAssetCommand {
            name: "Test Asset".to_string(),
            asset_type_id: type_id,
            project_id: Some(project_id), // Should not be set for org-level
            organization_id: org_id,
            level: AssetLevel::Organization,
            external_ref: "https://example.com/asset/1".to_string(),
            source: "manual".to_string(),
            metadata: serde_json::json!({}),
            idempotency_key: None,
        };

        let result = service.create(&cmd, None).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            AssetServiceError::InvalidLevel(_)
        ));
    }
}
