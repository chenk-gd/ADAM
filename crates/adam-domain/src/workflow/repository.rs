//! Repository traits for workflow automation entities.
//!
//! Write operations use optimistic locking via `lock_version` CAS and return
//! [`RepositoryError::ConcurrentModification`] on conflict. Unique-key
//! violations on idempotency keys are the final idempotency guard.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::sync::Arc;

use crate::asset::instance::{AssetId, OrganizationId, ProjectId};
use crate::repository::RepositoryError;
use crate::workflow::action::{CreateActionCommand, WorkflowAction, WorkflowActionId};
use crate::workflow::agent_task::{AgentTask, AgentTaskId, Capability, CreateAgentTaskCommand};
use crate::workflow::approval_gate::{ApprovalGate, ApprovalGateId, CreateApprovalGateCommand};
use crate::workflow::dead_letter::{DeadLetter, DeadLetterId, DeadLetterStatus};
use crate::workflow::event::{CorrelationId, EventType, WorkflowEvent, WorkflowEventId};
use crate::workflow::instance::{CreateInstanceCommand, WorkflowInstance, WorkflowInstanceId};
use crate::workflow::rule::{PromotionRule, RuleScope};
use crate::workflow::state_machine::AgentTaskStatus;

// ---------------------------------------------------------------------------
// WorkflowEventRepository
// ---------------------------------------------------------------------------

/// Repository for the append-only workflow event log.
#[async_trait]
pub trait WorkflowEventRepository: Send + Sync {
    /// Append an event. Duplicate idempotency keys return the existing event
    /// via [`RepositoryError::DuplicateIdempotencyKey`].
    async fn append(&self, event: &WorkflowEvent) -> Result<WorkflowEvent, RepositoryError>;

    /// Find an event by id.
    async fn find_by_id(
        &self,
        id: &WorkflowEventId,
    ) -> Result<Option<WorkflowEvent>, RepositoryError>;

    /// Find an event by organization-scoped idempotency key.
    async fn find_by_idempotency_key(
        &self,
        organization_id: &OrganizationId,
        idempotency_key: &str,
    ) -> Result<Option<WorkflowEvent>, RepositoryError>;

    /// Find events by correlation id.
    async fn find_by_correlation_id(
        &self,
        correlation_id: &CorrelationId,
    ) -> Result<Vec<WorkflowEvent>, RepositoryError>;

    /// Find events emitted by a given source asset.
    async fn find_by_asset(
        &self,
        asset_id: &AssetId,
    ) -> Result<Vec<WorkflowEvent>, RepositoryError>;
}

#[async_trait]
impl<T: WorkflowEventRepository + ?Sized> WorkflowEventRepository for Arc<T> {
    async fn append(&self, event: &WorkflowEvent) -> Result<WorkflowEvent, RepositoryError> {
        self.as_ref().append(event).await
    }
    async fn find_by_id(
        &self,
        id: &WorkflowEventId,
    ) -> Result<Option<WorkflowEvent>, RepositoryError> {
        self.as_ref().find_by_id(id).await
    }
    async fn find_by_idempotency_key(
        &self,
        organization_id: &OrganizationId,
        idempotency_key: &str,
    ) -> Result<Option<WorkflowEvent>, RepositoryError> {
        self.as_ref()
            .find_by_idempotency_key(organization_id, idempotency_key)
            .await
    }
    async fn find_by_correlation_id(
        &self,
        correlation_id: &CorrelationId,
    ) -> Result<Vec<WorkflowEvent>, RepositoryError> {
        self.as_ref().find_by_correlation_id(correlation_id).await
    }
    async fn find_by_asset(
        &self,
        asset_id: &AssetId,
    ) -> Result<Vec<WorkflowEvent>, RepositoryError> {
        self.as_ref().find_by_asset(asset_id).await
    }
}

// ---------------------------------------------------------------------------
// PromotionRuleRepository
// ---------------------------------------------------------------------------

/// Repository for promotion rules. Administration operations that atomically
/// replace the active rule set should run at SERIALIZABLE isolation.
#[async_trait]
pub trait PromotionRuleRepository: Send + Sync {
    /// Create a rule.
    async fn create(&self, rule: &PromotionRule) -> Result<PromotionRule, RepositoryError>;

    /// Find a rule by id.
    async fn find_by_id(&self, id: &uuid::Uuid) -> Result<Option<PromotionRule>, RepositoryError>;

    /// Load enabled rules eligible for an event type in an organization.
    async fn find_enabled_for(
        &self,
        organization_id: &OrganizationId,
        event_type: EventType,
        scope: Option<RuleScope>,
        now: DateTime<Utc>,
    ) -> Result<Vec<PromotionRule>, RepositoryError>;
}

