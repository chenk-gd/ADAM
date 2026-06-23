//! Agent task service (Slice 2).

use std::sync::Arc;

use adam_domain::repository::RepositoryError;
use adam_domain::virtual_instance::VirtualInstance;
use adam_domain::workflow::WorkflowError;
use adam_domain::workflow::action::WorkflowActionId;
use adam_domain::workflow::agent_task::{
    AgentTask, AgentTaskId, Capability, CreateAgentTaskCommand,
};
use adam_domain::workflow::idempotency::agent_task_idempotency_key;
use adam_domain::workflow::repository::{
    AgentTaskRepository, WorkflowActionRepository, WorkflowEventRepository,
    WorkflowInstanceRepository,
};
use adam_domain::workflow::state_machine::{ActionStatus, AgentTaskStatus};
use adam_domain::{AssetId, ProjectId};

use super::{ClockRef, SystemClock, WorkflowActionService, WorkflowActionServiceError};

/// Errors raised by [`AgentTaskService`].
#[derive(Debug, thiserror::Error)]
pub enum AgentTaskServiceError {
    #[error("repository error: {0}")]
    Repository(RepositoryError),
    #[error("workflow error: {0}")]
    Workflow(WorkflowError),
    #[error("action service error: {0}")]
    ActionService(WorkflowActionServiceError),
    #[error("agent task not found: {0}")]
    TaskNotFound(AgentTaskId),
    #[error("workflow action not found: {0}")]
    ActionNotFound(WorkflowActionId),
    #[error("action is not ready for agent task creation: {0}")]
    ActionNotReady(WorkflowActionId),
    #[error("agent task is not claimed: {0}")]
    TaskNotClaimed(AgentTaskId),
}

impl From<RepositoryError> for AgentTaskServiceError {
    fn from(err: RepositoryError) -> Self {
        AgentTaskServiceError::Repository(err)
    }
}

impl From<WorkflowError> for AgentTaskServiceError {
    fn from(err: WorkflowError) -> Self {
        AgentTaskServiceError::Workflow(err)
    }
}

impl From<WorkflowActionServiceError> for AgentTaskServiceError {
    fn from(err: WorkflowActionServiceError) -> Self {
        AgentTaskServiceError::ActionService(err)
    }
}

/// Typed link to the assets an agent produced while executing a task.
///
/// Replaces the untyped `Vec<Uuid>` produced-asset list at the service boundary
/// so callers can distinguish a generated virtual context from a published real
/// asset (design §4 AgentTask / S2-T4). The raw ids are still what gets
/// persisted on [`AgentTask::produced_asset_ids`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProducedAssets {
    /// No assets were produced.
    None,
    /// A virtual instance was built as the agent's working context.
    VirtualInstance(adam_domain::VirtualInstanceId),
    /// One or more real assets were published/updated.
    Assets(Vec<AssetId>),
}

impl ProducedAssets {
    /// Flatten the typed links into the raw id list persisted on the task.
    pub fn into_asset_ids(self) -> Vec<uuid::Uuid> {
        match self {
            ProducedAssets::None => Vec::new(),
            ProducedAssets::VirtualInstance(id) => vec![id.0],
            ProducedAssets::Assets(ids) => ids.into_iter().map(|a| a.0).collect(),
        }
    }
}

/// Orchestrates agent task creation, claiming, result submission, and expiry.
#[derive(Clone)]
pub struct AgentTaskService<TR, AR, ER, IR>
where
    TR: AgentTaskRepository + ?Sized,
    AR: WorkflowActionRepository + ?Sized,
    ER: WorkflowEventRepository + ?Sized,
    IR: WorkflowInstanceRepository + ?Sized,
{
    task_repo: Arc<TR>,
    action_repo: Arc<AR>,
    event_repo: Arc<ER>,
    instance_repo: Arc<IR>,
    clock: ClockRef,
}

