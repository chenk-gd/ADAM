//! Dead letter queue: events/actions that exhausted retries.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::asset::instance::{OrganizationId, ProjectId};

/// Unique identifier for a dead letter entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeadLetterId(pub Uuid);

impl DeadLetterId {
    /// Generate a new random id.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create from an existing UUID.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for DeadLetterId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for DeadLetterId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// What kind of source produced the dead letter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeadLetterSource {
    Event,
    Action,
    Instance,
}

impl DeadLetterSource {
    /// Stable storage representation.
    pub fn as_str(self) -> &'static str {
        match self {
            DeadLetterSource::Event => "event",
            DeadLetterSource::Action => "action",
            DeadLetterSource::Instance => "instance",
        }
    }
}

/// Lifecycle state of a dead letter entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeadLetterStatus {
    Open,
    Assigned,
    Replayed,
    Resolved,
    Ignored,
}

impl DeadLetterStatus {
    /// Stable storage representation.
    pub fn as_str(self) -> &'static str {
        match self {
            DeadLetterStatus::Open => "open",
            DeadLetterStatus::Assigned => "assigned",
            DeadLetterStatus::Replayed => "replayed",
            DeadLetterStatus::Resolved => "resolved",
            DeadLetterStatus::Ignored => "ignored",
        }
    }

    /// Whether the entry is in a terminal state.
    pub fn is_terminal(self) -> bool {
        matches!(self, DeadLetterStatus::Resolved | DeadLetterStatus::Ignored)
    }
}

/// A dead letter entry pending operator review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeadLetter {
    pub id: DeadLetterId,
    pub organization_id: OrganizationId,
    pub project_id: Option<ProjectId>,
    pub source_type: DeadLetterSource,
    pub source_id: Uuid,
    pub reason: String,
    pub context: serde_json::Value,
    pub status: DeadLetterStatus,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}