#[async_trait]
impl<T: PromotionRuleRepository + ?Sized> PromotionRuleRepository for Arc<T> {
    async fn create(&self, rule: &PromotionRule) -> Result<PromotionRule, RepositoryError> {
        self.as_ref().create(rule).await
    }
    async fn find_by_id(&self, id: &uuid::Uuid) -> Result<Option<PromotionRule>, RepositoryError> {
        self.as_ref().find_by_id(id).await
    }
    async fn find_enabled_for(
        &self,
        organization_id: &OrganizationId,
        event_type: EventType,
        scope: Option<RuleScope>,
        now: DateTime<Utc>,
    ) -> Result<Vec<PromotionRule>, RepositoryError> {
        self.as_ref()
            .find_enabled_for(organization_id, event_type, scope, now)
            .await
    }
}

// ---------------------------------------------------------------------------
// WorkflowInstanceRepository
// ---------------------------------------------------------------------------

/// Repository for workflow instances (Saga coordinators).
#[async_trait]
pub trait WorkflowInstanceRepository: Send + Sync {
    /// Create a new instance.
    async fn create(
        &self,
        cmd: &CreateInstanceCommand,
    ) -> Result<WorkflowInstance, RepositoryError>;

    /// Find an instance by id.
    async fn find_by_id(
        &self,
        id: &WorkflowInstanceId,
    ) -> Result<Option<WorkflowInstance>, RepositoryError>;

    /// Update an instance status with CAS on `lock_version`.
    /// Returns the new lock version, or `ConcurrentModification` on mismatch.
    async fn update_cas(
        &self,
        id: &WorkflowInstanceId,
        expected_lock_version: i64,
        new_status: crate::workflow::state_machine::InstanceStatus,
    ) -> Result<i64, RepositoryError>;

    /// Find non-terminal instances for recovery after a process restart.
    async fn find_non_terminal(&self) -> Result<Vec<WorkflowInstance>, RepositoryError>;
}

#[async_trait]
impl<T: WorkflowInstanceRepository + ?Sized> WorkflowInstanceRepository for Arc<T> {
    async fn create(
        &self,
        cmd: &CreateInstanceCommand,
    ) -> Result<WorkflowInstance, RepositoryError> {
        self.as_ref().create(cmd).await
    }
    async fn find_by_id(
        &self,
        id: &WorkflowInstanceId,
    ) -> Result<Option<WorkflowInstance>, RepositoryError> {
        self.as_ref().find_by_id(id).await
    }
    async fn update_cas(
        &self,
        id: &WorkflowInstanceId,
        expected_lock_version: i64,
        new_status: crate::workflow::state_machine::InstanceStatus,
    ) -> Result<i64, RepositoryError> {
        self.as_ref()
            .update_cas(id, expected_lock_version, new_status)
            .await
    }
    async fn find_non_terminal(&self) -> Result<Vec<WorkflowInstance>, RepositoryError> {
        self.as_ref().find_non_terminal().await
    }
}

// ---------------------------------------------------------------------------
// WorkflowActionRepository
// ---------------------------------------------------------------------------

/// Repository for workflow actions.
#[async_trait]
pub trait WorkflowActionRepository: Send + Sync {
    /// Create an action. Duplicate idempotency keys return
    /// [`RepositoryError::DuplicateIdempotencyKey`].
    async fn create(&self, cmd: &CreateActionCommand) -> Result<WorkflowAction, RepositoryError>;

    /// Find an action by id.
    async fn find_by_id(
        &self,
        id: &WorkflowActionId,
    ) -> Result<Option<WorkflowAction>, RepositoryError>;

    /// Find an action by organization-scoped idempotency key.
    async fn find_by_idempotency_key(
        &self,
        organization_id: &OrganizationId,
        idempotency_key: &str,
    ) -> Result<Option<WorkflowAction>, RepositoryError>;

    /// Find actions belonging to an instance.
    async fn find_by_instance(
        &self,
        instance_id: &WorkflowInstanceId,
    ) -> Result<Vec<WorkflowAction>, RepositoryError>;

    /// Find active (non-terminal) actions targeting an asset.
    async fn find_active_by_target(
        &self,
        target_asset_id: &AssetId,
    ) -> Result<Vec<WorkflowAction>, RepositoryError>;

    /// Update an action with CAS on `lock_version`. The updated action is
    /// returned; the caller supplies the full new field set.
    async fn update_cas(
        &self,
        action: &WorkflowAction,
        expected_lock_version: i64,
    ) -> Result<WorkflowAction, RepositoryError>;
}

