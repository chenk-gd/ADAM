//! Dead letter service (Slice 3).

use std::sync::Arc;

use adam_domain::workflow::dead_letter::{
    DeadLetter, DeadLetterId, DeadLetterSource, DeadLetterStatus,
};
use adam_domain::workflow::repository::DeadLetterRepository;
use adam_domain::{OrganizationId, ProjectId, RepositoryError};
use chrono::Utc;

/// Errors raised by [`DeadLetterService`].
#[derive(Debug, thiserror::Error)]
pub enum DeadLetterServiceError {
    #[error("repository error: {0}")]
    Repository(RepositoryError),
    #[error("dead letter not found: {0}")]
    NotFound(DeadLetterId),
    #[error("dead letter is terminal: {0}")]
    Terminal(DeadLetterId),
}

impl From<RepositoryError> for DeadLetterServiceError {
    fn from(err: RepositoryError) -> Self {
        DeadLetterServiceError::Repository(err)
    }
}

/// Operator-facing dead letter queue operations.
#[derive(Clone)]
pub struct DeadLetterService<DR>
where
    DR: DeadLetterRepository + ?Sized,
{
    dead_letter_repo: Arc<DR>,
}

impl<DR> DeadLetterService<DR>
where
    DR: DeadLetterRepository + ?Sized,
{
    /// Create a new service.
    pub fn new(dead_letter_repo: Arc<DR>) -> Self {
        Self { dead_letter_repo }
    }

    /// Enqueue a workflow item that needs operator attention.
    pub async fn enqueue(
        &self,
        organization_id: OrganizationId,
        project_id: Option<ProjectId>,
        source_type: DeadLetterSource,
        source_id: uuid::Uuid,
        reason: impl Into<String>,
        context: serde_json::Value,
    ) -> Result<DeadLetter, DeadLetterServiceError> {
        let entry = DeadLetter {
            id: DeadLetterId::new(),
            organization_id,
            project_id,
            source_type,
            source_id,
            reason: reason.into(),
            context,
            status: DeadLetterStatus::Open,
            created_at: Utc::now(),
            resolved_at: None,
        };
        Ok(self.dead_letter_repo.enqueue(&entry).await?)
    }

    /// List dead letters by status, optionally narrowing to a project.
    pub async fn list(
        &self,
        organization_id: OrganizationId,
        status: DeadLetterStatus,
        project_id: Option<ProjectId>,
    ) -> Result<Vec<DeadLetter>, DeadLetterServiceError> {
        let mut entries = self
            .dead_letter_repo
            .find_by_status(&organization_id, status)
            .await?;
        if let Some(project_id) = project_id {
            entries.retain(|entry| entry.project_id == Some(project_id));
        }
        Ok(entries)
    }

    /// Mark a dead letter as replayed. The actual source re-dispatch is owned
    /// by the caller that knows how to replay the source type.
    pub async fn replay(&self, id: DeadLetterId) -> Result<DeadLetter, DeadLetterServiceError> {
        self.move_non_terminal(id, DeadLetterStatus::Replayed, None)
            .await
    }

    /// Resolve a dead letter after manual repair.
    pub async fn resolve(&self, id: DeadLetterId) -> Result<DeadLetter, DeadLetterServiceError> {
        self.move_non_terminal(id, DeadLetterStatus::Resolved, Some(Utc::now()))
            .await
    }

    /// Ignore a dead letter intentionally.
    pub async fn ignore(&self, id: DeadLetterId) -> Result<DeadLetter, DeadLetterServiceError> {
        self.move_non_terminal(id, DeadLetterStatus::Ignored, Some(Utc::now()))
            .await
    }

    async fn move_non_terminal(
        &self,
        id: DeadLetterId,
        status: DeadLetterStatus,
        resolved_at: Option<chrono::DateTime<Utc>>,
    ) -> Result<DeadLetter, DeadLetterServiceError> {
        let current = self
            .dead_letter_repo
            .find_by_id(&id)
            .await?
            .ok_or(DeadLetterServiceError::NotFound(id))?;
        if current.status.is_terminal() {
            return Err(DeadLetterServiceError::Terminal(id));
        }
        Ok(self
            .dead_letter_repo
            .update_status(&id, status, resolved_at)
            .await?)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use adam_domain::workflow::dead_letter::{DeadLetterSource, DeadLetterStatus};
    use adam_domain::workflow::in_memory::InMemoryDeadLetterRepository;
    use adam_domain::workflow::repository::DeadLetterRepository;
    use adam_domain::{OrganizationId, ProjectId};
    use uuid::Uuid;

    use super::*;

    fn svc() -> (
        DeadLetterService<InMemoryDeadLetterRepository>,
        Arc<InMemoryDeadLetterRepository>,
    ) {
        let repo = Arc::new(InMemoryDeadLetterRepository::default());
        (DeadLetterService::new(repo.clone()), repo)
    }

    fn org() -> OrganizationId {
        OrganizationId::from_uuid(Uuid::nil())
    }

    fn project(id: u128) -> ProjectId {
        ProjectId::from_uuid(Uuid::from_u128(id))
    }

    #[tokio::test]
    async fn enqueue_creates_open_entry_with_context() {
        let (service, _repo) = svc();

        let entry = service
            .enqueue(
                org(),
                Some(project(1)),
                DeadLetterSource::Action,
                Uuid::from_u128(99),
                "non-compensable failure",
                serde_json::json!({"action_id":"a1","error":"boom"}),
            )
            .await
            .unwrap();

        assert_eq!(entry.status, DeadLetterStatus::Open);
        assert_eq!(entry.reason, "non-compensable failure");
        assert_eq!(entry.context["error"], "boom");
    }

    #[tokio::test]
    async fn list_filters_by_status_and_project() {
        let (service, _repo) = svc();
        service
            .enqueue(
                org(),
                Some(project(1)),
                DeadLetterSource::Action,
                Uuid::new_v4(),
                "first",
                serde_json::json!({}),
            )
            .await
            .unwrap();
        service
            .enqueue(
                org(),
                Some(project(2)),
                DeadLetterSource::Action,
                Uuid::new_v4(),
                "second",
                serde_json::json!({}),
            )
            .await
            .unwrap();

        let entries = service
            .list(org(), DeadLetterStatus::Open, Some(project(1)))
            .await
            .unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].project_id, Some(project(1)));
    }

    #[tokio::test]
    async fn replay_marks_open_entry_replayed_without_resolving() {
        let (service, _repo) = svc();
        let entry = service
            .enqueue(
                org(),
                None,
                DeadLetterSource::Event,
                Uuid::new_v4(),
                "retry later",
                serde_json::json!({}),
            )
            .await
            .unwrap();

        let replayed = service.replay(entry.id).await.unwrap();

        assert_eq!(replayed.status, DeadLetterStatus::Replayed);
        assert!(replayed.resolved_at.is_none());
    }

    #[tokio::test]
    async fn resolve_and_ignore_are_terminal_with_resolution_timestamp() {
        let (service, _repo) = svc();
        let resolved = service
            .enqueue(
                org(),
                None,
                DeadLetterSource::Action,
                Uuid::new_v4(),
                "fixed",
                serde_json::json!({}),
            )
            .await
            .unwrap();
        let ignored = service
            .enqueue(
                org(),
                None,
                DeadLetterSource::Action,
                Uuid::new_v4(),
                "accepted",
                serde_json::json!({}),
            )
            .await
            .unwrap();

        let resolved = service.resolve(resolved.id).await.unwrap();
        let ignored = service.ignore(ignored.id).await.unwrap();

        assert_eq!(resolved.status, DeadLetterStatus::Resolved);
        assert!(resolved.resolved_at.is_some());
        assert_eq!(ignored.status, DeadLetterStatus::Ignored);
        assert!(ignored.resolved_at.is_some());
    }

    #[tokio::test]
    async fn terminal_entry_cannot_be_replayed() {
        let (service, repo) = svc();
        let entry = service
            .enqueue(
                org(),
                None,
                DeadLetterSource::Action,
                Uuid::new_v4(),
                "done",
                serde_json::json!({}),
            )
            .await
            .unwrap();
        repo.update_status(&entry.id, DeadLetterStatus::Resolved, Some(Utc::now()))
            .await
            .unwrap();

        let err = service.replay(entry.id).await.unwrap_err();

        assert!(matches!(err, DeadLetterServiceError::Terminal(id) if id == entry.id));
    }
}
