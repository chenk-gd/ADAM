//! Workflow actions: individual steps within a workflow instance.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::asset::instance::{AssetId, AssetTypeId, OrganizationId};
use crate::workflow::instance::WorkflowInstanceId;
use crate::workflow::rule::{ActionType, AutomationLevel};
use crate::workflow::state_machine::ActionStatus;

/// Unique identifier for a workflow action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowActionId(pub Uuid);

impl WorkflowActionId {
    /// Generate a new random id.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create from an existing UUID.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for WorkflowActionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for WorkflowActionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Why an action is blocked. Mirrors design §8 blocked reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockedReason {
    MissingDependency,
    DirtyDependency,
    WaitingApproval,
    PipelineFailed,
    PolicyDenied,
    ExecutorUnavailable,
    ExternalSystemUnavailable,
    WaitingManualIntervention,
}

impl BlockedReason {
    /// Stable storage representation.
    pub fn as_str(self) -> &'static str {
        match self {
            BlockedReason::MissingDependency => "missing_dependency",
            BlockedReason::DirtyDependency => "dirty_dependency",
            BlockedReason::WaitingApproval => "waiting_approval",
            BlockedReason::PipelineFailed => "pipeline_failed",
            BlockedReason::PolicyDenied => "policy_denied",
            BlockedReason::ExecutorUnavailable => "executor_unavailable",
            BlockedReason::ExternalSystemUnavailable => "external_system_unavailable",
            BlockedReason::WaitingManualIntervention => "waiting_manual_intervention",
        }
    }
}

/// A single step within a workflow instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowAction {
    pub id: WorkflowActionId,
    pub organization_id: OrganizationId,
    pub instance_id: WorkflowInstanceId,
    pub action_type: ActionType,
    pub target_asset_id: Option<AssetId>,
    pub target_asset_type_id: Option<AssetTypeId>,
    pub status: ActionStatus,
    pub lock_version: i64,
    pub idempotency_key: String,
    pub preconditions: serde_json::Value,
    pub postconditions: serde_json::Value,
    pub automation_level: AutomationLevel,
    pub is_required: bool,
    pub order_index: i32,
    pub compensation_action_type: Option<ActionType>,
    pub compensation_payload: Option<serde_json::Value>,
    pub compensation_policy: CompensationPolicy,
    pub retry_count: i32,
    pub max_retries: i32,
    pub next_retry_at: Option<DateTime<Utc>>,
    pub blocked_reason: Option<BlockedReason>,
    pub result_payload: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl WorkflowAction {
    /// Whether this action still has retry budget.
    pub fn can_retry(&self) -> bool {
        self.retry_count < self.max_retries
    }

    /// Whether the action is in a terminal state for the current attempt.
    pub fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }
}

/// Compensation policy for a side-effecting action (design §8 Saga).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompensationPolicy {
    /// No compensation needed.
    #[default]
    None,
    /// Best-effort compensation; failure is logged but not fatal.
    BestEffort,
    /// Compensation must succeed before the instance can fail.
    RequiredBeforeFail,
    /// Only a human can compensate; block for manual intervention.
    ManualOnly,
}

impl CompensationPolicy {
    /// Stable storage representation.
    pub fn as_str(self) -> &'static str {
        match self {
            CompensationPolicy::None => "none",
            CompensationPolicy::BestEffort => "best_effort",
            CompensationPolicy::RequiredBeforeFail => "required_before_fail",
            CompensationPolicy::ManualOnly => "manual_only",
        }
    }
}

/// Command for creating a new workflow action.
#[derive(Debug, Clone)]
pub struct CreateActionCommand {
    pub organization_id: OrganizationId,
    pub instance_id: WorkflowInstanceId,
    pub action_type: ActionType,
    pub target_asset_id: Option<AssetId>,
    pub target_asset_type_id: Option<AssetTypeId>,
    pub idempotency_key: String,
    pub preconditions: serde_json::Value,
    pub postconditions: serde_json::Value,
    pub automation_level: AutomationLevel,
    pub is_required: bool,
    pub order_index: i32,
    pub compensation_action_type: Option<ActionType>,
    pub compensation_payload: Option<serde_json::Value>,
    pub compensation_policy: CompensationPolicy,
    pub max_retries: i32,
}