#[async_trait]
impl<T: WorkflowActionRepository + ?Sized> WorkflowActionRepository for Arc<T> {
    async fn create(&self, cmd: &CreateActionCommand) -> Result<WorkflowAction, RepositoryError> {
        self.as_ref().create(cmd).await
    }
    async fn find_by_id(
        &self,
        id: &WorkflowActionId,
    ) -> Result<Option<WorkflowAction>, RepositoryError> {
        self.as_ref().find_by_id(id).await
    }
    async fn find_by_idempotency_key(
        &self,
        organization_id: &OrganizationId,
        idempotency_key: &str,
    ) -> Result<Option<WorkflowAction>, RepositoryError> {
        self.as_ref()
            .find_by_idempotency_key(organization_id, idempotency_key)
            .await
    }
    async fn find_by_instance(
        &self,
        instance_id: &WorkflowInstanceId,
    ) -> Result<Vec<WorkflowAction>, RepositoryError> {
        self.as_ref().find_by_instance(instance_id).await
    }
    async fn find_active_by_target(
        &self,
        target_asset_id: &AssetId,
    ) -> Result<Vec<WorkflowAction>, RepositoryError> {
        self.as_ref().find_active_by_target(target_asset_id).await
    }
    async fn update_cas(
        &self,
        action: &WorkflowAction,
        expected_lock_version: i64,
    ) -> Result<WorkflowAction, RepositoryError> {
        self.as_ref()
            .update_cas(action, expected_lock_version)
            .await
    }
}

// ---------------------------------------------------------------------------
// AgentTaskRepository
// ---------------------------------------------------------------------------

/// Repository for agent tasks.
#[async_trait]
pub trait AgentTaskRepository: Send + Sync {
    /// Create a task. Duplicate idempotency keys return
    /// [`RepositoryError::DuplicateIdempotencyKey`].
    async fn create(&self, cmd: &CreateAgentTaskCommand) -> Result<AgentTask, RepositoryError>;

    /// Find a task by id.
    async fn find_by_id(&self, id: &AgentTaskId) -> Result<Option<AgentTask>, RepositoryError>;

    /// Find tasks created for a workflow action.
    async fn find_by_action(
        &self,
        action_id: &WorkflowActionId,
    ) -> Result<Vec<AgentTask>, RepositoryError>;

    /// List queued tasks for a capability and optional project.
    async fn list_queued(
        &self,
        organization_id: &OrganizationId,
        capability: &Capability,
        project_id: Option<&ProjectId>,
    ) -> Result<Vec<AgentTask>, RepositoryError>;

