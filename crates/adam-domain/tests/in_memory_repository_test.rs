//! Integration tests for in-memory repositories

use adam_domain::repository::{
    AssetRepository, CreateAssetCommand, DirtyQueueEntry, DirtyQueueRepository, RepositoryError,
};
use adam_domain::{
    AssetId, AssetState, AssetTypeId, InMemoryAssetRepository, InMemoryDirtyQueueRepository,
    OrganizationId, ProjectId,
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
    assert_eq!(asset.current_state, AssetState::Clean);

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
    assert_eq!(asset.current_state, AssetState::Clean);

    repo.update_state(&asset.id, AssetState::Dirty)
        .await
        .unwrap();

    let updated = repo.find_by_id(&asset.id).await.unwrap().unwrap();
    assert_eq!(updated.current_state, AssetState::Dirty);
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
        created_at: Utc::now(),
        resolved_at: None,
    };

    repo.upsert(&entry1).await.unwrap();

    let entry2 = DirtyQueueEntry {
        id: uuid::Uuid::new_v4(),
        asset_id,
        upstream_asset_id: upstream_id,
        upstream_version: "2.0.0".to_string(),
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
        created_at: Utc::now(),
        resolved_at: None,
    };

    let entry2 = DirtyQueueEntry {
        id: uuid::Uuid::new_v4(),
        asset_id: asset_id2,
        upstream_asset_id: upstream_id,
        upstream_version: "1.0.0".to_string(),
        created_at: Utc::now(),
        resolved_at: None,
    };

    repo.upsert(&entry1).await.unwrap();
    repo.upsert(&entry2).await.unwrap();

    let all_unresolved = repo.find_all_unresolved().await.unwrap();
    assert_eq!(all_unresolved.len(), 2);
}
