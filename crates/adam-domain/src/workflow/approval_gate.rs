//! Approval gates: human authorization checkpoints for actions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::asset::instance::OrganizationId;
use crate::workflow::action::WorkflowActionId;
use crate::workflow::state_machine::GateStatus;

/// Unique identifier for an approval gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ApprovalGateId(pub Uuid);

impl ApprovalGateId {
    /// Generate a new random id.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create from an existing UUID.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for ApprovalGateId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ApprovalGateId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Who must approve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApproverType {
    Role,
    User,
    Group,
}

impl ApproverType {
    /// Stable storage representation.
    pub fn as_str(self) -> &'static str {
        match self {
            ApproverType::Role => "role",
            ApproverType::User => "user",
            ApproverType::Group => "group",
        }
    }
}

/// A human authorization checkpoint for a workflow action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalGate {
    pub id: ApprovalGateId,
    pub organization_id: OrganizationId,
    pub action_id: WorkflowActionId,
    pub approver_type: ApproverType,
    pub approver_ref: String,
    pub status: GateStatus,
    pub decision_payload: Option<serde_json::Value>,
    pub deadline: Option<DateTime<Utc>>,
    pub decided_by: Option<String>,
    pub decided_at: Option<DateTime<Utc>>,
    pub lock_version: i64,
    pub created_at: DateTime<Utc>,
}

impl ApprovalGate {
    /// Whether the gate is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }

    /// Whether the gate has expired relative to `now`.
    pub fn is_expired_at(&self, now: DateTime<Utc>) -> bool {
        matches!(self.deadline, Some(d) if now > d)
    }
}

/// Command for creating a new approval gate.
#[derive(Debug, Clone)]
pub struct CreateApprovalGateCommand {
    pub organization_id: OrganizationId,
    pub action_id: WorkflowActionId,
    pub approver_type: ApproverType,
    pub approver_ref: String,
    pub deadline: Option<DateTime<Utc>>,
}
