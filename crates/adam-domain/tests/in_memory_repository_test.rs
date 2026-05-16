//! Integration tests for in-memory repositories

use adam_domain::asset::state::AssetState;
use adam_domain::repository::{
    AssetRepository, CreateAssetCommand, DependencyRepository, DirtyQueueEntry,
    DirtyQueueRepository, RepositoryError,
};
use adam_domain::{
    AssetId, AssetTypeId, InMemoryAssetRepository, InMemoryDependencyRepository,
    InMemoryDirtyQueueRepository, OrganizationId, ProjectId,
};
use chrono::Utc;

#[tokio::test]
async fn memory_repo_creates_asset() {
    let repo = InMemoryAssetRepository::new();
    let org_id = OrganizationId::new();
    let project_id = ProjectId::new();
    let type_id = AssetTypeId::new();

    let cmd = CreateAssetCommand {
        name: "Test Asset".to_string(),
        asset_type_id: type_id,
        project_id: Some(project_id),
        organization_id: org_id,
        level: adam_domain::AssetLevel::Project,
        external_ref: "https://example.com/asset/1".to_string(),
        source: "manual".to_string(),
        metadata: serde_json::json!({}),
        idempotency_key: None,
    };

    let asset = repo.create(&cmd).await.unwrap();
    assert_eq!(asset.name, "Test Asset");
    assert_eq!(asset.state(), AssetState::Clean);

    // Verify it can be found
    let found = repo.find_by_id(&asset.id).await.unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().name, "Test Asset");
}

#[tokio::test]
async fn memory_repo_enforces_idempotency() {
    let repo = InMemoryAssetRepository::new();
    let org_id = OrganizationId::new();
    let type_id = AssetTypeId::new();

    let cmd = CreateAssetCommand {
        name: "Test Asset".to_string(),
        asset_type_id: type_id,
        project_id: None,
        organization_id: org_id,
        level: adam_domain::AssetLevel::Organization,
        external_ref: "https://example.com/asset/1".to_string(),
        source: "manual".to_string(),
        metadata: serde_json::json!({}),
        idempotency_key: Some("git:org1:proj1:abc123".to_string()),
    };

    let asset = repo.create(&cmd).await.unwrap();
    let result = repo.create(&cmd).await;

    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        RepositoryError::DuplicateIdempotencyKey(_)
    ));

    // Verify first asset still exists
    let found = repo.find_by_id(&asset.id).await.unwrap().unwrap();
    assert_eq!(found.name, "Test Asset");
}

#[tokio::test]
async fn memory_repo_update_state() {
    let repo = InMemoryAssetRepository::new();
    let org_id = OrganizationId::new();
    let project_id = ProjectId::new();
    let type_id = AssetTypeId::new();

    let cmd = CreateAssetCommand {
        name: "Test Asset".to_string(),
        asset_type_id: type_id,
        project_id: Some(project_id),
        organization_id: org_id,
        level: adam_domain::AssetLevel::Project,
        external_ref: "https://example.com/asset/1".to_string(),
        source: "manual".to_string(),
        metadata: serde_json::json!({}),
        idempotency_key: None,
    };

    let asset = repo.create(&cmd).await.unwrap();
    assert_eq!(asset.state(), AssetState::Clean);

    repo.update_state(&asset.id, AssetState::Dirty)
        .await
        .unwrap();

    let updated = repo.find_by_id(&asset.id).await.unwrap().unwrap();
    assert_eq!(updated.state(), AssetState::Dirty);
}

#[tokio::test]
async fn memory_repo_update_state_rejects_invalid_transition() {
    let repo = InMemoryAssetRepository::new();
    let org_id = OrganizationId::new();
    let project_id = ProjectId::new();
    let type_id = AssetTypeId::new();

    let cmd = CreateAssetCommand {
        name: "Test Asset".to_string(),
        asset_type_id: type_id,
        project_id: Some(project_id),
        organization_id: org_id,
        level: adam_domain::AssetLevel::Project,
        external_ref: "https://example.com/asset/1".to_string(),
        source: "manual".to_string(),
        metadata: serde_json::json!({}),
        idempotency_key: None,
    };

    let asset = repo.create(&cmd).await.unwrap();

    // First archive the asset
    repo.update_state(&asset.id, AssetState::Archived)
        .await
        .unwrap();

    // Try to transition from Archived to Clean - should fail
    let result = repo.update_state(&asset.id, AssetState::Clean).await;
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        RepositoryError::InvalidStateTransition(_)
    ));
}

