//! Workflow instance service — Saga coordinator lifecycle (Slice 1).
//!
//! Slice 1 only needs single-action instances: one instance coordinates one
//! workflow action. The service creates instances, advances them through the
//! [`InstanceStatus`] state machine with CAS, and completes/fails them when
//! their action reaches a terminal state. Multi-action Saga compensation is
//! deferred to Slice 3.

use std::sync::Arc;

use adam_domain::workflow::WorkflowError;
use adam_domain::workflow::event::{CorrelationId, EventType, WorkflowEvent, WorkflowEventId};
use adam_domain::workflow::instance::{
    CreateInstanceCommand, WorkflowInstance, WorkflowInstanceId,
};
use adam_domain::workflow::repository::{WorkflowEventRepository, WorkflowInstanceRepository};
use adam_domain::workflow::state_machine::{InstanceStatus, StateMachine};
use adam_domain::{OrganizationId, ProjectId, RepositoryError};

use super::{ClockRef, SystemClock};

/// Errors raised by [`WorkflowInstanceService`].
#[derive(Debug, thiserror::Error)]
pub enum WorkflowInstanceServiceError {
    #[error("repository error: {0}")]
    Repository(RepositoryError),
    #[error("workflow error: {0}")]
    Workflow(WorkflowError),
    #[error("instance not found: {0}")]
    NotFound(WorkflowInstanceId),
}

impl From<RepositoryError> for WorkflowInstanceServiceError {
    fn from(err: RepositoryError) -> Self {
        WorkflowInstanceServiceError::Repository(err)
    }
}

impl From<WorkflowError> for WorkflowInstanceServiceError {
    fn from(err: WorkflowError) -> Self {
        WorkflowInstanceServiceError::Workflow(err)
    }
}

/// Manages [`WorkflowInstance`] lifecycle with CAS optimistic locking.
#[derive(Clone)]
pub struct WorkflowInstanceService<IR, ER>
where
    IR: WorkflowInstanceRepository + ?Sized,
    ER: WorkflowEventRepository + ?Sized,
{
    instance_repo: Arc<IR>,
    event_repo: Arc<ER>,
    clock: ClockRef,
}

