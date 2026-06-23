//! Tests for [`super::PromotionRuleEvaluator`], kept in a separate file to
//! keep the evaluator module under the file-size budget.

use super::*;
use adam_domain::workflow::event::{CorrelationId, EventType, WorkflowEvent, WorkflowEventId};
use adam_domain::workflow::in_memory::{
    InMemoryPromotionRuleRepository, InMemoryWorkflowActionRepository,
    InMemoryWorkflowInstanceRepository,
};
use adam_domain::workflow::repository::{WorkflowActionRepository, WorkflowInstanceRepository};
use adam_domain::workflow::rule::{
    ActionTemplate, ActionType, AutomationLevel, MutexGroup, PromotionRule, PromotionRuleId,
    RuleScope,
};
use adam_domain::workflow::state_machine::InstanceStatus;
use adam_domain::{AssetId, AssetTypeId, OrganizationId};
use chrono::{TimeZone, Utc};

struct FixedClock;
impl super::super::Clock for FixedClock {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        Utc.with_ymd_and_hms(2026, 6, 18, 9, 0, 0).unwrap()
    }
}

fn clock() -> ClockRef {
    Arc::new(FixedClock)
}

type TestEvaluator = PromotionRuleEvaluator<
    InMemoryPromotionRuleRepository,
    InMemoryWorkflowInstanceRepository,
    InMemoryWorkflowActionRepository,
>;

fn evaluator() -> (TestEvaluator, Arc<InMemoryPromotionRuleRepository>) {
    let rule_repo = Arc::new(InMemoryPromotionRuleRepository::default());
    let instance_repo = Arc::new(InMemoryWorkflowInstanceRepository::default());
    let action_repo = Arc::new(InMemoryWorkflowActionRepository::default());
    let svc =
        PromotionRuleEvaluator::with_clock(rule_repo.clone(), instance_repo, action_repo, clock());
    (svc, rule_repo)
}

/// Like [`evaluator`] but also returns the instance/action repos so tests can
/// assert side effects on instances (e.g. orphan cancellation).
fn evaluator_with_repos() -> (
    TestEvaluator,
    Arc<InMemoryPromotionRuleRepository>,
    Arc<InMemoryWorkflowInstanceRepository>,
    Arc<InMemoryWorkflowActionRepository>,
) {
    let rule_repo = Arc::new(InMemoryPromotionRuleRepository::default());
    let instance_repo = Arc::new(InMemoryWorkflowInstanceRepository::default());
    let action_repo = Arc::new(InMemoryWorkflowActionRepository::default());
    let svc = PromotionRuleEvaluator::with_clock(
        rule_repo.clone(),
        instance_repo.clone(),
        action_repo.clone(),
        clock(),
    );
    (svc, rule_repo, instance_repo, action_repo)
}

