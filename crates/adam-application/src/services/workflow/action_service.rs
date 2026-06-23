//! Workflow action service — action state transitions (Slice 1).
//!
//! Drives a [`WorkflowAction`] through the [`ActionStatus`] state machine with
//! CAS optimistic locking, evaluates preconditions/postconditions at the
//! service boundary, and emits a result event (`ActionSucceeded`/`ActionFailed`)
//! that carries the action's correlation id so the full chain is reconstructable.

use std::sync::Arc;

use adam_domain::workflow::WorkflowError;
use adam_domain::workflow::action::{WorkflowAction, WorkflowActionId};
use adam_domain::workflow::event::{CorrelationId, EventType, WorkflowEvent, WorkflowEventId};
use adam_domain::workflow::repository::{
    WorkflowActionRepository, WorkflowEventRepository, WorkflowInstanceRepository,
};
use adam_domain::workflow::state_machine::{ActionStatus, StateMachine};
use adam_domain::{AssetId, RepositoryError};

use super::{ClockRef, SystemClock};

/// Errors raised by [`WorkflowActionService`].
#[derive(Debug, thiserror::Error)]
pub enum WorkflowActionServiceError {
    #[error("repository error: {0}")]
    Repository(RepositoryError),
    #[error("workflow error: {0}")]
    Workflow(WorkflowError),
    #[error("action not found: {0}")]
    NotFound(WorkflowActionId),
    /// A precondition/postcondition evaluated false.
    #[error("condition unmet for action {action_id}: {detail}")]
    ConditionUnmet {
        action_id: WorkflowActionId,
        detail: String,
    },
}

impl From<RepositoryError> for WorkflowActionServiceError {
    fn from(err: RepositoryError) -> Self {
        WorkflowActionServiceError::Repository(err)
    }
}

impl From<WorkflowError> for WorkflowActionServiceError {
    fn from(err: WorkflowError) -> Self {
        WorkflowActionServiceError::Workflow(err)
    }
}

/// Drives [`WorkflowAction`] state transitions.
#[derive(Clone)]
pub struct WorkflowActionService<AR, ER, IR>
where
    AR: WorkflowActionRepository + ?Sized,
    ER: WorkflowEventRepository + ?Sized,
    IR: WorkflowInstanceRepository + ?Sized,
{
    action_repo: Arc<AR>,
    event_repo: Arc<ER>,
    instance_repo: Arc<IR>,
    clock: ClockRef,
}

