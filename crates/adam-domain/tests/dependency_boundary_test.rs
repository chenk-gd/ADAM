use adam_domain::dependency::boundary::{AssetLevel, DependencyBoundaryContext, DependencyError};
use adam_domain::{OrganizationId, ProjectId};

fn create_test_context(
    source_level: AssetLevel,
    target_level: AssetLevel,
    source_project_id: Option<ProjectId>,
    target_project_id: Option<ProjectId>,
    source_org_id: OrganizationId,
    target_org_id: OrganizationId,
) -> DependencyBoundaryContext {
    DependencyBoundaryContext {
        source_level,
        target_level,
        source_project_id,
        target_project_id,
        source_org_id,
        target_org_id,
    }
}

#[test]
fn project_asset_cannot_depend_on_organization_asset() {
    let org_id = OrganizationId::new();
    let ctx = create_test_context(
        AssetLevel::Project,
        AssetLevel::Organization,
        Some(ProjectId::new()),
        None,
        org_id,
        org_id,
    );
    assert!(matches!(
        ctx.validate(),
        Err(DependencyError::ProjectCannotDependOnOrganization)
    ));
}

#[test]
fn organization_asset_cannot_depend_on_project_asset() {
    let org_id = OrganizationId::new();
    let ctx = create_test_context(
        AssetLevel::Organization,
        AssetLevel::Project,
        None,
        Some(ProjectId::new()),
        org_id,
        org_id,
    );
    assert!(matches!(
        ctx.validate(),
        Err(DependencyError::OrganizationCannotDependOnProject)
    ));
}

#[test]
fn same_project_dependency_is_valid() {
    let org_id = OrganizationId::new();
    let project_id = ProjectId::new();
    let ctx = create_test_context(
        AssetLevel::Project,
        AssetLevel::Project,
        Some(project_id),
        Some(project_id),
        org_id,
        org_id,
    );
    assert!(ctx.validate().is_ok());
}

#[test]
fn cross_project_dependency_is_invalid() {
    let org_id = OrganizationId::new();
    let project_a = ProjectId::new();
    let project_b = ProjectId::new();
    let ctx = create_test_context(
        AssetLevel::Project,
        AssetLevel::Project,
        Some(project_a),
        Some(project_b),
        org_id,
        org_id,
    );
    assert!(matches!(
        ctx.validate(),
        Err(DependencyError::CrossProjectDependency)
    ));
}

#[test]
fn same_organization_org_level_dependency_is_valid() {
    let org_id = OrganizationId::new();
    let ctx = create_test_context(
        AssetLevel::Organization,
        AssetLevel::Organization,
        None,
        None,
        org_id,
        org_id,
    );
    assert!(ctx.validate().is_ok());
}

#[test]
fn cross_organization_dependency_is_invalid() {
    let org_a = OrganizationId::new();
    let org_b = OrganizationId::new();
    let ctx = create_test_context(
        AssetLevel::Organization,
        AssetLevel::Organization,
        None,
        None,
        org_a,
        org_b,
    );
    assert!(matches!(
        ctx.validate(),
        Err(DependencyError::CrossOrganizationDependency)
    ));
}

#[test]
fn project_asset_without_project_id_is_invalid() {
    let org_id = OrganizationId::new();
    let ctx = create_test_context(
        AssetLevel::Project,
        AssetLevel::Project,
        None, // Missing project ID
        Some(ProjectId::new()),
        org_id,
        org_id,
    );
    assert!(matches!(
        ctx.validate(),
        Err(DependencyError::MissingProjectId)
    ));
}

#[test]
fn organization_asset_with_project_id_is_invalid() {
    let org_id = OrganizationId::new();
    let ctx = create_test_context(
        AssetLevel::Organization,
        AssetLevel::Organization,
        Some(ProjectId::new()), // Should be None
        None,
        org_id,
        org_id,
    );
    assert!(matches!(
        ctx.validate(),
        Err(DependencyError::OrganizationAssetHasProjectId)
    ));
}