impl<IR, ER> WorkflowInstanceService<IR, ER>
where
    IR: WorkflowInstanceRepository + ?Sized,
    ER: WorkflowEventRepository + ?Sized,
{
    /// Create a new service.
    pub fn new(instance_repo: Arc<IR>, event_repo: Arc<ER>) -> Self {
        Self {
            instance_repo,
            event_repo,
            clock: Arc::new(SystemClock),
        }
    }

    /// Create a new service with a custom clock (for tests).
    pub fn with_clock(instance_repo: Arc<IR>, event_repo: Arc<ER>, clock: ClockRef) -> Self {
        Self {
            instance_repo,
            event_repo,
            clock,
        }
    }

    /// Create a new instance in `Pending`.
    pub async fn create(
        &self,
        organization_id: OrganizationId,
        project_id: Option<ProjectId>,
        correlation_id: CorrelationId,
        template: adam_domain::workflow::instance::WorkflowTemplate,
        cascade_depth: i32,
    ) -> Result<WorkflowInstance, WorkflowInstanceServiceError> {
        let instance = self
            .instance_repo
            .create(&CreateInstanceCommand {
                organization_id,
                project_id,
                correlation_id,
                template,
                cascade_depth,
            })
            .await?;
        Ok(instance)
    }

    /// Advance an instance to `target` using CAS. Illegal transitions return
    /// a [`WorkflowError::IllegalTransition`] and emit no event.
    pub async fn advance(
        &self,
        instance_id: WorkflowInstanceId,
        target: InstanceStatus,
    ) -> Result<WorkflowInstance, WorkflowInstanceServiceError> {
        let instance = self
            .instance_repo
            .find_by_id(&instance_id)
            .await?
            .ok_or(WorkflowInstanceServiceError::NotFound(instance_id))?;

        instance.status.validate_transition(target)?;

        self.instance_repo
            .update_cas(&instance_id, instance.lock_version, target)
            .await?;

        let updated = self
            .instance_repo
            .find_by_id(&instance_id)
            .await?
            .ok_or(WorkflowInstanceServiceError::NotFound(instance_id))?;
        Ok(updated)
    }

    /// Mark an instance completed and emit an `ActionSucceeded`-style marker
    /// event carrying the instance id in its payload. The event shares the
    /// instance correlation id so the full chain can be reconstructed.
    pub async fn complete(
        &self,
        instance_id: WorkflowInstanceId,
    ) -> Result<WorkflowInstance, WorkflowInstanceServiceError> {
        let instance = self.advance(instance_id, InstanceStatus::Completed).await?;
        self.emit_instance_event(&instance, EventType::ActionSucceeded)
            .await?;
        Ok(instance)
    }

    /// Mark an instance failed and emit an `ActionFailed` marker event.
    pub async fn fail(
        &self,
        instance_id: WorkflowInstanceId,
    ) -> Result<WorkflowInstance, WorkflowInstanceServiceError> {
        let instance = self.advance(instance_id, InstanceStatus::Failed).await?;
        self.emit_instance_event(&instance, EventType::ActionFailed)
            .await?;
        Ok(instance)
    }

    async fn emit_instance_event(
        &self,
        instance: &WorkflowInstance,
        event_type: EventType,
    ) -> Result<WorkflowEvent, WorkflowInstanceServiceError> {
        let event = WorkflowEvent {
            id: WorkflowEventId::new(),
            organization_id: instance.organization_id,
            project_id: instance.project_id,
            correlation_id: instance.correlation_id,
            event_type,
            source_asset_id: adam_domain::AssetId::from_uuid(instance.id.0),
            source_asset_type_id: adam_domain::AssetTypeId::from_uuid(uuid::Uuid::nil()),
            payload: serde_json::json!({"instance_id": instance.id.0.to_string()}),
            cascade_depth: instance.cascade_depth,
            triggering_action_id: None,
            idempotency_key: format!("instance:{}:{}", instance.id.0, event_type.as_str()),
            created_at: self.clock.now(),
        };
        let stored = self.event_repo.append(&event).await?;
        Ok(stored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adam_domain::workflow::in_memory::{
        InMemoryWorkflowEventRepository, InMemoryWorkflowInstanceRepository,
    };
    use adam_domain::workflow::instance::WorkflowTemplate;
    use chrono::{TimeZone, Utc};

    struct FixedClock;
    impl super::super::Clock for FixedClock {
        fn now(&self) -> chrono::DateTime<chrono::Utc> {
            Utc.with_ymd_and_hms(2026, 6, 18, 9, 0, 0).unwrap()
        }
    }

    fn svc() -> (
        WorkflowInstanceService<
            InMemoryWorkflowInstanceRepository,
            InMemoryWorkflowEventRepository,
        >,
        Arc<InMemoryWorkflowInstanceRepository>,
        Arc<InMemoryWorkflowEventRepository>,
    ) {
        let instance_repo = Arc::new(InMemoryWorkflowInstanceRepository::default());
        let event_repo = Arc::new(InMemoryWorkflowEventRepository::default());
        let svc = WorkflowInstanceService::with_clock(
            instance_repo.clone(),
            event_repo.clone(),
            Arc::new(FixedClock),
        );
        (svc, instance_repo, event_repo)
    }

    #[tokio::test]
    async fn creates_instance_in_pending() {
        let (svc, repo, _) = svc();
        let instance = svc
            .create(
                OrganizationId::from_uuid(uuid::Uuid::nil()),
                None,
                CorrelationId::from_uuid(uuid::Uuid::nil()),
                WorkflowTemplate::Feature,
                0,
            )
            .await
            .unwrap();
        assert_eq!(instance.status, InstanceStatus::Pending);
        assert_eq!(instance.lock_version, 1);
        assert_eq!(
            repo.find_by_id(&instance.id).await.unwrap().unwrap().status,
            InstanceStatus::Pending
        );
    }

    #[tokio::test]
    async fn advance_validates_state_machine() {
        let (svc, _, _) = svc();
        let instance = svc
            .create(
                OrganizationId::from_uuid(uuid::Uuid::nil()),
                None,
                CorrelationId::from_uuid(uuid::Uuid::nil()),
                WorkflowTemplate::Feature,
                0,
            )
            .await
            .unwrap();

        // Pending -> Ready is legal.
        let ready = svc
            .advance(instance.id, InstanceStatus::Ready)
            .await
            .unwrap();
        assert_eq!(ready.status, InstanceStatus::Ready);

        // Ready -> Pending is illegal.
        let err = svc
            .advance(instance.id, InstanceStatus::Pending)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            WorkflowInstanceServiceError::Workflow(WorkflowError::IllegalTransition { .. })
        ));
    }

    #[tokio::test]
    async fn complete_emits_succeeded_event_with_correlation() {
        let (svc, _, event_repo) = svc();
        let correlation = CorrelationId::from_uuid(uuid::Uuid::nil());
        let instance = svc
            .create(
                OrganizationId::from_uuid(uuid::Uuid::nil()),
                None,
                correlation,
                WorkflowTemplate::Feature,
                0,
            )
            .await
            .unwrap();
        svc.advance(instance.id, InstanceStatus::Ready)
            .await
            .unwrap();
        svc.advance(instance.id, InstanceStatus::InProgress)
            .await
            .unwrap();
        let completed = svc.complete(instance.id).await.unwrap();
        assert_eq!(completed.status, InstanceStatus::Completed);

        let chain = event_repo
            .find_by_correlation_id(&correlation)
            .await
            .unwrap();
        assert!(
            chain
                .iter()
                .any(|e| e.event_type == EventType::ActionSucceeded),
            "completion must emit an ActionSucceeded event on the same correlation"
        );
    }
}