fn feature_rule(id: u128, scope: RuleScope, max_depth: i32) -> PromotionRule {
    PromotionRule {
        id: PromotionRuleId::from_uuid(uuid::Uuid::from_u128(id)),
        organization_id: OrganizationId::from_uuid(uuid::Uuid::nil()),
        scope,
        scope_ref: None,
        event_type: EventType::AssetPublished,
        source_asset_type_id: None,
        mutex_group: None,
        rule_version: 1,
        priority: 0,
        automation_level: AutomationLevel::Automatic,
        filters: serde_json::json!({}),
        preconditions: serde_json::json!([]),
        action_template: ActionTemplate {
            action_type: ActionType::UpsertWorkItem,
            payload: serde_json::json!({"work_item_kind":"feature"}),
            is_required: true,
            order_index: 0,
        },
        max_cascade_depth: max_depth,
        effective_from: None,
        effective_to: None,
        rollout_segment: 100,
        enabled: true,
        dry_run: false,
        audit_only: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn published_event(cascade_depth: i32) -> WorkflowEvent {
    WorkflowEvent {
        id: WorkflowEventId::from_uuid(uuid::Uuid::from_u128(100)),
        organization_id: OrganizationId::from_uuid(uuid::Uuid::nil()),
        project_id: None,
        correlation_id: CorrelationId::from_uuid(uuid::Uuid::nil()),
        event_type: EventType::AssetPublished,
        source_asset_id: AssetId::from_uuid(uuid::Uuid::from_u128(7)),
        source_asset_type_id: AssetTypeId::from_uuid(uuid::Uuid::from_u128(2)),
        payload: serde_json::json!({"version":"1.0.0"}),
        cascade_depth,
        triggering_action_id: None,
        idempotency_key: "evt:req-1".to_string(),
        created_at: Utc::now(),
    }
}

#[tokio::test]
async fn creates_action_for_matching_rule() {
    let (svc, rule_repo) = evaluator();
    rule_repo
        .create(&feature_rule(1, RuleScope::Organization, 5))
        .await
        .unwrap();

    let outcome = svc.evaluate(&published_event(0)).await.unwrap();
    assert_eq!(outcome.created.len(), 1);
    assert!(!outcome.created[0].reused);
    assert_eq!(outcome.cascade_exceeded.len(), 0);
}

#[tokio::test]
async fn replaying_same_event_does_not_create_duplicate_action() {
    let (svc, rule_repo) = evaluator();
    rule_repo
        .create(&feature_rule(1, RuleScope::Organization, 5))
        .await
        .unwrap();

    let event = published_event(0);
    let first = svc.evaluate(&event).await.unwrap();
    let second = svc.evaluate(&event).await.unwrap();

    assert_eq!(first.created.len(), 1);
    assert_eq!(second.created.len(), 1);
    assert_eq!(
        first.created[0].action_id, second.created[0].action_id,
        "replay must reuse the same action"
    );
    assert!(
        second.created[0].reused,
        "second evaluation must mark the action as reused"
    );
}

#[tokio::test]
async fn cascade_depth_exceeded_records_violation_without_action() {
    let (svc, rule_repo) = evaluator();
    // max_cascade_depth = 2, event depth = 2 -> exceeded (depth >= max).
    rule_repo
        .create(&feature_rule(1, RuleScope::Organization, 2))
        .await
        .unwrap();

    let outcome = svc.evaluate(&published_event(2)).await.unwrap();
    assert_eq!(outcome.created.len(), 0, "no action on cascade overflow");
    assert_eq!(outcome.cascade_exceeded.len(), 1);
    assert_eq!(outcome.cascade_exceeded[0].depth, 2);
    assert_eq!(outcome.cascade_exceeded[0].max, 2);
}

#[tokio::test]
async fn conflicting_rules_resolve_to_single_winner() {
    let (svc, rule_repo) = evaluator();
    // Two organization rules in the same mutex group: higher version wins.
    let mut lower = feature_rule(1, RuleScope::Organization, 5);
    lower.mutex_group = Some(MutexGroup::new("work_item"));
    lower.rule_version = 1;
    let mut higher = feature_rule(2, RuleScope::Organization, 5);
    higher.mutex_group = Some(MutexGroup::new("work_item"));
    higher.rule_version = 3;

    rule_repo.create(&lower).await.unwrap();
    rule_repo.create(&higher).await.unwrap();

    let outcome = svc.evaluate(&published_event(0)).await.unwrap();
    assert_eq!(
        outcome.created.len(),
        1,
        "mutex group keeps a single winner"
    );
    assert_eq!(outcome.created[0].rule_id, higher.id.0);
    assert_eq!(outcome.evaluated.len(), 2);
}

#[tokio::test]
async fn payload_filter_excludes_non_matching_rule() {
    let (svc, rule_repo) = evaluator();
    let mut rule = feature_rule(1, RuleScope::Organization, 5);
    rule.filters = serde_json::json!({"kind": "epic"});

    rule_repo.create(&rule).await.unwrap();

    // Event payload has no `kind: epic` -> rule filtered out.
    let outcome = svc.evaluate(&published_event(0)).await.unwrap();
    assert_eq!(outcome.created.len(), 0);
}

#[tokio::test]
async fn concurrent_duplicate_action_cancels_orphan_instance() {
    use adam_domain::workflow::action::CreateActionCommand;
    use adam_domain::workflow::idempotency::action_idempotency_key;
    use adam_domain::workflow::instance::CreateInstanceCommand;

    let (svc, rule_repo, instance_repo, action_repo) = evaluator_with_repos();
    let rule = feature_rule(1, RuleScope::Organization, 5);
    rule_repo.create(&rule).await.unwrap();

    let event = published_event(0);

    // Simulate a concurrent winner: pre-create the winning instance + action
    // with the exact idempotency key the evaluator will derive, then advance
    // the winner instance to Ready (as a real winner would be).
    let winner_instance = instance_repo
        .create(&CreateInstanceCommand {
            organization_id: event.organization_id,
            project_id: event.project_id,
            correlation_id: event.correlation_id,
            template: adam_domain::workflow::instance::WorkflowTemplate::Feature,
            cascade_depth: 1,
        })
        .await
        .unwrap();
    instance_repo
        .update_cas(
            &winner_instance.id,
            winner_instance.lock_version,
            InstanceStatus::Ready,
        )
        .await
        .unwrap();
    let key = action_idempotency_key(rule.id.0, event.id.0, Some(event.source_asset_id));
    action_repo
        .create(&CreateActionCommand {
            organization_id: event.organization_id,
            instance_id: winner_instance.id,
            action_type: ActionType::UpsertWorkItem,
            target_asset_id: Some(event.source_asset_id),
            target_asset_type_id: None,
            idempotency_key: key,
            preconditions: serde_json::json!([]),
            postconditions: serde_json::json!({}),
            automation_level: AutomationLevel::Automatic,
            is_required: true,
            order_index: 0,
            compensation_action_type: None,
            compensation_payload: None,
            compensation_policy: adam_domain::workflow::action::CompensationPolicy::None,
            max_retries: 3,
        })
        .await
        .unwrap();

    // The evaluator's fast-path finds the existing action via its idempotency
    // key, so it must NOT create a new instance.
    let outcome = svc.evaluate(&event).await.unwrap();
    assert_eq!(outcome.created.len(), 1);
    assert!(
        outcome.created[0].reused,
        "concurrent winner is reused, not duplicated"
    );

    // Only the winner instance should remain non-terminal; the evaluator must
    // not have left a Ready orphan.
    let non_terminal: Vec<_> = instance_repo.find_non_terminal().await.unwrap();
    assert_eq!(non_terminal.len(), 1, "no orphan instance left behind");
    assert_eq!(non_terminal[0].id, winner_instance.id);
}
