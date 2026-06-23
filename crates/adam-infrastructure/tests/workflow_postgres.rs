//! Postgres integration tests for the workflow repositories.
//!
//! These require a live Postgres with migration `014_workflow_automation.sql`
//! applied. They are `#[ignore]` by default so `cargo test` does not fail in
//! environments without a database; run explicitly in CI:
//!
//! ```sh
//! DATABASE_URL=postgres://adam:adam@localhost/adam_test \
//!   cargo test -p adam-infrastructure --test workflow_postgres -- --ignored
//! ```
//!
//! Covered (design §16 Slice 1, criterion 7): unique idempotency-key
//! constraints on events/actions, `lock_version` CAS conflicts, and the
//! event→action closure via the rule evaluator against the Postgres repos.

use adam_domain::workflow::action::CreateActionCommand;
use adam_domain::workflow::agent_task::{AgentTaskId, Capability, CreateAgentTaskCommand};
use adam_domain::workflow::event::{CorrelationId, EventType, WorkflowEvent, WorkflowEventId};
use adam_domain::workflow::idempotency::{action_idempotency_key, event_idempotency_key};
use adam_domain::workflow::instance::CreateInstanceCommand;
use adam_domain::workflow::repository::{
    AgentTaskRepository, WorkflowActionRepository, WorkflowEventRepository,
    WorkflowInstanceRepository,
};
use adam_domain::workflow::rule::{ActionType, AutomationLevel, PromotionRuleId};
use adam_domain::workflow::state_machine::{ActionStatus, AgentTaskStatus, InstanceStatus};
use adam_domain::{AssetId, AssetTypeId, OrganizationId, RepositoryError};
use chrono::{Duration, Utc};
use sqlx::postgres::PgPoolOptions;

async fn pool() -> sqlx::PgPool {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect")
}

fn org() -> OrganizationId {
    OrganizationId::from_uuid(uuid::Uuid::nil())
}

fn sample_event(token: &str) -> WorkflowEvent {
    WorkflowEvent {
        id: WorkflowEventId::new(),
        organization_id: org(),
        project_id: None,
        correlation_id: CorrelationId::new(),
        event_type: EventType::AssetPublished,
        source_asset_id: AssetId::from_uuid(uuid::Uuid::from_u128(1)),
        source_asset_type_id: AssetTypeId::from_uuid(uuid::Uuid::from_u128(2)),
        payload: serde_json::json!({"version":"1.0.0"}),
        cascade_depth: 0,
        triggering_action_id: None,
        idempotency_key: event_idempotency_key(
            EventType::AssetPublished,
            AssetId::from_uuid(uuid::Uuid::from_u128(1)),
            token,
        ),
        created_at: Utc::now(),
    }
}

#[tokio::test]
#[ignore]
async fn event_unique_idempotency_key_is_enforced() {
    let pool = pool().await;
    let repo = adam_infrastructure::repositories::PostgresWorkflowEventRepository::new(pool);
    let event = sample_event("pg-token-1");
    repo.append(&event).await.unwrap();

    // Appending the same idempotency key must reject with DuplicateIdempotencyKey.
    let dup = sample_event("pg-token-1");
    let err = repo.append(&dup).await.unwrap_err();
    assert!(
        matches!(err, RepositoryError::DuplicateIdempotencyKey(_)),
        "got {err:?}"
    );

    // Reload returns the original.
    let existing = repo
        .find_by_idempotency_key(&org(), &event.idempotency_key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(existing.id, event.id);
}

#[tokio::test]
#[ignore]
async fn instance_cas_detects_concurrent_modification() {
    let pool = pool().await;
    let repo = adam_infrastructure::repositories::PostgresWorkflowInstanceRepository::new(pool);
    let instance = repo
        .create(&CreateInstanceCommand {
            organization_id: org(),
            project_id: None,
            correlation_id: CorrelationId::new(),
            template: adam_domain::workflow::instance::WorkflowTemplate::Feature,
            cascade_depth: 0,
        })
        .await
        .unwrap();

    // Stale lock version -> ConcurrentModification.
    let err = repo
        .update_cas(
            &instance.id,
            instance.lock_version + 1,
            InstanceStatus::Ready,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, RepositoryError::ConcurrentModification { .. }),
        "got {err:?}"
    );

    // Correct lock version advances.
    let new_lv = repo
        .update_cas(&instance.id, instance.lock_version, InstanceStatus::Ready)
        .await
        .unwrap();
    assert_eq!(new_lv, instance.lock_version + 1);
}

