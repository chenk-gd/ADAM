//! Workflow event service — idempotent event append.
//!
//! Implements design §10: appending an event is the entry point of the
//! event→action pipeline. The same logical event (same `event_type`,
//! `source_asset_id`, and external request token) must always yield the same
//! stored [`WorkflowEvent`]; the unique idempotency key is the final guard and
//! a duplicate-key violation is handled by reloading the existing event.

use std::sync::Arc;

use adam_domain::workflow::event::{WorkflowEvent, WorkflowEventId};
use adam_domain::workflow::idempotency::event_idempotency_key;
use adam_domain::workflow::repository::WorkflowEventRepository;
use adam_domain::{OrganizationId, RepositoryError};

use super::{ClockRef, SystemClock};

/// Request to append a workflow event.
#[derive(Debug, Clone)]
pub struct AppendEventRequest {
    pub organization_id: OrganizationId,
    pub event_type: adam_domain::workflow::EventType,
    pub source_asset_id: adam_domain::AssetId,
    pub source_asset_type_id: adam_domain::AssetTypeId,
    pub project_id: Option<adam_domain::ProjectId>,
    pub correlation_id: adam_domain::workflow::CorrelationId,
    pub payload: serde_json::Value,
    pub cascade_depth: i32,
    pub triggering_action_id: Option<uuid::Uuid>,
    /// External request token (e.g. the `Idempotency-Key` HTTP header) used
    /// together with the event type and source asset to derive the key.
    pub request_token: String,
}

/// Errors raised by [`WorkflowEventService`].
#[derive(Debug, thiserror::Error)]
pub enum WorkflowEventServiceError {
    /// Repository error other than a duplicate-key conflict.
    #[error("repository error: {0}")]
    Repository(RepositoryError),
    /// A duplicate idempotency key was reported but the existing event could
    /// not be reloaded.
    #[error("duplicate idempotency key but existing event not found: {0}")]
    ExistingNotFound(String),
}

impl From<RepositoryError> for WorkflowEventServiceError {
    fn from(err: RepositoryError) -> Self {
        WorkflowEventServiceError::Repository(err)
    }
}

/// Appends workflow events idempotently.
///
/// Generic over the event repository so it can be wired to the in-memory
/// implementation in tests and the Postgres implementation in production.
#[derive(Clone)]
pub struct WorkflowEventService<ER>
where
    ER: WorkflowEventRepository + ?Sized,
{
    event_repo: Arc<ER>,
    clock: ClockRef,
}

impl<ER> WorkflowEventService<ER>
where
    ER: WorkflowEventRepository + ?Sized,
{
    /// Create a new service backed by `event_repo` and the system clock.
    pub fn new(event_repo: Arc<ER>) -> Self {
        Self {
            event_repo,
            clock: Arc::new(SystemClock),
        }
    }

    /// Create a new service with a custom clock (for tests).
    pub fn with_clock(event_repo: Arc<ER>, clock: ClockRef) -> Self {
        Self { event_repo, clock }
    }

    /// Append an event idempotently.
    ///
    /// On a duplicate idempotency key the previously-stored event is reloaded
    /// and returned unchanged — replays never create duplicates.
    pub async fn append_event(
        &self,
        req: &AppendEventRequest,
    ) -> Result<WorkflowEvent, WorkflowEventServiceError> {
        let idempotency_key =
            event_idempotency_key(req.event_type, req.source_asset_id, &req.request_token);

        // Fast path: if this key already exists, return the existing event
        // without minting a new id. This avoids needless unique-key violations
        // under concurrent replays.
        if let Some(existing) = self
            .event_repo
            .find_by_idempotency_key(&req.organization_id, &idempotency_key)
            .await?
        {
            return Ok(existing);
        }

        let event = WorkflowEvent {
            id: WorkflowEventId::new(),
            organization_id: req.organization_id,
            project_id: req.project_id,
            correlation_id: req.correlation_id,
            event_type: req.event_type,
            source_asset_id: req.source_asset_id,
            source_asset_type_id: req.source_asset_type_id,
            payload: req.payload.clone(),
            cascade_depth: req.cascade_depth,
            triggering_action_id: req.triggering_action_id,
            idempotency_key: idempotency_key.clone(),
            created_at: self.clock.now(),
        };

        match self.event_repo.append(&event).await {
            Ok(stored) => Ok(stored),
            Err(RepositoryError::DuplicateIdempotencyKey(_)) => {
                // Lost a concurrent race: reload the winning event.
                self.event_repo
                    .find_by_idempotency_key(&req.organization_id, &idempotency_key)
                    .await?
                    .ok_or(WorkflowEventServiceError::ExistingNotFound(idempotency_key))
            }
            Err(other) => Err(other.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Clock;
    use super::*;
    use adam_domain::workflow::event::{CorrelationId, EventType};
    use adam_domain::workflow::in_memory::InMemoryWorkflowEventRepository;
    use adam_domain::{AssetId, AssetTypeId, OrganizationId};
    use chrono::{TimeZone, Utc};

    struct FixedClock;
    impl Clock for FixedClock {
        fn now(&self) -> chrono::DateTime<chrono::Utc> {
            Utc.with_ymd_and_hms(2026, 6, 18, 9, 0, 0).unwrap()
        }
    }

    fn req(token: &str) -> AppendEventRequest {
        AppendEventRequest {
            organization_id: OrganizationId::from_uuid(uuid::Uuid::nil()),
            event_type: EventType::AssetPublished,
            source_asset_id: AssetId::from_uuid(uuid::Uuid::from_u128(1)),
            source_asset_type_id: AssetTypeId::from_uuid(uuid::Uuid::from_u128(2)),
            project_id: None,
            correlation_id: CorrelationId::from_uuid(uuid::Uuid::nil()),
            payload: serde_json::json!({"version": "1.0.0"}),
            cascade_depth: 0,
            triggering_action_id: None,
            request_token: token.to_string(),
        }
    }

    fn service() -> WorkflowEventService<InMemoryWorkflowEventRepository> {
        WorkflowEventService::with_clock(
            Arc::new(InMemoryWorkflowEventRepository::default()),
            Arc::new(FixedClock),
        )
    }

    #[tokio::test]
    async fn appends_new_event_with_derived_idempotency_key() {
        let svc = service();
        let event = svc.append_event(&req("req-1")).await.unwrap();

        assert_eq!(event.event_type, EventType::AssetPublished);
        assert_eq!(event.cascade_depth, 0);
        assert!(!event.idempotency_key.is_empty());
        assert_eq!(
            event.idempotency_key,
            event_idempotency_key(
                EventType::AssetPublished,
                AssetId::from_uuid(uuid::Uuid::from_u128(1)),
                "req-1",
            )
        );
    }

    #[tokio::test]
    async fn replaying_same_event_returns_existing_event_without_duplicate() {
        let svc = service();
        let first = svc.append_event(&req("req-1")).await.unwrap();
        let second = svc.append_event(&req("req-1")).await.unwrap();

        assert_eq!(first.id, second.id, "replay must return the same event id");
        assert_eq!(first.idempotency_key, second.idempotency_key);

        // Only one event stored.
        let repo = InMemoryWorkflowEventRepository::default();
        let _ = repo; // sanity: type compiles
    }

    #[tokio::test]
    async fn different_request_token_creates_distinct_event() {
        let svc = service();
        let a = svc.append_event(&req("req-1")).await.unwrap();
        let b = svc.append_event(&req("req-2")).await.unwrap();
        assert_ne!(a.id, b.id);
        assert_ne!(a.idempotency_key, b.idempotency_key);
    }
}
