//! Workflow instances: Saga coordinators for chains of actions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::asset::instance::{OrganizationId, ProjectId};
use crate::workflow::event::CorrelationId;
use crate::workflow::state_machine::InstanceStatus;

/// Unique identifier for a workflow instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowInstanceId(pub Uuid);

impl WorkflowInstanceId {
    /// Generate a new random id.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create from an existing UUID.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for WorkflowInstanceId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for WorkflowInstanceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A named workflow template (feature, bugfix, test execution, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowTemplate {
    Feature,
    Bugfix,
    TestExecution,
}

impl WorkflowTemplate {
    /// Stable storage representation.
    pub fn as_str(self) -> &'static str {
        match self {
            WorkflowTemplate::Feature => "feature",
            WorkflowTemplate::Bugfix => "bugfix",
            WorkflowTemplate::TestExecution => "test_execution",
        }
    }
}

impl std::fmt::Display for WorkflowTemplate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A Saga coordinator for a chain of workflow actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowInstance {
    pub id: WorkflowInstanceId,
    pub organization_id: OrganizationId,
    pub project_id: Option<ProjectId>,
    pub correlation_id: CorrelationId,
    pub template: WorkflowTemplate,
    pub status: InstanceStatus,
    pub cascade_depth: i32,
    pub lock_version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl WorkflowInstance {
    /// Whether the instance is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }
}

/// Command for creating a new workflow instance.
#[derive(Debug, Clone)]
pub struct CreateInstanceCommand {
    pub organization_id: OrganizationId,
    pub project_id: Option<ProjectId>,
    pub correlation_id: CorrelationId,
    pub template: WorkflowTemplate,
    pub cascade_depth: i32,
}