impl<TR, AR, ER, IR> AgentTaskService<TR, AR, ER, IR>
where
    TR: AgentTaskRepository + ?Sized,
    AR: WorkflowActionRepository + ?Sized,
    ER: WorkflowEventRepository + ?Sized,
    IR: WorkflowInstanceRepository + ?Sized,
{
    /// Create a new service using the system clock.
    pub fn new(
        task_repo: Arc<TR>,
        action_repo: Arc<AR>,
        event_repo: Arc<ER>,
        instance_repo: Arc<IR>,
    ) -> Self {
        Self {
            task_repo,
            action_repo,
            event_repo,
            instance_repo,
            clock: Arc::new(SystemClock),
        }
    }

    /// Create a new service with a custom clock (for tests).
    pub fn with_clock(
        task_repo: Arc<TR>,
        action_repo: Arc<AR>,
        event_repo: Arc<ER>,
        instance_repo: Arc<IR>,
        clock: ClockRef,
    ) -> Self {
        Self {
            task_repo,
            action_repo,
            event_repo,
            instance_repo,
            clock,
        }
    }

    /// Create an idempotent queued task for a ready agent-executable action.
    pub async fn create_task_for_action(
        &self,
        action_id: WorkflowActionId,
        capability: Capability,
    ) -> Result<AgentTask, AgentTaskServiceError> {
        let action = self
            .action_repo
            .find_by_id(&action_id)
            .await?
            .ok_or(AgentTaskServiceError::ActionNotFound(action_id))?;

        if action.status != ActionStatus::Ready {
            return Err(AgentTaskServiceError::ActionNotReady(action_id));
        }

        let instance = self
            .instance_repo
            .find_by_id(&action.instance_id)
            .await?
            .ok_or(AgentTaskServiceError::ActionNotFound(action_id))?;

        let idempotency_key =
            agent_task_attempt_idempotency_key(action.id.0, &capability.0, action.retry_count);
        if let Some(existing) = self
            .task_repo
            .find_by_action(&action.id)
            .await?
            .into_iter()
            .find(|task| task.idempotency_key == idempotency_key)
        {
            return Ok(existing);
        }

        let cmd = CreateAgentTaskCommand {
            organization_id: action.organization_id,
            project_id: instance.project_id,
            action_id: action.id,
            capability,
            idempotency_key: idempotency_key.clone(),
            expires_at: None,
        };

        match self.task_repo.create(&cmd).await {
            Ok(task) => Ok(task),
            Err(RepositoryError::DuplicateIdempotencyKey(_)) => self
                .task_repo
                .find_by_action(&action.id)
                .await?
                .into_iter()
                .find(|task| task.idempotency_key == idempotency_key)
                .ok_or({
                    AgentTaskServiceError::Workflow(WorkflowError::DuplicateIdempotencyKey(
                        idempotency_key,
                    ))
                }),
            Err(other) => Err(other.into()),
        }
    }

    /// Atomically claim a queued task and set its lease expiry.
    pub async fn claim_task(
        &self,
        task_id: AgentTaskId,
        agent_id: &str,
        lease_duration: chrono::Duration,
    ) -> Result<Option<AgentTask>, AgentTaskServiceError> {
        let claimed_at = self.clock.now();
        let expires_at = claimed_at + lease_duration;
        self.task_repo
            .claim_cas(&task_id, agent_id, claimed_at, expires_at)
            .await
            .map_err(Into::into)
    }

    /// Build the AI execution context for a claimed task by reusing the
    /// [`VirtualInstance`] construction logic (design §4 / S2-T4).
    ///
    /// The context is anchored on the parent action's `target_asset_id` and
    /// typed by its `target_asset_type_id`. A task whose action carries no
    /// target asset type cannot build a context and yields `None` (the agent
    /// receives only the raw task). This does not persist the virtual instance;
    /// callers that want it stored should pass it to a
    /// [`VirtualInstanceRepository`].
    pub async fn build_claim_context(
        &self,
        task: &AgentTask,
    ) -> Result<Option<VirtualInstance>, AgentTaskServiceError> {
        let action = self
            .action_repo
            .find_by_id(&task.action_id)
            .await?
            .ok_or(AgentTaskServiceError::ActionNotFound(task.action_id))?;

        let Some(target_type) = action.target_asset_type_id else {
            return Ok(None);
        };
        let instance = self
            .instance_repo
            .find_by_id(&action.instance_id)
            .await?
            .ok_or(AgentTaskServiceError::ActionNotFound(task.action_id))?;
        let anchors = action.target_asset_id.into_iter().collect::<Vec<_>>();
        let project_id = instance
            .project_id
            .or(task.project_id)
            .unwrap_or_else(|| ProjectId::from_uuid(uuid::Uuid::nil()));

        let target_type_name = action.action_type.as_str().to_string();
        let created_by = task
            .agent_id
            .clone()
            .unwrap_or_else(|| "system".to_string());

        Ok(Some(VirtualInstance::new(
            target_type,
            target_type_name,
            anchors,
            project_id,
            task.organization_id,
            created_by,
        )))
    }

    /// Submit a typed result, linking the produced assets (virtual instance or
    /// real assets) to the task and driving the parent action to `Succeeded`.
    /// This is the S2-T4 entry point; it delegates to [`submit_result`] after
    /// flattening the typed links into raw ids.
    pub async fn submit_result_typed(
        &self,
        task_id: AgentTaskId,
        result_payload: serde_json::Value,
        produced: ProducedAssets,
    ) -> Result<AgentTask, AgentTaskServiceError> {
        let asset_ids = produced.into_asset_ids();
        self.submit_result(task_id, result_payload, asset_ids).await
    }

    /// Store task result, link produced assets, and drive the parent action to succeeded.
    ///
    /// Ordering: the parent action is driven to `Succeeded` *before* the task is
    /// marked `Succeeded`. If the action transition fails (illegal transition,
    /// postcondition unmet, CAS conflict), the task remains in its non-terminal
    /// `Claimed`/`Running` state so the agent can retry or the lease can expire —
    /// we never leave a terminal task pointing at a non-terminal action.
    pub async fn submit_result(
        &self,
        task_id: AgentTaskId,
        result_payload: serde_json::Value,
        produced_asset_ids: Vec<uuid::Uuid>,
    ) -> Result<AgentTask, AgentTaskServiceError> {
        let task = self
            .task_repo
            .find_by_id(&task_id)
            .await?
            .ok_or(AgentTaskServiceError::TaskNotFound(task_id))?;

        if !matches!(
            task.status,
            AgentTaskStatus::Claimed | AgentTaskStatus::Running
        ) {
            return Err(AgentTaskServiceError::TaskNotClaimed(task_id));
        }

        let action = self
            .action_repo
            .find_by_id(&task.action_id)
            .await?
            .ok_or(AgentTaskServiceError::ActionNotFound(task.action_id))?;
        let action_service = WorkflowActionService::with_clock(
            self.action_repo.clone(),
            self.event_repo.clone(),
            self.instance_repo.clone(),
            self.clock.clone(),
        );

        // Drive the parent action to Succeeded first. Only once that succeeds do
        // we persist the task result; a failure here propagates without marking
        // the task terminal.
        let action_for_success = if action.status == ActionStatus::Ready {
            action_service
                .transition(action.id, ActionStatus::InProgress, None)
                .await?
        } else {
            action
        };
        if action_for_success.status == ActionStatus::InProgress {
            action_service
                .transition(
                    action_for_success.id,
                    ActionStatus::Succeeded,
                    Some(&result_payload),
                )
                .await?;
        }

        // Action completed — now persist the task result (CAS to Succeeded).
        let mut updated_task = task.clone();
        updated_task.status = AgentTaskStatus::Succeeded;
        updated_task.result_payload = Some(result_payload);
        updated_task.produced_asset_ids = produced_asset_ids;
        let stored_task = self
            .task_repo
            .update_cas(&updated_task, task.lock_version)
            .await?;

        Ok(stored_task)
    }

    /// Expire tasks whose lease has elapsed, then apply the parent action policy.
    ///
    /// For each expired task: mark the task `Expired` (CAS) and drive the parent
    /// `WorkflowAction` to `Failed` when its retry budget is exhausted, so the
    /// workflow does not stall on a dead lease. CAS conflicts on the task (the
    /// agent raced to submit a result) are skipped for that task — the rest of
    /// the batch still proceeds. Action-transition failures are tolerated (the
    /// action may already be terminal) and recorded per-task rather than
    /// aborting the whole scan.
    pub async fn timeout_expired(
        &self,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<AgentTask>, AgentTaskServiceError> {
        let expired = self.task_repo.find_expired(now).await?;
        let action_service = WorkflowActionService::with_clock(
            self.action_repo.clone(),
            self.event_repo.clone(),
            self.instance_repo.clone(),
            self.clock.clone(),
        );

        let mut updated = Vec::new();
        for task in expired {
            // CAS the task to Expired. A ConcurrentModification means the agent
            // submitted a result between the scan and here — skip it; its lease
            // is no longer the source of truth.
            let mut next = task.clone();
            next.status = AgentTaskStatus::Expired;
            let stored = match self.task_repo.update_cas(&next, task.lock_version).await {
                Ok(stored) => stored,
                Err(RepositoryError::ConcurrentModification { .. }) => continue,
                Err(err) => return Err(err.into()),
            };

            // Fail the parent action when its retry budget is exhausted. A
            // non-failing transition (action already terminal, or budget remains
            // for requeue) is tolerated — the action service returns the
            // IllegalTransition/NotFound error, which we swallow here rather than
            // abort the batch.
            if let Err(err) = self
                .fail_parent_action(&task.action_id, &action_service)
                .await
            {
                tracing::warn!(
                    action_id = %task.action_id,
                    error = %err,
                    "timeout_expired: could not fail parent action"
                );
            }

            updated.push(stored);
        }
        Ok(updated)
    }

    /// Fail the parent action of an expired task when its retry budget is
    /// exhausted. If retries remain, the action is left for requeue by the
    /// caller; otherwise it is driven to `Failed` (emitting an `ActionFailed`
    /// event sharing the correlation id) so downstream dirty propagation can
    /// proceed.
    async fn fail_parent_action(
        &self,
        action_id: &WorkflowActionId,
        action_service: &WorkflowActionService<AR, ER, IR>,
    ) -> Result<(), AgentTaskServiceError> {
        let action = self
            .action_repo
            .find_by_id(action_id)
            .await?
            .ok_or(AgentTaskServiceError::ActionNotFound(*action_id))?;

        // Only act on actions that are still mid-flight (Ready/InProgress). A
        // terminal or blocked action is left untouched.
        if !matches!(
            action.status,
            ActionStatus::Ready | ActionStatus::InProgress
        ) {
            return Ok(());
        }

        // Retry budget exhausted -> fail. When retries remain, leave the action
        // for the caller to requeue (out of scope for this slice's lease scan).
        if action.retry_count >= action.max_retries {
            // Drive Ready -> InProgress (if needed) -> Failed so the transition
            // matrix is respected.
            let target = if action.status == ActionStatus::Ready {
                action_service
                    .transition(action.id, ActionStatus::InProgress, None)
                    .await?
            } else {
                action
            };
            if target.status == ActionStatus::InProgress {
                action_service
                    .transition(target.id, ActionStatus::Failed, None)
                    .await?;
            }
        } else {
            let mut retry = action.clone();
            retry.retry_count += 1;
            retry.status = ActionStatus::Ready;
            self.action_repo
                .update_cas(&retry, action.lock_version)
                .await?;
        }
        Ok(())
    }
}

fn agent_task_attempt_idempotency_key(
    action_id: uuid::Uuid,
    capability: &str,
    retry_count: i32,
) -> String {
    format!(
        "{}:attempt:{retry_count}",
        agent_task_idempotency_key(action_id, capability)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use adam_domain::workflow::action::CreateActionCommand;
    use adam_domain::workflow::agent_task::Capability;
    use adam_domain::workflow::in_memory::{
        InMemoryAgentTaskRepository, InMemoryWorkflowActionRepository,
        InMemoryWorkflowEventRepository, InMemoryWorkflowInstanceRepository,
    };
    use adam_domain::workflow::instance::{CreateInstanceCommand, WorkflowTemplate};
    use adam_domain::workflow::repository::{
        AgentTaskRepository, WorkflowActionRepository, WorkflowInstanceRepository,
    };
    use adam_domain::workflow::rule::{ActionType, AutomationLevel};
    use adam_domain::workflow::state_machine::{ActionStatus, AgentTaskStatus, InstanceStatus};
    use adam_domain::workflow::{CorrelationId, WorkflowActionId};
    use adam_domain::{AssetId, AssetTypeId, OrganizationId};
    use chrono::{Duration, TimeZone, Utc};
    use std::sync::Arc;

    struct FixedClock;
    impl super::super::Clock for FixedClock {
        fn now(&self) -> chrono::DateTime<chrono::Utc> {
            Utc.with_ymd_and_hms(2026, 6, 20, 9, 0, 0).unwrap()
        }
    }

    type TestAgentTaskService = AgentTaskService<
        InMemoryAgentTaskRepository,
        InMemoryWorkflowActionRepository,
        InMemoryWorkflowEventRepository,
        InMemoryWorkflowInstanceRepository,
    >;

    fn org() -> OrganizationId {
        OrganizationId::from_uuid(uuid::Uuid::nil())
    }

    async fn seed_ready_action(
        repo: &InMemoryWorkflowActionRepository,
        instance_repo: &InMemoryWorkflowInstanceRepository,
    ) -> WorkflowActionId {
        seed_ready_action_with_project(repo, instance_repo, None).await
    }

    async fn seed_ready_action_with_project(
        repo: &InMemoryWorkflowActionRepository,
        instance_repo: &InMemoryWorkflowInstanceRepository,
        project_id: Option<ProjectId>,
    ) -> WorkflowActionId {
        let instance = instance_repo
            .create(&CreateInstanceCommand {
                organization_id: org(),
                project_id,
                correlation_id: CorrelationId::from_uuid(uuid::Uuid::nil()),
                template: WorkflowTemplate::Feature,
                cascade_depth: 1,
            })
            .await
            .unwrap();

        instance_repo
            .update_cas(&instance.id, instance.lock_version, InstanceStatus::Ready)
            .await
            .unwrap();

        let action = repo
            .create(&CreateActionCommand {
                organization_id: org(),
                instance_id: instance.id,
                action_type: ActionType::UpsertWorkItem,
                target_asset_id: Some(AssetId::from_uuid(uuid::Uuid::from_u128(7))),
                target_asset_type_id: Some(AssetTypeId::from_uuid(uuid::Uuid::from_u128(2))),
                idempotency_key: format!("action:{}", uuid::Uuid::new_v4()),
                preconditions: serde_json::json!([]),
                postconditions: serde_json::json!({}),
                automation_level: AutomationLevel::AgentSuggested,
                is_required: true,
                order_index: 0,
                compensation_action_type: None,
                compensation_payload: None,
                compensation_policy: adam_domain::workflow::CompensationPolicy::None,
                max_retries: 3,
            })
            .await
            .unwrap();

        let mut ready = action.clone();
        ready.status = ActionStatus::Ready;
        repo.update_cas(&ready, action.lock_version).await.unwrap();
        action.id
    }

    fn service() -> (
        TestAgentTaskService,
        Arc<InMemoryAgentTaskRepository>,
        Arc<InMemoryWorkflowActionRepository>,
        Arc<InMemoryWorkflowInstanceRepository>,
    ) {
        let task_repo = Arc::new(InMemoryAgentTaskRepository::default());
        let action_repo = Arc::new(InMemoryWorkflowActionRepository::default());
        let event_repo = Arc::new(InMemoryWorkflowEventRepository::default());
        let instance_repo = Arc::new(InMemoryWorkflowInstanceRepository::default());
        let service = AgentTaskService::with_clock(
            task_repo.clone(),
            action_repo.clone(),
            event_repo,
            instance_repo.clone(),
            Arc::new(FixedClock),
        );
        (service, task_repo, action_repo, instance_repo)
    }

    #[tokio::test]
    async fn creates_task_for_ready_agent_executable_action_idempotently() {
        let (service, _task_repo, action_repo, instance_repo) = service();
        let action_id = seed_ready_action(&action_repo, &instance_repo).await;

        let first = service
            .create_task_for_action(action_id, Capability::create_virtual_asset_context())
            .await
            .unwrap();
        let second = service
            .create_task_for_action(action_id, Capability::create_virtual_asset_context())
            .await
            .unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(first.status, AgentTaskStatus::Queued);
    }

    #[tokio::test]
    async fn claim_task_is_atomic_and_sets_lease() {
        let (service, _task_repo, action_repo, instance_repo) = service();
        let action_id = seed_ready_action(&action_repo, &instance_repo).await;
        let task = service
            .create_task_for_action(action_id, Capability::create_virtual_asset_context())
            .await
            .unwrap();

        let first = service
            .claim_task(task.id, "agent-1", Duration::minutes(5))
            .await
            .unwrap()
            .unwrap();
        let second = service
            .claim_task(task.id, "agent-2", Duration::minutes(5))
            .await
            .unwrap();

        assert_eq!(first.agent_id.as_deref(), Some("agent-1"));
        assert_eq!(first.status, AgentTaskStatus::Claimed);
        assert!(first.expires_at.unwrap() > first.claimed_at.unwrap());
        assert!(second.is_none());
    }

    #[tokio::test]
    async fn submit_result_links_outputs_and_succeeds_parent_action() {
        let (service, task_repo, action_repo, instance_repo) = service();
        let action_id = seed_ready_action(&action_repo, &instance_repo).await;
        let task = service
            .create_task_for_action(action_id, Capability::create_virtual_asset_context())
            .await
            .unwrap();
        let claimed = service
            .claim_task(task.id, "agent-1", Duration::minutes(5))
            .await
            .unwrap()
            .unwrap();

        let produced = vec![uuid::Uuid::from_u128(42)];
        let result = serde_json::json!({"ok": true});
        let completed = service
            .submit_result(claimed.id, result.clone(), produced.clone())
            .await
            .unwrap();

        assert_eq!(completed.status, AgentTaskStatus::Succeeded);
        assert_eq!(completed.result_payload, Some(result));
        assert_eq!(completed.produced_asset_ids, produced);
        assert_eq!(
            action_repo
                .find_by_id(&action_id)
                .await
                .unwrap()
                .unwrap()
                .status,
            ActionStatus::Succeeded
        );
        assert_eq!(
            task_repo
                .find_by_id(&task.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            AgentTaskStatus::Succeeded
        );
    }

    #[tokio::test]
    async fn timeout_expired_marks_claimed_task_expired() {
        let (service, task_repo, action_repo, instance_repo) = service();
        let action_id = seed_ready_action(&action_repo, &instance_repo).await;
        let task = service
            .create_task_for_action(action_id, Capability::create_virtual_asset_context())
            .await
            .unwrap();
        service
            .claim_task(task.id, "agent-1", Duration::minutes(5))
            .await
            .unwrap();

        let expired = service
            .timeout_expired(Utc.with_ymd_and_hms(2026, 6, 20, 9, 6, 0).unwrap())
            .await
            .unwrap();

        assert_eq!(expired.len(), 1);
        assert_eq!(
            task_repo
                .find_by_id(&task.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            AgentTaskStatus::Expired
        );
    }

    #[tokio::test]
    async fn timeout_expired_fails_parent_action_when_retries_exhausted() {
        let (service, _task_repo, action_repo, instance_repo) = service();
        let action_id = seed_ready_action(&action_repo, &instance_repo).await;

        // Exhaust the retry budget so the lease scan must fail the parent action.
        let mut action = action_repo.find_by_id(&action_id).await.unwrap().unwrap();
        action.retry_count = action.max_retries;
        action_repo
            .update_cas(&action, action.lock_version)
            .await
            .unwrap();

        let task = service
            .create_task_for_action(action_id, Capability::create_virtual_asset_context())
            .await
            .unwrap();
        service
            .claim_task(task.id, "agent-1", Duration::minutes(5))
            .await
            .unwrap();

        service
            .timeout_expired(Utc.with_ymd_and_hms(2026, 6, 20, 9, 6, 0).unwrap())
            .await
            .unwrap();

        // The parent action is driven Ready -> InProgress -> Failed.
        let final_action = action_repo.find_by_id(&action_id).await.unwrap().unwrap();
        assert_eq!(final_action.status, ActionStatus::Failed);
    }

    #[tokio::test]
    async fn timeout_expired_leaves_action_when_retries_remain() {
        let (service, _task_repo, action_repo, instance_repo) = service();
        let action_id = seed_ready_action(&action_repo, &instance_repo).await;
        // Leave retry_count below max_retries (seed sets retry_count = 0, max = 3).
        let task = service
            .create_task_for_action(action_id, Capability::create_virtual_asset_context())
            .await
            .unwrap();
        service
            .claim_task(task.id, "agent-1", Duration::minutes(5))
            .await
            .unwrap();

        service
            .timeout_expired(Utc.with_ymd_and_hms(2026, 6, 20, 9, 6, 0).unwrap())
            .await
            .unwrap();

        // Retries remain -> action is left in Ready (not failed) for requeue,
        // and retry_count is bumped so the next attempt gets a fresh task.
        let final_action = action_repo.find_by_id(&action_id).await.unwrap().unwrap();
        assert_eq!(final_action.status, ActionStatus::Ready);
        assert_eq!(final_action.retry_count, 1);
    }

    #[tokio::test]
    async fn submit_result_leaves_task_non_terminal_when_action_postcondition_fails() {
        let (service, task_repo, action_repo, instance_repo) = service();
        let action_id = seed_ready_action(&action_repo, &instance_repo).await;

        // Tighten postconditions so a wrong result payload cannot satisfy them.
        let mut action = action_repo.find_by_id(&action_id).await.unwrap().unwrap();
        action.postconditions = serde_json::json!({"work_item_kind": "feature"});
        action_repo
            .update_cas(&action, action.lock_version)
            .await
            .unwrap();

        let task = service
            .create_task_for_action(action_id, Capability::create_virtual_asset_context())
            .await
            .unwrap();
        let claimed = service
            .claim_task(task.id, "agent-1", Duration::minutes(5))
            .await
            .unwrap()
            .unwrap();

        // Result does NOT satisfy the postcondition -> transition errors and the
        // task must remain Claimed (not Succeeded).
        let wrong = serde_json::json!({"work_item_kind": "bug"});
        let err = service
            .submit_result(claimed.id, wrong, vec![])
            .await
            .unwrap_err();
        assert!(matches!(err, AgentTaskServiceError::ActionService(_)));

        let task_after = task_repo.find_by_id(&task.id).await.unwrap().unwrap();
        assert_eq!(task_after.status, AgentTaskStatus::Claimed);
        // The action never reached Succeeded.
        let action_after = action_repo.find_by_id(&action_id).await.unwrap().unwrap();
        assert_ne!(action_after.status, ActionStatus::Succeeded);
    }

    #[tokio::test]
    async fn build_claim_context_reuses_virtual_instance_construction() {
        let (service, _task_repo, action_repo, instance_repo) = service();
        let action_id = seed_ready_action(&action_repo, &instance_repo).await;
        let task = service
            .create_task_for_action(action_id, Capability::create_virtual_asset_context())
            .await
            .unwrap();
        let claimed = service
            .claim_task(task.id, "agent-7", Duration::minutes(5))
            .await
            .unwrap()
            .unwrap();

        let context = service
            .build_claim_context(&claimed)
            .await
            .unwrap()
            .expect("context must be built when target_asset_type_id is set");

        // The virtual instance is anchored on the action's target asset and
        // typed by its target asset type (seed_ready_action sets both).
        assert_eq!(
            context.target_type,
            AssetTypeId::from_uuid(uuid::Uuid::from_u128(2))
        );
        assert_eq!(
            context.anchors,
            vec![AssetId::from_uuid(uuid::Uuid::from_u128(7))]
        );
        assert_eq!(context.created_by, "agent-7");
        assert_eq!(context.organization_id, org());
    }

    #[tokio::test]
    async fn submit_result_typed_links_virtual_instance_and_succeeds_action() {
        let (service, _task_repo, action_repo, instance_repo) = service();
        let action_id = seed_ready_action(&action_repo, &instance_repo).await;
        let task = service
            .create_task_for_action(action_id, Capability::create_virtual_asset_context())
            .await
            .unwrap();
        let claimed = service
            .claim_task(task.id, "agent-1", Duration::minutes(5))
            .await
            .unwrap()
            .unwrap();

        let vi_id = adam_domain::VirtualInstanceId::new();
        let completed = service
            .submit_result_typed(
                claimed.id,
                serde_json::json!({"ok": true}),
                ProducedAssets::VirtualInstance(vi_id),
            )
            .await
            .unwrap();

        assert_eq!(completed.status, AgentTaskStatus::Succeeded);
        // The virtual instance id is persisted as the produced asset link.
        assert_eq!(completed.produced_asset_ids, vec![vi_id.0]);
        assert_eq!(
            action_repo
                .find_by_id(&action_id)
                .await
                .unwrap()
                .unwrap()
                .status,
            ActionStatus::Succeeded
        );
    }

    #[tokio::test]
    async fn build_claim_context_returns_none_without_target_asset_type() {
        let (service, _task_repo, action_repo, instance_repo) = service();
        let action_id = seed_ready_action(&action_repo, &instance_repo).await;

        // Strip the target asset type so no context can be built.
        let mut action = action_repo.find_by_id(&action_id).await.unwrap().unwrap();
        action.target_asset_type_id = None;
        action_repo
            .update_cas(&action, action.lock_version)
            .await
            .unwrap();

        let task = service
            .create_task_for_action(action_id, Capability::create_virtual_asset_context())
            .await
            .unwrap();
        let claimed = service
            .claim_task(task.id, "agent-1", Duration::minutes(5))
            .await
            .unwrap()
            .unwrap();

        let context = service.build_claim_context(&claimed).await.unwrap();
        assert!(context.is_none());
    }

    #[tokio::test]
    async fn create_task_copies_project_from_parent_instance() {
        let (service, task_repo, action_repo, instance_repo) = service();
        let project_id = ProjectId::from_uuid(uuid::Uuid::from_u128(99));
        let action_id =
            seed_ready_action_with_project(&action_repo, &instance_repo, Some(project_id)).await;

        let task = service
            .create_task_for_action(action_id, Capability::create_virtual_asset_context())
            .await
            .unwrap();

        assert_eq!(task.project_id, Some(project_id));
        let queued = task_repo
            .list_queued(
                &org(),
                &Capability::create_virtual_asset_context(),
                Some(&project_id),
            )
            .await
            .unwrap();
        assert_eq!(queued.len(), 1);
    }

    #[tokio::test]
    async fn build_claim_context_uses_parent_instance_project() {
        let (service, _task_repo, action_repo, instance_repo) = service();
        let project_id = ProjectId::from_uuid(uuid::Uuid::from_u128(100));
        let action_id =
            seed_ready_action_with_project(&action_repo, &instance_repo, Some(project_id)).await;
        let task = service
            .create_task_for_action(action_id, Capability::create_virtual_asset_context())
            .await
            .unwrap();
        let claimed = service
            .claim_task(task.id, "agent-7", Duration::minutes(5))
            .await
            .unwrap()
            .unwrap();

        let context = service
            .build_claim_context(&claimed)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(context.project_id, project_id);
    }

    #[tokio::test]
    async fn expired_task_with_retries_remaining_allows_new_queued_attempt() {
        let (service, _task_repo, action_repo, instance_repo) = service();
        let action_id = seed_ready_action(&action_repo, &instance_repo).await;
        let first = service
            .create_task_for_action(action_id, Capability::create_virtual_asset_context())
            .await
            .unwrap();
        service
            .claim_task(first.id, "agent-1", Duration::minutes(5))
            .await
            .unwrap();

        service
            .timeout_expired(Utc.with_ymd_and_hms(2026, 6, 20, 9, 6, 0).unwrap())
            .await
            .unwrap();
        let second = service
            .create_task_for_action(action_id, Capability::create_virtual_asset_context())
            .await
            .unwrap();

        assert_ne!(first.id, second.id);
        assert_eq!(second.status, AgentTaskStatus::Queued);
        assert_eq!(
            action_repo
                .find_by_id(&action_id)
                .await
                .unwrap()
                .unwrap()
                .retry_count,
            1
        );
    }

    #[tokio::test]
    async fn submit_result_does_not_succeed_action_when_task_cas_loses_race() {
        let (service, task_repo, action_repo, instance_repo) = service();
        let action_id = seed_ready_action(&action_repo, &instance_repo).await;
        let task = service
            .create_task_for_action(action_id, Capability::create_virtual_asset_context())
            .await
            .unwrap();
        let claimed = service
            .claim_task(task.id, "agent-1", Duration::minutes(5))
            .await
            .unwrap()
            .unwrap();

        let mut raced = claimed.clone();
        raced.status = AgentTaskStatus::Expired;
        task_repo
            .update_cas(&raced, claimed.lock_version)
            .await
            .unwrap();

        let err = service
            .submit_result(claimed.id, serde_json::json!({"ok": true}), vec![])
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            AgentTaskServiceError::Repository(RepositoryError::ConcurrentModification { .. })
                | AgentTaskServiceError::TaskNotClaimed(_)
        ));
        let action = action_repo.find_by_id(&action_id).await.unwrap().unwrap();
        assert_ne!(action.status, ActionStatus::Succeeded);
    }
}
