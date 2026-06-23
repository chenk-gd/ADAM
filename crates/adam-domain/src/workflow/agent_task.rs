//! Agent tasks: executable units claimed by AI agents.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::asset::instance::{OrganizationId, ProjectId};
use crate::workflow::action::WorkflowActionId;
use crate::workflow::state_machine::AgentTaskStatus;

/// Unique identifier for an agent task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentTaskId(pub Uuid);

impl AgentTaskId {
    /// Generate a new random id.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create from an existing UUID.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for AgentTaskId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for AgentTaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The capability an agent must have to claim a task.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Capability(pub String);

impl Capability {
    /// Create a new capability name.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// The capability used to build AI context for a work item.
    pub fn create_virtual_asset_context() -> Self {
        Self("create_virtual_asset_context".to_string())
    }
}

/// A task an AI agent can claim and execute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTask {
    pub id: AgentTaskId,
    pub organization_id: OrganizationId,
    pub project_id: Option<ProjectId>,
    pub action_id: WorkflowActionId,
    pub capability: Capability,
    pub status: AgentTaskStatus,
    pub agent_id: Option<String>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub result_payload: Option<serde_json::Value>,
    pub produced_asset_ids: Vec<Uuid>,
    pub lock_version: i64,
    pub idempotency_key: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl AgentTask {
    /// Whether the task is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }

    /// Whether the task has expired relative to `now`.
    pub fn is_expired_at(&self, now: DateTime<Utc>) -> bool {
        matches!(self.expires_at, Some(exp) if now > exp)
    }
}

/// Command for creating a new agent task.
#[derive(Debug, Clone)]
pub struct CreateAgentTaskCommand {
    pub organization_id: OrganizationId,
    pub project_id: Option<ProjectId>,
    pub action_id: WorkflowActionId,
    pub capability: Capability,
    pub idempotency_key: String,
    pub expires_at: Option<DateTime<Utc>>,
}