impl<AR, ER, IR> WorkflowActionService<AR, ER, IR>
where
    AR: WorkflowActionRepository + ?Sized,
    ER: WorkflowEventRepository + ?Sized,
    IR: WorkflowInstanceRepository + ?Sized,
{
    /// Create a new service.
    pub fn new(action_repo: Arc<AR>, event_repo: Arc<ER>, instance_repo: Arc<IR>) -> Self {
        Self {
            action_repo,
            event_repo,
            instance_repo,
            clock: Arc::new(SystemClock),
        }
    }

    /// Create a new service with a custom clock (for tests).
    pub fn with_clock(
        action_repo: Arc<AR>,
        event_repo: Arc<ER>,
        instance_repo: Arc<IR>,
        clock: ClockRef,
    ) -> Self {
        Self {
            action_repo,
            event_repo,
            instance_repo,
            clock,
        }
    }

    /// Transition an action to `target` using CAS.
    ///
    /// Precondition: for `Succeeded`, every key in the action's
    /// `postconditions` must be present and equal in `result`. Illegal
    /// transitions return [`WorkflowError::IllegalTransition`] and emit no
    /// event.
    pub async fn transition(
        &self,
        action_id: WorkflowActionId,
        target: ActionStatus,
        result: Option<&serde_json::Value>,
    ) -> Result<WorkflowAction, WorkflowActionServiceError> {
        let action = self
            .action_repo
            .find_by_id(&action_id)
            .await?
            .ok_or(WorkflowActionServiceError::NotFound(action_id))?;

        action.status.validate_transition(target)?;

        if target == ActionStatus::Succeeded {
            self.check_postconditions(&action, result)?;
        }

        let mut updated = action.clone();
        updated.status = target;
        if let Some(r) = result {
            updated.result_payload = Some(r.clone());
        }
        let stored = self
            .action_repo
            .update_cas(&updated, action.lock_version)
            .await?;

        // Emit a result event sharing the action's correlation id. The
        // correlation id is recovered from the parent workflow instance so the
        // full event→action→result chain is reconstructable via correlation_id.
        let event_type = if target == ActionStatus::Succeeded {
            EventType::ActionSucceeded
        } else if target == ActionStatus::Failed {
            EventType::ActionFailed
        } else {
            // Non-terminal or non-result transitions emit no result event.
            return Ok(stored);
        };
        let correlation_id = self.resolve_correlation_id(&stored).await;
        self.emit_result_event(&stored, event_type, correlation_id)
            .await?;
        Ok(stored)
    }

    /// Resolve the correlation id for a result event from the action's parent
    /// workflow instance. Falls back to a deterministic id derived from the
    /// action when the instance cannot be loaded, so the event is still emitted
    /// (best effort) rather than dropped.
    async fn resolve_correlation_id(&self, action: &WorkflowAction) -> CorrelationId {
        if let Ok(Some(instance)) = self.instance_repo.find_by_id(&action.instance_id).await {
            return instance.correlation_id;
        }
        CorrelationId::from_uuid(action.id.0)
    }

    fn check_postconditions(
        &self,
        action: &WorkflowAction,
        result: Option<&serde_json::Value>,
    ) -> Result<(), WorkflowActionServiceError> {
        let Some(conds) = action.postconditions.as_object() else {
            return Ok(());
        };
        if conds.is_empty() {
            return Ok(());
        }
        let Some(result_obj) = result.and_then(|v| v.as_object()) else {
            return Err(WorkflowActionServiceError::ConditionUnmet {
                action_id: action.id,
                detail: "result payload missing or not an object".to_string(),
            });
        };
        for (k, expected) in conds {
            if result_obj.get(k) != Some(expected) {
                return Err(WorkflowActionServiceError::ConditionUnmet {
                    action_id: action.id,
                    detail: format!("postcondition `{k}` not satisfied"),
                });
            }
        }
        Ok(())
    }

    async fn emit_result_event(
        &self,
        action: &WorkflowAction,
        event_type: EventType,
        correlation_id: CorrelationId,
    ) -> Result<WorkflowEvent, WorkflowActionServiceError> {
        let event = WorkflowEvent {
            id: WorkflowEventId::new(),
            organization_id: action.organization_id,
            project_id: None,
            // Propagate the parent instance's correlation id so the
            // event→action→result chain is reconstructable via correlation_id.
            correlation_id,
            event_type,
            source_asset_id: action
                .target_asset_id
                .unwrap_or(AssetId::from_uuid(action.id.0)),
            source_asset_type_id: adam_domain::AssetTypeId::from_uuid(uuid::Uuid::nil()),
            payload: serde_json::json!({
                "action_id": action.id.0.to_string(),
                "action_type": action.action_type.as_str(),
            }),
            cascade_depth: 0,
            triggering_action_id: Some(action.id.0),
            idempotency_key: format!("action:{}:{}", action.id.0, event_type.as_str()),
            created_at: self.clock.now(),
        };
        let stored = self.event_repo.append(&event).await?;
        Ok(stored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adam_domain::workflow::action::CreateActionCommand;
    use adam_domain::workflow::in_memory::{
        InMemoryWorkflowActionRepository, InMemoryWorkflowEventRepository,
        InMemoryWorkflowInstanceRepository,
    };
    use adam_domain::workflow::instance::CreateInstanceCommand;
    use adam_domain::workflow::repository::WorkflowInstanceRepository;
    use adam_domain::workflow::rule::ActionType;
    use adam_domain::{AssetId, AssetTypeId, OrganizationId};
    use chrono::{TimeZone, Utc};

    struct FixedClock;
    impl super::super::Clock for FixedClock {
        fn now(&self) -> chrono::DateTime<chrono::Utc> {
            Utc.with_ymd_and_hms(2026, 6, 18, 9, 0, 0).unwrap()
        }
    }

    async fn seed_action(
        repo: &InMemoryWorkflowActionRepository,
        instance_repo: &InMemoryWorkflowInstanceRepository,
        postconditions: serde_json::Value,
    ) -> WorkflowAction {
        let instance = instance_repo
            .create(&CreateInstanceCommand {
                organization_id: OrganizationId::from_uuid(uuid::Uuid::nil()),
                project_id: None,
                correlation_id: CorrelationId::from_uuid(uuid::Uuid::nil()),
                template: adam_domain::workflow::instance::WorkflowTemplate::Feature,
                cascade_depth: 1,
            })
            .await
            .unwrap();
        repo.create(&CreateActionCommand {
            organization_id: OrganizationId::from_uuid(uuid::Uuid::nil()),
            instance_id: instance.id,
            action_type: ActionType::UpsertWorkItem,
            target_asset_id: Some(AssetId::from_uuid(uuid::Uuid::from_u128(9))),
            target_asset_type_id: Some(AssetTypeId::from_uuid(uuid::Uuid::from_u128(2))),
            idempotency_key: format!("act:{}", uuid::Uuid::new_v4()),
            preconditions: serde_json::json!([]),
            postconditions,
            automation_level: adam_domain::workflow::rule::AutomationLevel::Automatic,
            is_required: true,
            order_index: 0,
            compensation_action_type: None,
            compensation_payload: None,
            compensation_policy: adam_domain::workflow::action::CompensationPolicy::None,
            max_retries: 3,
        })
        .await
        .unwrap()
    }

    type TestSvc = WorkflowActionService<
        InMemoryWorkflowActionRepository,
        InMemoryWorkflowEventRepository,
        InMemoryWorkflowInstanceRepository,
    >;

    fn svc() -> (
        TestSvc,
        Arc<InMemoryWorkflowActionRepository>,
        Arc<InMemoryWorkflowEventRepository>,
        Arc<InMemoryWorkflowInstanceRepository>,
    ) {
        let action_repo = Arc::new(InMemoryWorkflowActionRepository::default());
        let event_repo = Arc::new(InMemoryWorkflowEventRepository::default());
        let instance_repo = Arc::new(InMemoryWorkflowInstanceRepository::default());
        let svc = WorkflowActionService::with_clock(
            action_repo.clone(),
            event_repo.clone(),
            instance_repo.clone(),
            Arc::new(FixedClock),
        );
        (svc, action_repo, event_repo, instance_repo)
    }

    #[tokio::test]
    async fn illegal_transition_returns_error_and_emits_no_event() {
        let (svc, action_repo, event_repo, instance_repo) = svc();
        let action = seed_action(&action_repo, &instance_repo, serde_json::json!({})).await;

        // Pending -> Succeeded is illegal (must go through Ready/InProgress).
        let err = svc
            .transition(action.id, ActionStatus::Succeeded, None)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            WorkflowActionServiceError::Workflow(WorkflowError::IllegalTransition { .. })
        ));
        // Result events are correlated with the parent instance (Uuid::nil()).
        assert_eq!(
            event_repo
                .find_by_correlation_id(&CorrelationId::from_uuid(uuid::Uuid::nil()))
                .await
                .unwrap()
                .len(),
            0
        );
    }

    #[tokio::test]
    async fn success_emits_succeeded_event_when_postconditions_met() {
        let (svc, action_repo, event_repo, instance_repo) = svc();
        let action = seed_action(
            &action_repo,
            &instance_repo,
            serde_json::json!({"work_item_kind":"feature"}),
        )
        .await;

        // Pending -> Ready -> InProgress -> Succeeded.
        svc.transition(action.id, ActionStatus::Ready, None)
            .await
            .unwrap();
        svc.transition(action.id, ActionStatus::InProgress, None)
            .await
            .unwrap();
        let result = serde_json::json!({"work_item_kind":"feature","id":"wi-1"});
        let done = svc
            .transition(action.id, ActionStatus::Succeeded, Some(&result))
            .await
            .unwrap();
        assert_eq!(done.status, ActionStatus::Succeeded);

        // The result event must be reachable via the parent instance's
        // correlation id (Uuid::nil()), proving the correlation chain.
        let events = event_repo
            .find_by_correlation_id(&CorrelationId::from_uuid(uuid::Uuid::nil()))
            .await
            .unwrap();
        assert!(
            events
                .iter()
                .any(|e| e.event_type == EventType::ActionSucceeded)
        );
    }

    #[tokio::test]
    async fn failed_postcondition_blocks_success_without_event() {
        let (svc, action_repo, event_repo, instance_repo) = svc();
        let action = seed_action(
            &action_repo,
            &instance_repo,
            serde_json::json!({"work_item_kind":"feature"}),
        )
        .await;
        svc.transition(action.id, ActionStatus::Ready, None)
            .await
            .unwrap();
        svc.transition(action.id, ActionStatus::InProgress, None)
            .await
            .unwrap();

        let result = serde_json::json!({"work_item_kind":"bug"}); // wrong kind
        let err = svc
            .transition(action.id, ActionStatus::Succeeded, Some(&result))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            WorkflowActionServiceError::ConditionUnmet { .. }
        ));
        assert_eq!(
            event_repo
                .find_by_correlation_id(&CorrelationId::from_uuid(uuid::Uuid::nil()))
                .await
                .unwrap()
                .len(),
            0
        );
    }
}