#[tokio::test]
async fn memory_repo_update_state_not_found() {
    let repo = InMemoryAssetRepository::new();
    let fake_id = AssetId::new();

    let result = repo.update_state(&fake_id, AssetState::Dirty).await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), RepositoryError::NotFound(_)));
}

#[tokio::test]
async fn dirty_queue_upsert_inserts_new_entry() {
    let repo = InMemoryDirtyQueueRepository::new();
    let asset_id = AssetId::new();
    let upstream_id = AssetId::new();

    let entry = DirtyQueueEntry {
        id: uuid::Uuid::new_v4(),
        asset_id,
        upstream_asset_id: upstream_id,
        upstream_version: "1.0.0".to_string(),
        upstream_old_version: "0.0.0".to_string(),
        impact_level: "medium".to_string(),
        since: Utc::now(),
        created_at: Utc::now(),
        resolved_at: None,
    };

    repo.upsert(&entry).await.unwrap();

    let unresolved = repo.find_unresolved_by_asset(&asset_id).await.unwrap();
    assert_eq!(unresolved.len(), 1);
    assert_eq!(unresolved[0].upstream_version, "1.0.0");
}

#[tokio::test]
async fn dirty_queue_upsert_updates_existing_entry() {
    let repo = InMemoryDirtyQueueRepository::new();
    let asset_id = AssetId::new();
    let upstream_id = AssetId::new();

    let entry1 = DirtyQueueEntry {
        id: uuid::Uuid::new_v4(),
        asset_id,
        upstream_asset_id: upstream_id,
        upstream_version: "1.0.0".to_string(),
        upstream_old_version: "0.0.0".to_string(),
        impact_level: "medium".to_string(),
        since: Utc::now(),
        created_at: Utc::now(),
        resolved_at: None,
    };

    repo.upsert(&entry1).await.unwrap();

    let entry2 = DirtyQueueEntry {
        id: uuid::Uuid::new_v4(),
        asset_id,
        upstream_asset_id: upstream_id,
        upstream_version: "2.0.0".to_string(),
        upstream_old_version: "1.0.0".to_string(),
        impact_level: "medium".to_string(),
        since: Utc::now(),
        created_at: Utc::now(),
        resolved_at: None,
    };

    repo.upsert(&entry2).await.unwrap();

    let unresolved = repo.find_unresolved_by_asset(&asset_id).await.unwrap();
    assert_eq!(unresolved.len(), 1);
    assert_eq!(unresolved[0].upstream_version, "2.0.0");
}

#[tokio::test]
async fn dirty_queue_resolve_marks_entry_resolved() {
    let repo = InMemoryDirtyQueueRepository::new();
    let asset_id = AssetId::new();
    let upstream_id = AssetId::new();

    let entry = DirtyQueueEntry {
        id: uuid::Uuid::new_v4(),
        asset_id,
        upstream_asset_id: upstream_id,
        upstream_version: "1.0.0".to_string(),
        upstream_old_version: "0.0.0".to_string(),
        impact_level: "medium".to_string(),
        since: Utc::now(),
        created_at: Utc::now(),
        resolved_at: None,
    };

    repo.upsert(&entry).await.unwrap();

    // Verify entry exists
    let unresolved_before = repo.find_unresolved_by_asset(&asset_id).await.unwrap();
    assert_eq!(unresolved_before.len(), 1);

    // Resolve the entry
    repo.resolve(&entry.id).await.unwrap();

    // Verify entry is resolved
    let unresolved_after = repo.find_unresolved_by_asset(&asset_id).await.unwrap();
    assert!(unresolved_after.is_empty());

    // Verify it's still in all_unresolved
    let all_unresolved = repo.find_all_unresolved().await.unwrap();
    assert!(all_unresolved.is_empty());
}