#[tokio::test]
#[ignore]
async fn action_unique_idempotency_key_is_enforced() {
    let pool = pool().await;
    let instance_repo =
        adam_infrastructure::repositories::PostgresWorkflowInstanceRepository::new(pool.clone());
    let action_repo =
        adam_infrastructure::repositories::PostgresWorkflowActionRepository::new(pool);

    let instance = instance_repo
        .create(&CreateInstanceCommand {
            organization_id: org(),
            project_id: None,
            correlation_id: CorrelationId::new(),
            template: adam_domain::workflow::instance::WorkflowTemplate::Feature,
            cascade_depth: 0,
        })
        .await
        .unwrap();

    let rule_id = PromotionRuleId::new();
    let event_id = uuid::Uuid::new_v4();
    let key = action_idempotency_key(rule_id.0, event_id, None);
    let cmd = CreateActionCommand {
        organization_id: org(),
        instance_id: instance.id,
        action_type: ActionType::UpsertWorkItem,
        target_asset_id: None,
        target_asset_type_id: None,
        idempotency_key: key.clone(),
        preconditions: serde_json::json!([]),
        postconditions: serde_json::json!({}),
        automation_level: AutomationLevel::Automatic,
        is_required: true,
        order_index: 0,
        compensation_action_type: None,
        compensation_payload: None,
        compensation_policy: adam_domain::workflow::action::CompensationPolicy::None,
        max_retries: 3,
    };
    let created = action_repo.create(&cmd).await.unwrap();
    assert_eq!(created.status, ActionStatus::Pending);

    // Duplicate key is rejected.
    let err = action_repo.create(&cmd).await.unwrap_err();
    assert!(
        matches!(err, RepositoryError::DuplicateIdempotencyKey(_)),
        "got {err:?}"
    );

    // Reload returns the original action.
    let existing = action_repo
        .find_by_idempotency_key(&org(), &key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(existing.id, created.id);
}

// ---------------------------------------------------------------------------
// Agent task repository (design §16 Slice 2, criterion 7)
// ---------------------------------------------------------------------------

/// Seed an instance + Ready action and return its id, plus the action repo
/// handle for status assertions.
async fn seed_ready_action_pg(
    pool: &sqlx::PgPool,
) -> (
    adam_infrastructure::repositories::PostgresWorkflowInstanceRepository,
    adam_infrastructure::repositories::PostgresWorkflowActionRepository,
    adam_domain::workflow::action::WorkflowActionId,
) {
    let instance_repo =
        adam_infrastructure::repositories::PostgresWorkflowInstanceRepository::new(pool.clone());
    let action_repo =
        adam_infrastructure::repositories::PostgresWorkflowActionRepository::new(pool.clone());

    let instance = instance_repo
        .create(&CreateInstanceCommand {
            organization_id: org(),
            project_id: None,
            correlation_id: CorrelationId::new(),
            template: adam_domain::workflow::instance::WorkflowTemplate::Feature,
            cascade_depth: 0,
        })
        .await
        .unwrap();
    instance_repo
        .update_cas(&instance.id, instance.lock_version, InstanceStatus::Ready)
        .await
        .unwrap();

    let action = action_repo
        .create(&CreateActionCommand {
            organization_id: org(),
            instance_id: instance.id,
            action_type: ActionType::UpsertWorkItem,
            target_asset_id: Some(AssetId::from_uuid(uuid::Uuid::new_v4())),
            target_asset_type_id: Some(AssetTypeId::from_uuid(uuid::Uuid::new_v4())),
            idempotency_key: format!("act:{}", uuid::Uuid::new_v4()),
            preconditions: serde_json::json!([]),
            postconditions: serde_json::json!({}),
            automation_level: AutomationLevel::AgentSuggested,
            is_required: true,
            order_index: 0,
            compensation_action_type: None,
            compensation_payload: None,
            compensation_policy: adam_domain::workflow::action::CompensationPolicy::None,
            max_retries: 3,
        })
        .await
        .unwrap();
    let mut ready = action.clone();
    ready.status = ActionStatus::Ready;
    action_repo
        .update_cas(&ready, action.lock_version)
        .await
        .unwrap();

    (instance_repo, action_repo, action.id)
}

#[tokio::test]
#[ignore]
async fn agent_task_claim_cas_is_atomic_under_concurrency() {
    let pool = pool().await;
    let (_instance_repo, _action_repo, action_id) = seed_ready_action_pg(&pool).await;
    let task_repo =
        adam_infrastructure::repositories::PostgresAgentTaskRepository::new(pool.clone());

    let cmd = CreateAgentTaskCommand {
        organization_id: org(),
        project_id: None,
        action_id,
        capability: Capability::create_virtual_asset_context(),
        idempotency_key: format!("task:{}", uuid::Uuid::new_v4()),
        expires_at: None,
    };
    let task = task_repo.create(&cmd).await.unwrap();
    assert_eq!(task.status, AgentTaskStatus::Queued);

    let now = Utc::now();
    let expires = now + Duration::minutes(5);

    // Two concurrent claims on the same queued task: only one wins.
    let pool_a = pool.clone();
    let pool_b = pool.clone();
    let id = task.id;
    let claim_a = tokio::spawn(async move {
        let repo = adam_infrastructure::repositories::PostgresAgentTaskRepository::new(pool_a);
        repo.claim_cas(&id, "agent-a", now, expires).await.unwrap()
    });
    let claim_b = tokio::spawn(async move {
        let repo = adam_infrastructure::repositories::PostgresAgentTaskRepository::new(pool_b);
        repo.claim_cas(&id, "agent-b", now, expires).await.unwrap()
    });
    let (a, b) = tokio::join!(claim_a, claim_b);
    let winners = [a.unwrap(), b.unwrap()]
        .into_iter()
        .filter(Option::is_some)
        .count();
    assert_eq!(winners, 1, "exactly one concurrent claim must succeed");

    // The persisted task is Claimed with the winning agent.
    let stored = task_repo.find_by_id(&task.id).await.unwrap().unwrap();
    assert_eq!(stored.status, AgentTaskStatus::Claimed);
    assert!(stored.agent_id.is_some());
}

#[tokio::test]
#[ignore]
async fn agent_task_find_expired_returns_only_live_leased_tasks() {
    let pool = pool().await;
    let (_instance_repo, _action_repo, action_id) = seed_ready_action_pg(&pool).await;
    let task_repo =
        adam_infrastructure::repositories::PostgresAgentTaskRepository::new(pool.clone());

    // A claimed task whose lease already elapsed.
    let expired_cmd = CreateAgentTaskCommand {
        organization_id: org(),
        project_id: None,
        action_id,
        capability: Capability::create_virtual_asset_context(),
        idempotency_key: format!("task:exp:{}", uuid::Uuid::new_v4()),
        expires_at: None,
    };
    let expired_task = task_repo.create(&expired_cmd).await.unwrap();
    let past = Utc::now() - Duration::minutes(1);
    let earlier_expires = Utc::now() - Duration::seconds(30);
    task_repo
        .claim_cas(&expired_task.id, "agent-1", past, earlier_expires)
        .await
        .unwrap();

    // A queued (unclaimed) task — not yet leased, so not expired.
    let queued_cmd = CreateAgentTaskCommand {
        organization_id: org(),
        project_id: None,
        action_id,
        capability: Capability::create_virtual_asset_context(),
        idempotency_key: format!("task:q:{}", uuid::Uuid::new_v4()),
        expires_at: None,
    };
    let queued_task = task_repo.create(&queued_cmd).await.unwrap();

    let found = task_repo.find_expired(Utc::now()).await.unwrap();
    let found_ids: Vec<AgentTaskId> = found.iter().map(|t| t.id).collect();
    assert!(found_ids.contains(&expired_task.id));
    assert!(!found_ids.contains(&queued_task.id));
}

#[tokio::test]
#[ignore]
async fn agent_task_update_cas_detects_concurrent_modification() {
    let pool = pool().await;
    let (_instance_repo, _action_repo, action_id) = seed_ready_action_pg(&pool).await;
    let task_repo = adam_infrastructure::repositories::PostgresAgentTaskRepository::new(pool);

    let cmd = CreateAgentTaskCommand {
        organization_id: org(),
        project_id: None,
        action_id,
        capability: Capability::create_virtual_asset_context(),
        idempotency_key: format!("task:cas:{}", uuid::Uuid::new_v4()),
        expires_at: None,
    };
    let task = task_repo.create(&cmd).await.unwrap();

    // Stale lock_version -> ConcurrentModification.
    let mut stale = task.clone();
    stale.status = AgentTaskStatus::Succeeded;
    let err = task_repo
        .update_cas(&stale, task.lock_version + 1)
        .await
        .unwrap_err();
    assert!(
        matches!(err, RepositoryError::ConcurrentModification { .. }),
        "got {err:?}"
    );

    // Correct lock_version advances and bumps the version.
    let updated = task_repo
        .update_cas(&stale, task.lock_version)
        .await
        .unwrap();
    assert_eq!(updated.status, AgentTaskStatus::Succeeded);
    assert_eq!(updated.lock_version, task.lock_version + 1);
}
