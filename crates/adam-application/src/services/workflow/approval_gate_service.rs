//! Approval gate service (Slice 3).

use std::sync::Arc;

use adam_domain::workflow::WorkflowError;
use adam_domain::workflow::action::{WorkflowAction, WorkflowActionId};
use adam_domain::workflow::approval_gate::{
    ApprovalGate, ApprovalGateId, ApproverType, CreateApprovalGateCommand,
};
use adam_domain::workflow::event::{CorrelationId, EventType, WorkflowEvent, WorkflowEventId};
use adam_domain::workflow::repository::{
    ApprovalGateRepository, WorkflowActionRepository, WorkflowEventRepository,
    WorkflowInstanceRepository,
};
use adam_domain::workflow::state_machine::{ActionStatus, GateStatus, StateMachine};
use adam_domain::{AssetId, AssetTypeId, RepositoryError};
use chrono::{DateTime, Utc};

use super::{ClockRef, SystemClock};

/// Decision recorded on an approval gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateDecision {
    Approve,
    Reject,
}

/// Errors raised by [`ApprovalGateService`].
#[derive(Debug, thiserror::Error)]
pub enum ApprovalGateServiceError {
    #[error("repository error: {0}")]
    Repository(RepositoryError),
    #[error("workflow error: {0}")]
    Workflow(WorkflowError),
    #[error("approval gate not found: {0}")]
    GateNotFound(ApprovalGateId),
    #[error("workflow action not found: {0}")]
    ActionNotFound(WorkflowActionId),
}

impl From<RepositoryError> for ApprovalGateServiceError {
    fn from(err: RepositoryError) -> Self {
        ApprovalGateServiceError::Repository(err)
    }
}

impl From<WorkflowError> for ApprovalGateServiceError {
    fn from(err: WorkflowError) -> Self {
        ApprovalGateServiceError::Workflow(err)
    }
}

/// Coordinates human approval gates and their parent workflow actions.
#[derive(Clone)]
pub struct ApprovalGateService<GR, AR, ER, IR>
where
    GR: ApprovalGateRepository + ?Sized,
    AR: WorkflowActionRepository + ?Sized,
    ER: WorkflowEventRepository + ?Sized,
    IR: WorkflowInstanceRepository + ?Sized,
{
    gate_repo: Arc<GR>,
    action_repo: Arc<AR>,
    event_repo: Arc<ER>,
    instance_repo: Arc<IR>,
    clock: ClockRef,
}