    /// Atomically claim a queued task (`Queued -> Claimed`).
    ///
    /// Returns the claimed task, or `None` if the task was no longer queued.
    async fn claim_cas(
        &self,
        id: &AgentTaskId,
        agent_id: &str,
        claimed_at: chrono::DateTime<chrono::Utc>,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<AgentTask>, RepositoryError>;

    /// Update a task with CAS on `lock_version`.
    async fn update_cas(
        &self,
        task: &AgentTask,
        expected_lock_version: i64,
    ) -> Result<AgentTask, RepositoryError>;

    /// Find tasks in a non-terminal state whose expiry has passed.
    async fn find_expired(
        &self,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<AgentTask>, RepositoryError>;

    /// Find tasks by status for an organization (for observability/retry).
    async fn find_by_status(
        &self,
        organization_id: &OrganizationId,
        status: AgentTaskStatus,
    ) -> Result<Vec<AgentTask>, RepositoryError>;
}

#[async_trait]
impl<T: AgentTaskRepository + ?Sized> AgentTaskRepository for Arc<T> {
    async fn create(&self, cmd: &CreateAgentTaskCommand) -> Result<AgentTask, RepositoryError> {
        self.as_ref().create(cmd).await
    }
    async fn find_by_id(&self, id: &AgentTaskId) -> Result<Option<AgentTask>, RepositoryError> {
        self.as_ref().find_by_id(id).await
    }
    async fn find_by_action(
        &self,
        action_id: &WorkflowActionId,
    ) -> Result<Vec<AgentTask>, RepositoryError> {
        self.as_ref().find_by_action(action_id).await
    }
    async fn list_queued(
        &self,
        organization_id: &OrganizationId,
        capability: &Capability,
        project_id: Option<&ProjectId>,
    ) -> Result<Vec<AgentTask>, RepositoryError> {
        self.as_ref()
            .list_queued(organization_id, capability, project_id)
            .await
    }
    async fn claim_cas(
        &self,
        id: &AgentTaskId,
        agent_id: &str,
        claimed_at: chrono::DateTime<chrono::Utc>,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<AgentTask>, RepositoryError> {
        self.as_ref()
            .claim_cas(id, agent_id, claimed_at, expires_at)
            .await
    }
    async fn update_cas(
        &self,
        task: &AgentTask,
        expected_lock_version: i64,
    ) -> Result<AgentTask, RepositoryError> {
        self.as_ref().update_cas(task, expected_lock_version).await
    }
    async fn find_expired(
        &self,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<AgentTask>, RepositoryError> {
        self.as_ref().find_expired(now).await
    }
    async fn find_by_status(
        &self,
        organization_id: &OrganizationId,
        status: AgentTaskStatus,
    ) -> Result<Vec<AgentTask>, RepositoryError> {
        self.as_ref().find_by_status(organization_id, status).await
    }
}

// ---------------------------------------------------------------------------
// ApprovalGateRepository
// ---------------------------------------------------------------------------

/// Repository for approval gates.
#[async_trait]
pub trait ApprovalGateRepository: Send + Sync {
    /// Create a gate.
    async fn create(
        &self,
        cmd: &CreateApprovalGateCommand,
    ) -> Result<ApprovalGate, RepositoryError>;

    /// Find a gate by id.
    async fn find_by_id(
        &self,
        id: &ApprovalGateId,
    ) -> Result<Option<ApprovalGate>, RepositoryError>;

    /// Find gates by action.
    async fn find_by_action(
        &self,
        action_id: &WorkflowActionId,
    ) -> Result<Vec<ApprovalGate>, RepositoryError>;

    /// Find pending gates for an organization.
    async fn find_pending(
        &self,
        organization_id: &OrganizationId,
    ) -> Result<Vec<ApprovalGate>, RepositoryError>;

    /// Record a decision with CAS on `lock_version`.
    async fn update_cas(
        &self,
        gate: &ApprovalGate,
        expected_lock_version: i64,
    ) -> Result<ApprovalGate, RepositoryError>;
}

#[async_trait]
impl<T: ApprovalGateRepository + ?Sized> ApprovalGateRepository for Arc<T> {
    async fn create(
        &self,
        cmd: &CreateApprovalGateCommand,
    ) -> Result<ApprovalGate, RepositoryError> {
        self.as_ref().create(cmd).await
    }
    async fn find_by_id(
        &self,
        id: &ApprovalGateId,
    ) -> Result<Option<ApprovalGate>, RepositoryError> {
        self.as_ref().find_by_id(id).await
    }
    async fn find_by_action(
        &self,
        action_id: &WorkflowActionId,
    ) -> Result<Vec<ApprovalGate>, RepositoryError> {
        self.as_ref().find_by_action(action_id).await
    }
    async fn find_pending(
        &self,
        organization_id: &OrganizationId,
    ) -> Result<Vec<ApprovalGate>, RepositoryError> {
        self.as_ref().find_pending(organization_id).await
    }
    async fn update_cas(
        &self,
        gate: &ApprovalGate,
        expected_lock_version: i64,
    ) -> Result<ApprovalGate, RepositoryError> {
        self.as_ref().update_cas(gate, expected_lock_version).await
    }
}

// ---------------------------------------------------------------------------
// DeadLetterRepository
// ---------------------------------------------------------------------------

/// Repository for the workflow dead letter queue.
#[async_trait]
pub trait DeadLetterRepository: Send + Sync {
    /// Enqueue a dead letter entry.
    async fn enqueue(&self, entry: &DeadLetter) -> Result<DeadLetter, RepositoryError>;

    /// Find an entry by id.
    async fn find_by_id(&self, id: &DeadLetterId) -> Result<Option<DeadLetter>, RepositoryError>;

    /// List entries by status for an organization.
    async fn find_by_status(
        &self,
        organization_id: &OrganizationId,
        status: DeadLetterStatus,
    ) -> Result<Vec<DeadLetter>, RepositoryError>;

    /// Update an entry's status.
    async fn update_status(
        &self,
        id: &DeadLetterId,
        status: DeadLetterStatus,
        resolved_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<DeadLetter, RepositoryError>;
}

#[async_trait]
impl<T: DeadLetterRepository + ?Sized> DeadLetterRepository for Arc<T> {
    async fn enqueue(&self, entry: &DeadLetter) -> Result<DeadLetter, RepositoryError> {
        self.as_ref().enqueue(entry).await
    }
    async fn find_by_id(&self, id: &DeadLetterId) -> Result<Option<DeadLetter>, RepositoryError> {
        self.as_ref().find_by_id(id).await
    }
    async fn find_by_status(
        &self,
        organization_id: &OrganizationId,
        status: DeadLetterStatus,
    ) -> Result<Vec<DeadLetter>, RepositoryError> {
        self.as_ref().find_by_status(organization_id, status).await
    }
    async fn update_status(
        &self,
        id: &DeadLetterId,
        status: DeadLetterStatus,
        resolved_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<DeadLetter, RepositoryError> {
        self.as_ref().update_status(id, status, resolved_at).await
    }
}