#[tokio::test]
async fn dirty_queue_allows_multiple_unresolved_for_different_assets() {
    let repo = InMemoryDirtyQueueRepository::new();
    let asset_id1 = AssetId::new();
    let asset_id2 = AssetId::new();
    let upstream_id = AssetId::new();

    let entry1 = DirtyQueueEntry {
        id: uuid::Uuid::new_v4(),
        asset_id: asset_id1,
        upstream_asset_id: upstream_id,
        upstream_version: "1.0.0".to_string(),
        upstream_old_version: "0.0.0".to_string(),
        impact_level: "medium".to_string(),
        since: Utc::now(),
        created_at: Utc::now(),
        resolved_at: None,
    };

    let entry2 = DirtyQueueEntry {
        id: uuid::Uuid::new_v4(),
        asset_id: asset_id2,
        upstream_asset_id: upstream_id,
        upstream_version: "1.0.0".to_string(),
        upstream_old_version: "0.0.0".to_string(),
        impact_level: "medium".to_string(),
        since: Utc::now(),
        created_at: Utc::now(),
        resolved_at: None,
    };

    repo.upsert(&entry1).await.unwrap();
    repo.upsert(&entry2).await.unwrap();

    let all_unresolved = repo.find_all_unresolved().await.unwrap();
    assert_eq!(all_unresolved.len(), 2);
}

#[tokio::test]
async fn dependency_repo_create_and_find_downstream() {
    let repo = InMemoryDependencyRepository::new();
    let asset1 = AssetId::new();
    let asset2 = AssetId::new();
    let asset3 = AssetId::new();

    // asset1 and asset2 depend on asset3 (asset3 is upstream of asset1 and asset2)
    repo.create_dependency(&asset1, &asset3).await.unwrap();
    repo.create_dependency(&asset2, &asset3).await.unwrap();

    // Find downstream of asset3 (should return asset1 and asset2)
    let downstream = repo.find_downstream(&asset3).await.unwrap();
    assert_eq!(downstream.len(), 2);
    assert!(downstream.contains(&asset1));
    assert!(downstream.contains(&asset2));
}

#[tokio::test]
async fn dependency_repo_find_upstream() {
    let repo = InMemoryDependencyRepository::new();
    let asset1 = AssetId::new();
    let asset2 = AssetId::new();
    let asset3 = AssetId::new();

    // asset1 depends on asset2 and asset3
    repo.create_dependency(&asset1, &asset2).await.unwrap();
    repo.create_dependency(&asset1, &asset3).await.unwrap();

    // Find upstream of asset1 (should return asset2 and asset3)
    let upstream = repo.find_upstream(&asset1).await.unwrap();
    assert_eq!(upstream.len(), 2);
    assert!(upstream.contains(&asset2));
    assert!(upstream.contains(&asset3));
}

#[tokio::test]
async fn dependency_repo_find_downstream_empty_when_no_dependents() {
    let repo = InMemoryDependencyRepository::new();
    let asset1 = AssetId::new();

    // No dependencies created
    let downstream = repo.find_downstream(&asset1).await.unwrap();
    assert!(downstream.is_empty());
}

#[tokio::test]
async fn dependency_repo_find_upstream_empty_when_no_dependencies() {
    let repo = InMemoryDependencyRepository::new();
    let asset1 = AssetId::new();

    // No dependencies created
    let upstream = repo.find_upstream(&asset1).await.unwrap();
    assert!(upstream.is_empty());
}

#[tokio::test]
async fn dependency_repo_multiple_downstreams_same_upstream() {
    let repo = InMemoryDependencyRepository::new();
    let upstream = AssetId::new();
    let downstreams: Vec<AssetId> = (0..5).map(|_| AssetId::new()).collect();

    for d in &downstreams {
        repo.create_dependency(d, &upstream).await.unwrap();
    }

    let found = repo.find_downstream(&upstream).await.unwrap();
    assert_eq!(found.len(), 5);
    for d in &downstreams {
        assert!(found.contains(d));
    }
}

