//! Promotion rules: decide which events create which workflow actions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::asset::instance::{AssetTypeId, OrganizationId};
use crate::workflow::event::EventType;

/// Unique identifier for a promotion rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PromotionRuleId(pub Uuid);

impl PromotionRuleId {
    /// Generate a new random id.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create from an existing UUID.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for PromotionRuleId {
    fn default() -> Self {
        Self::new()
    }
}

/// The scope at which a rule applies. Evaluated most-specific-first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleScope {
    /// Applies to a specific asset type.
    AssetType,
    /// Applies within a specific project.
    Project,
    /// Applies across an organization.
    Organization,
}

impl RuleScope {
    /// Specificity rank; higher is more specific.
    pub fn specificity(self) -> u8 {
        match self {
            RuleScope::AssetType => 3,
            RuleScope::Project => 2,
            RuleScope::Organization => 1,
        }
    }

    /// Stable storage representation.
    pub fn as_str(self) -> &'static str {
        match self {
            RuleScope::AssetType => "asset_type",
            RuleScope::Project => "project",
            RuleScope::Organization => "organization",
        }
    }
}

/// How much automation a rule permits for the actions it creates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationLevel {
    /// Fully automatic, no human or agent in the loop.
    #[default]
    Automatic,
    /// Agent may propose; human reviews.
    AgentSuggested,
    /// Human approval required before execution.
    HumanApprovalRequired,
    /// Human performs the action entirely.
    HumanOnly,
}

impl AutomationLevel {
    /// Safety rank for tie-breaking; higher means safer (more human control).
    pub fn safety_rank(self) -> u8 {
        match self {
            AutomationLevel::Automatic => 1,
            AutomationLevel::AgentSuggested => 2,
            AutomationLevel::HumanApprovalRequired => 3,
            AutomationLevel::HumanOnly => 4,
        }
    }

    /// Stable storage representation.
    pub fn as_str(self) -> &'static str {
        match self {
            AutomationLevel::Automatic => "automatic",
            AutomationLevel::AgentSuggested => "agent_suggested",
            AutomationLevel::HumanApprovalRequired => "human_approval_required",
            AutomationLevel::HumanOnly => "human_only",
        }
    }
}

/// A declared group of mutually-exclusive rules for the same event and target.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MutexGroup(pub String);

impl MutexGroup {
    /// Create a new mutex group name.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
}

/// The type of workflow action a rule creates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    /// Create or update a work item.
    UpsertWorkItem,
    /// Create a bugfix work item from a pipeline failure.
    CreateBugfixWork,
    /// Create an agent task to build AI context.
    CreateVirtualAssetContext,
    /// Request a human approval gate.
    RequestApproval,
    /// Resolve a dirty dependency.
    ResolveDirty,
}

impl ActionType {
    /// Stable storage representation.
    pub fn as_str(self) -> &'static str {
        match self {
            ActionType::UpsertWorkItem => "upsert_work_item",
            ActionType::CreateBugfixWork => "create_bugfix_work",
            ActionType::CreateVirtualAssetContext => "create_virtual_asset_context",
            ActionType::RequestApproval => "request_approval",
            ActionType::ResolveDirty => "resolve_dirty",
        }
    }
}

impl std::str::FromStr for ActionType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "upsert_work_item" => Ok(ActionType::UpsertWorkItem),
            "create_bugfix_work" => Ok(ActionType::CreateBugfixWork),
            "create_virtual_asset_context" => Ok(ActionType::CreateVirtualAssetContext),
            "request_approval" => Ok(ActionType::RequestApproval),
            "resolve_dirty" => Ok(ActionType::ResolveDirty),
            other => Err(format!("unknown action type: {other}")),
        }
    }
}

impl std::fmt::Display for ActionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Template describing the action a rule creates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionTemplate {
    /// Action type to create.
    pub action_type: ActionType,
    /// JSON payload merged into the action (e.g. `work_item_kind`).
    pub payload: serde_json::Value,
    /// Whether the created action is required for the instance to complete.
    #[serde(default = "default_true")]
    pub is_required: bool,
    /// Ordering within the instance.
    #[serde(default)]
    pub order_index: i32,
}

fn default_true() -> bool {
    true
}

/// A rule that decides whether an event should create workflow actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromotionRule {
    pub id: PromotionRuleId,
    pub organization_id: OrganizationId,
    pub scope: RuleScope,
    /// Scope target: asset type id, project id, or None for organization scope.
    pub scope_ref: Option<Uuid>,
    pub event_type: EventType,
    /// Restrict the rule to events from this source asset type.
    pub source_asset_type_id: Option<AssetTypeId>,
    pub mutex_group: Option<MutexGroup>,
    pub rule_version: i32,
    pub priority: i32,
    pub automation_level: AutomationLevel,
    /// Top-level exact-match filter on the event payload.
    pub filters: serde_json::Value,
    /// Explicit preconditions; first implementation keeps these enumerable.
    pub preconditions: serde_json::Value,
    pub action_template: ActionTemplate,
    pub max_cascade_depth: i32,
    pub effective_from: Option<DateTime<Utc>>,
    pub effective_to: Option<DateTime<Utc>>,
    /// Rollout segment 0..=100 for staged rollout.
    pub rollout_segment: i32,
    pub enabled: bool,
    /// Dry-run: log evaluation but do not suppress active rules.
    pub dry_run: bool,
    /// Audit-only: never suppress active rules, always log.
    pub audit_only: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl PromotionRule {
    /// Whether the rule is in effect at the given time.
    pub fn is_effective_at(&self, now: DateTime<Utc>) -> bool {
        if !self.enabled {
            return false;
        }
        if let Some(from) = self.effective_from {
            if now < from {
                return false;
            }
        }
        if let Some(to) = self.effective_to {
            if now > to {
                return false;
            }
        }
        true
    }
}