impl<GR, AR, ER, IR> ApprovalGateService<GR, AR, ER, IR>
where
    GR: ApprovalGateRepository + ?Sized,
    AR: WorkflowActionRepository + ?Sized,
    ER: WorkflowEventRepository + ?Sized,
    IR: WorkflowInstanceRepository + ?Sized,
{
    /// Create a new service.
    pub fn new(
        gate_repo: Arc<GR>,
        action_repo: Arc<AR>,
        event_repo: Arc<ER>,
        instance_repo: Arc<IR>,
    ) -> Self {
        Self::with_clock(
            gate_repo,
            action_repo,
            event_repo,
            instance_repo,
            Arc::new(SystemClock),
        )
    }

    /// Create a new service with a custom clock.
    pub fn with_clock(
        gate_repo: Arc<GR>,
        action_repo: Arc<AR>,
        event_repo: Arc<ER>,
        instance_repo: Arc<IR>,
        clock: ClockRef,
    ) -> Self {
        Self {
            gate_repo,
            action_repo,
            event_repo,
            instance_repo,
            clock,
        }
    }

    /// Create an approval gate for a ready action and move it to waiting.
    pub async fn request_approval(
        &self,
        action_id: WorkflowActionId,
        approver_type: ApproverType,
        approver_ref: impl Into<String>,
        deadline: Option<DateTime<Utc>>,
    ) -> Result<ApprovalGate, ApprovalGateServiceError> {
        let action = self
            .action_repo
            .find_by_id(&action_id)
            .await?
            .ok_or(ApprovalGateServiceError::ActionNotFound(action_id))?;
        action
            .status
            .validate_transition(ActionStatus::WaitingApproval)?;

        let gate = self
            .gate_repo
            .create(&CreateApprovalGateCommand {
                organization_id: action.organization_id,
                action_id,
                approver_type,
                approver_ref: approver_ref.into(),
                deadline,
            })
            .await?;

        let mut waiting = action.clone();
        waiting.status = ActionStatus::WaitingApproval;
        self.action_repo
            .update_cas(&waiting, action.lock_version)
            .await?;
        Ok(gate)
    }

    /// Record an approval decision and advance the parent action.
    pub async fn record_decision(
        &self,
        gate_id: ApprovalGateId,
        decision: GateDecision,
        decided_by: impl Into<String>,
        decision_payload: serde_json::Value,
    ) -> Result<ApprovalGate, ApprovalGateServiceError> {
        let gate = self
            .gate_repo
            .find_by_id(&gate_id)
            .await?
            .ok_or(ApprovalGateServiceError::GateNotFound(gate_id))?;
        let target_gate_status = match decision {
            GateDecision::Approve => GateStatus::Approved,
            GateDecision::Reject => GateStatus::Rejected,
        };
        gate.status.validate_transition(target_gate_status)?;

        let action = self
            .action_repo
            .find_by_id(&gate.action_id)
            .await?
            .ok_or(ApprovalGateServiceError::ActionNotFound(gate.action_id))?;
        let target_action_status = match decision {
            GateDecision::Approve => ActionStatus::Ready,
            GateDecision::Reject => ActionStatus::Failed,
        };
        action.status.validate_transition(target_action_status)?;

        let mut updated_gate = gate.clone();
        updated_gate.status = target_gate_status;
        updated_gate.decision_payload = Some(decision_payload.clone());
        updated_gate.decided_by = Some(decided_by.into());
        updated_gate.decided_at = Some(self.clock.now());

        let stored_gate = self
            .gate_repo
            .update_cas(&updated_gate, gate.lock_version)
            .await?;

        let mut updated_action = action.clone();
        updated_action.status = target_action_status;
        if decision == GateDecision::Reject {
            updated_action.result_payload = Some(decision_payload);
        }
        let stored_action = self
            .action_repo
            .update_cas(&updated_action, action.lock_version)
            .await?;

        let event_type = match decision {
            GateDecision::Approve => EventType::ApprovalGranted,
            GateDecision::Reject => EventType::ApprovalRejected,
        };
        self.emit_decision_event(&stored_gate, &stored_action, event_type)
            .await?;
        Ok(stored_gate)
    }

    async fn emit_decision_event(
        &self,
        gate: &ApprovalGate,
        action: &WorkflowAction,
        event_type: EventType,
    ) -> Result<WorkflowEvent, ApprovalGateServiceError> {
        let correlation_id = self.resolve_correlation_id(action).await;
        let event = WorkflowEvent {
            id: WorkflowEventId::new(),
            organization_id: gate.organization_id,
            project_id: self.resolve_project_id(action).await,
            correlation_id,
            event_type,
            source_asset_id: action
                .target_asset_id
                .unwrap_or(AssetId::from_uuid(action.id.0)),
            source_asset_type_id: action
                .target_asset_type_id
                .unwrap_or(AssetTypeId::from_uuid(uuid::Uuid::nil())),
            payload: serde_json::json!({
                "approval_gate_id": gate.id.0.to_string(),
                "action_id": action.id.0.to_string(),
                "decision": gate.status.to_string(),
                "decided_by": gate.decided_by,
            }),
            cascade_depth: 0,
            triggering_action_id: Some(action.id.0),
            idempotency_key: format!("approval:{}:{}", gate.id.0, event_type.as_str()),
            created_at: self.clock.now(),
        };
        Ok(self.event_repo.append(&event).await?)
    }

    async fn resolve_correlation_id(&self, action: &WorkflowAction) -> CorrelationId {
        if let Ok(Some(instance)) = self.instance_repo.find_by_id(&action.instance_id).await {
            return instance.correlation_id;
        }
        CorrelationId::from_uuid(action.id.0)
    }

    async fn resolve_project_id(&self, action: &WorkflowAction) -> Option<adam_domain::ProjectId> {
        self.instance_repo
            .find_by_id(&action.instance_id)
            .await
            .ok()
            .flatten()
            .and_then(|instance| instance.project_id)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use adam_domain::workflow::approval_gate::{ApproverType, CreateApprovalGateCommand};
    use adam_domain::workflow::in_memory::{
        InMemoryApprovalGateRepository, InMemoryWorkflowActionRepository,
        InMemoryWorkflowEventRepository, InMemoryWorkflowInstanceRepository,
    };
    use adam_domain::workflow::repository::{
        ApprovalGateRepository, WorkflowActionRepository, WorkflowEventRepository,
        WorkflowInstanceRepository,
    };
    use adam_domain::workflow::state_machine::{ActionStatus, GateStatus};
    use adam_domain::workflow::{
        action::{CreateActionCommand, WorkflowAction},
        event::{CorrelationId, EventType},
        instance::{CreateInstanceCommand, WorkflowTemplate},
        rule::{ActionType, AutomationLevel},
    };
    use adam_domain::{AssetId, AssetTypeId, OrganizationId};
    use chrono::{TimeZone, Utc};

    use super::*;

    struct FixedClock;
    impl crate::services::workflow::Clock for FixedClock {
        fn now(&self) -> chrono::DateTime<chrono::Utc> {
            Utc.with_ymd_and_hms(2026, 6, 18, 9, 0, 0).unwrap()
        }
    }

    type TestSvc = ApprovalGateService<
        InMemoryApprovalGateRepository,
        InMemoryWorkflowActionRepository,
        InMemoryWorkflowEventRepository,
        InMemoryWorkflowInstanceRepository,
    >;

    async fn seed_waiting_action(
        action_repo: &InMemoryWorkflowActionRepository,
        instance_repo: &InMemoryWorkflowInstanceRepository,
    ) -> WorkflowAction {
        let instance = instance_repo
            .create(&CreateInstanceCommand {
                organization_id: OrganizationId::from_uuid(uuid::Uuid::nil()),
                project_id: None,
                correlation_id: CorrelationId::from_uuid(uuid::Uuid::from_u128(42)),
                template: WorkflowTemplate::Feature,
                cascade_depth: 0,
            })
            .await
            .unwrap();
        let mut action = action_repo
            .create(&CreateActionCommand {
                organization_id: OrganizationId::from_uuid(uuid::Uuid::nil()),
                instance_id: instance.id,
                action_type: ActionType::UpsertWorkItem,
                target_asset_id: Some(AssetId::from_uuid(uuid::Uuid::from_u128(7))),
                target_asset_type_id: Some(AssetTypeId::from_uuid(uuid::Uuid::from_u128(8))),
                idempotency_key: format!("approval-action:{}", uuid::Uuid::new_v4()),
                preconditions: serde_json::json!({}),
                postconditions: serde_json::json!({}),
                automation_level: AutomationLevel::HumanApprovalRequired,
                is_required: true,
                order_index: 0,
                compensation_action_type: None,
                compensation_payload: None,
                compensation_policy: adam_domain::workflow::CompensationPolicy::None,
                max_retries: 0,
            })
            .await
            .unwrap();
        action.status = ActionStatus::Ready;
        action = action_repo.update_cas(&action, 1).await.unwrap();
        action.status = ActionStatus::WaitingApproval;
        action_repo.update_cas(&action, 2).await.unwrap()
    }

    fn svc() -> (
        TestSvc,
        Arc<InMemoryApprovalGateRepository>,
        Arc<InMemoryWorkflowActionRepository>,
        Arc<InMemoryWorkflowEventRepository>,
        Arc<InMemoryWorkflowInstanceRepository>,
    ) {
        let gate_repo = Arc::new(InMemoryApprovalGateRepository::default());
        let action_repo = Arc::new(InMemoryWorkflowActionRepository::default());
        let event_repo = Arc::new(InMemoryWorkflowEventRepository::default());
        let instance_repo = Arc::new(InMemoryWorkflowInstanceRepository::default());
        let service = ApprovalGateService::with_clock(
            gate_repo.clone(),
            action_repo.clone(),
            event_repo.clone(),
            instance_repo.clone(),
            Arc::new(FixedClock),
        );
        (service, gate_repo, action_repo, event_repo, instance_repo)
    }

    #[tokio::test]
    async fn request_approval_blocks_ready_action_and_records_gate() {
        let (service, gate_repo, action_repo, _event_repo, instance_repo) = svc();
        let mut action = seed_waiting_action(&action_repo, &instance_repo).await;
        action.status = ActionStatus::Ready;
        let action = action_repo
            .update_cas(&action, action.lock_version)
            .await
            .unwrap();

        let gate = service
            .request_approval(action.id, ApproverType::Role, "tech_lead", None)
            .await
            .unwrap();

        assert_eq!(gate.status, GateStatus::Pending);
        assert_eq!(gate.approver_ref, "tech_lead");
        let stored_action = action_repo.find_by_id(&action.id).await.unwrap().unwrap();
        assert_eq!(stored_action.status, ActionStatus::WaitingApproval);
        assert_eq!(gate_repo.find_by_action(&action.id).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn approving_gate_records_decision_and_unblocks_action() {
        let (service, gate_repo, action_repo, event_repo, instance_repo) = svc();
        let action = seed_waiting_action(&action_repo, &instance_repo).await;
        let gate = gate_repo
            .create(&CreateApprovalGateCommand {
                organization_id: action.organization_id,
                action_id: action.id,
                approver_type: ApproverType::User,
                approver_ref: "alice".to_string(),
                deadline: None,
            })
            .await
            .unwrap();

        let approved = service
            .record_decision(
                gate.id,
                GateDecision::Approve,
                "alice",
                serde_json::json!({"reason":"looks good"}),
            )
            .await
            .unwrap();

        assert_eq!(approved.status, GateStatus::Approved);
        assert_eq!(approved.decided_by.as_deref(), Some("alice"));
        let stored_action = action_repo.find_by_id(&action.id).await.unwrap().unwrap();
        assert_eq!(stored_action.status, ActionStatus::Ready);
        let events = event_repo
            .find_by_correlation_id(&CorrelationId::from_uuid(uuid::Uuid::from_u128(42)))
            .await
            .unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == EventType::ApprovalGranted
                && event.triggering_action_id == Some(action.id.0)
        }));
    }

    #[tokio::test]
    async fn rejecting_gate_records_decision_and_fails_action() {
        let (service, gate_repo, action_repo, event_repo, instance_repo) = svc();
        let action = seed_waiting_action(&action_repo, &instance_repo).await;
        let gate = gate_repo
            .create(&CreateApprovalGateCommand {
                organization_id: action.organization_id,
                action_id: action.id,
                approver_type: ApproverType::User,
                approver_ref: "bob".to_string(),
                deadline: None,
            })
            .await
            .unwrap();

        let rejected = service
            .record_decision(
                gate.id,
                GateDecision::Reject,
                "bob",
                serde_json::json!({"reason":"missing evidence"}),
            )
            .await
            .unwrap();

        assert_eq!(rejected.status, GateStatus::Rejected);
        let stored_action = action_repo.find_by_id(&action.id).await.unwrap().unwrap();
        assert_eq!(stored_action.status, ActionStatus::Failed);
        let events = event_repo
            .find_by_correlation_id(&CorrelationId::from_uuid(uuid::Uuid::from_u128(42)))
            .await
            .unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == EventType::ApprovalRejected
                && event.triggering_action_id == Some(action.id.0)
        }));
    }

    #[tokio::test]
    async fn decided_gate_cannot_be_decided_again() {
        let (service, gate_repo, action_repo, _event_repo, instance_repo) = svc();
        let action = seed_waiting_action(&action_repo, &instance_repo).await;
        let mut gate = gate_repo
            .create(&CreateApprovalGateCommand {
                organization_id: action.organization_id,
                action_id: action.id,
                approver_type: ApproverType::User,
                approver_ref: "carol".to_string(),
                deadline: None,
            })
            .await
            .unwrap();
        gate.status = GateStatus::Approved;
        gate.decided_by = Some("carol".to_string());
        gate.decided_at = Some(Utc::now());
        gate_repo
            .update_cas(&gate, gate.lock_version)
            .await
            .unwrap();

        let err = service
            .record_decision(gate.id, GateDecision::Reject, "dave", serde_json::json!({}))
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            ApprovalGateServiceError::Workflow(
                adam_domain::workflow::WorkflowError::IllegalTransition { .. }
            )
        ));
    }
}