#[tokio::test]
async fn asset_repo_find_by_id_not_found() {
    let repo = InMemoryAssetRepository::new();
    let fake_id = AssetId::new();

    let result = repo.find_by_id(&fake_id).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn asset_repo_find_by_project_id() {
    let repo = InMemoryAssetRepository::new();
    let org_id = OrganizationId::new();
    let project_id = ProjectId::new();
    let type_id = AssetTypeId::new();

    // Create asset in project
    let cmd = CreateAssetCommand {
        name: "Project Asset".to_string(),
        asset_type_id: type_id,
        project_id: Some(project_id),
        organization_id: org_id,
        level: adam_domain::AssetLevel::Project,
        external_ref: "https://example.com/asset/1".to_string(),
        source: "manual".to_string(),
        metadata: serde_json::json!({}),
        idempotency_key: None,
    };
    let _ = repo.create(&cmd).await.unwrap();

    // Create org-level asset (no project)
    let cmd2 = CreateAssetCommand {
        name: "Org Asset".to_string(),
        asset_type_id: type_id,
        project_id: None,
        organization_id: org_id,
        level: adam_domain::AssetLevel::Organization,
        external_ref: "https://example.com/asset/2".to_string(),
        source: "manual".to_string(),
        metadata: serde_json::json!({}),
        idempotency_key: None,
    };
    let _ = repo.create(&cmd2).await.unwrap();

    // Find by project_id
    let project_assets = repo.find_by_project_id(&project_id).await.unwrap();
    assert_eq!(project_assets.len(), 1);
    assert_eq!(project_assets[0].name, "Project Asset");
}

#[tokio::test]
async fn asset_repo_find_by_organization_id() {
    let repo = InMemoryAssetRepository::new();
    let org_id1 = OrganizationId::new();
    let org_id2 = OrganizationId::new();
    let project_id = ProjectId::new();
    let type_id = AssetTypeId::new();

    // Create asset in org1
    let cmd1 = CreateAssetCommand {
        name: "Org1 Asset".to_string(),
        asset_type_id: type_id,
        project_id: Some(project_id),
        organization_id: org_id1,
        level: adam_domain::AssetLevel::Project,
        external_ref: "https://example.com/asset/1".to_string(),
        source: "manual".to_string(),
        metadata: serde_json::json!({}),
        idempotency_key: None,
    };
    let _ = repo.create(&cmd1).await.unwrap();

    // Create asset in org2
    let cmd2 = CreateAssetCommand {
        name: "Org2 Asset".to_string(),
        asset_type_id: type_id,
        project_id: None,
        organization_id: org_id2,
        level: adam_domain::AssetLevel::Organization,
        external_ref: "https://example.com/asset/2".to_string(),
        source: "manual".to_string(),
        metadata: serde_json::json!({}),
        idempotency_key: None,
    };
    let _ = repo.create(&cmd2).await.unwrap();

    // Find by org_id1
    let org1_assets = repo.find_by_organization_id(&org_id1).await.unwrap();
    assert_eq!(org1_assets.len(), 1);
    assert_eq!(org1_assets[0].name, "Org1 Asset");

    // Find by org_id2
    let org2_assets = repo.find_by_organization_id(&org_id2).await.unwrap();
    assert_eq!(org2_assets.len(), 1);
    assert_eq!(org2_assets[0].name, "Org2 Asset");
}

#[tokio::test]
async fn dirty_queue_multiple_unresolved_same_upstream() {
    let repo = InMemoryDirtyQueueRepository::new();
    let asset1 = AssetId::new();
    let asset2 = AssetId::new();
    let upstream_id = AssetId::new();

    // Both asset1 and asset2 depend on the same upstream
    let entry1 = DirtyQueueEntry {
        id: uuid::Uuid::new_v4(),
        asset_id: asset1,
        upstream_asset_id: upstream_id,
        upstream_version: "2.0.0".to_string(),
        upstream_old_version: "1.0.0".to_string(),
        impact_level: "high".to_string(),
        since: Utc::now(),
        created_at: Utc::now(),
        resolved_at: None,
    };

    let entry2 = DirtyQueueEntry {
        id: uuid::Uuid::new_v4(),
        asset_id: asset2,
        upstream_asset_id: upstream_id,
        upstream_version: "2.0.0".to_string(),
        upstream_old_version: "1.0.0".to_string(),
        impact_level: "high".to_string(),
        since: Utc::now(),
        created_at: Utc::now(),
        resolved_at: None,
    };

    repo.upsert(&entry1).await.unwrap();
    repo.upsert(&entry2).await.unwrap();

    // Both should be in unresolved
    let all_unresolved = repo.find_all_unresolved().await.unwrap();
    assert_eq!(all_unresolved.len(), 2);

    // Verify each asset has its own entry
    let unresolved1 = repo.find_unresolved_by_asset(&asset1).await.unwrap();
    assert_eq!(unresolved1.len(), 1);

    let unresolved2 = repo.find_unresolved_by_asset(&asset2).await.unwrap();
    assert_eq!(unresolved2.len(), 1);
}
