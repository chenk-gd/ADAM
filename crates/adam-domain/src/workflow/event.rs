//! Workflow events: the append-only log of domain events that trigger actions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::asset::instance::{AssetId, AssetTypeId, OrganizationId, ProjectId};
use crate::workflow::idempotency::event_idempotency_key;

/// Correlation identifier that links all events/actions in one workflow chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CorrelationId(pub Uuid);

impl CorrelationId {
    /// Generate a new random correlation id.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create from an existing UUID.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for CorrelationId {
    fn default() -> Self {
        Self::new()
    }
}

/// The kind of domain event that can trigger workflow automation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    /// An asset version was published.
    AssetPublished,
    /// A dirty dependency was manually resolved.
    DirtyResolved,
    /// A pipeline run failed.
    PipelineFailed,
    /// A workflow action succeeded.
    ActionSucceeded,
    /// A workflow action failed.
    ActionFailed,
    /// An approval gate was granted.
    ApprovalGranted,
    /// An approval gate was rejected.
    ApprovalRejected,
}

impl EventType {
    /// Stable storage representation.
    pub fn as_str(self) -> &'static str {
        match self {
            EventType::AssetPublished => "asset_published",
            EventType::DirtyResolved => "dirty_resolved",
            EventType::PipelineFailed => "pipeline_failed",
            EventType::ActionSucceeded => "action_succeeded",
            EventType::ActionFailed => "action_failed",
            EventType::ApprovalGranted => "approval_granted",
            EventType::ApprovalRejected => "approval_rejected",
        }
    }
}

impl std::fmt::Display for EventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Unique identifier for a workflow event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowEventId(pub Uuid);

impl WorkflowEventId {
    /// Generate a new random id.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create from an existing UUID.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for WorkflowEventId {
    fn default() -> Self {
        Self::new()
    }
}

/// An immutable record of something that happened in the asset domain.
///
/// Events are append-only and identified by an idempotency key derived from
/// the event type, source asset, and an external request token. Replaying the
/// same event must not create duplicate workflow actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowEvent {
    pub id: WorkflowEventId,
    pub organization_id: OrganizationId,
    pub project_id: Option<ProjectId>,
    pub correlation_id: CorrelationId,
    pub event_type: EventType,
    pub source_asset_id: AssetId,
    pub source_asset_type_id: AssetTypeId,
    pub payload: serde_json::Value,
    /// Cascade depth from the originating event (0 for external events).
    pub cascade_depth: i32,
    /// Action that triggered this event, if internally generated.
    pub triggering_action_id: Option<Uuid>,
    /// Stored idempotency key; the final guard against duplicate events.
    pub idempotency_key: String,
    pub created_at: DateTime<Utc>,
}

impl WorkflowEvent {
    /// Compute the idempotency key for an event from its type, source, and
    /// request token. Key shape: `{event_type}:{source_asset_id}:{request_token}`.
    pub fn derive_idempotency_key(&self, request_token: &str) -> String {
        event_idempotency_key(self.event_type, self.source_asset_id, request_token)
    }
}
