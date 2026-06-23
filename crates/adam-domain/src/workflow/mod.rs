//! Workflow automation domain module.
//!
//! Implements the asset-driven workflow automation design (see
//! `docs/plans/2026-06-15-asset-driven-workflow-automation-design.md`):
//! events, promotion rules, workflow instances, actions, agent tasks,
//! approval gates, compensation, and the dead letter queue.

pub mod action;
pub mod agent_task;
pub mod approval_gate;
pub mod conflict;
pub mod dead_letter;
pub mod error;
pub mod event;
pub mod idempotency;
pub mod in_memory;
pub mod instance;
pub mod repository;
pub mod rule;
pub mod state_machine;

pub use action::{
    BlockedReason, CompensationPolicy, CreateActionCommand, WorkflowAction, WorkflowActionId,
};
pub use agent_task::{AgentTask, AgentTaskId, Capability, CreateAgentTaskCommand};
pub use approval_gate::{ApprovalGate, ApprovalGateId, ApproverType, CreateApprovalGateCommand};
pub use conflict::{ConflictResolution, resolve as resolve_conflicts};
pub use dead_letter::{DeadLetter, DeadLetterId, DeadLetterSource, DeadLetterStatus};
pub use error::WorkflowError;
pub use event::{CorrelationId, EventType, WorkflowEvent, WorkflowEventId};
pub use idempotency::{
    action_idempotency_key, agent_task_idempotency_key, event_idempotency_key,
    instance_action_idempotency_key,
};
pub use instance::{CreateInstanceCommand, WorkflowInstance, WorkflowInstanceId, WorkflowTemplate};
pub use repository::{
    AgentTaskRepository, ApprovalGateRepository, DeadLetterRepository, PromotionRuleRepository,
    WorkflowActionRepository, WorkflowEventRepository, WorkflowInstanceRepository,
};
pub use rule::{
    ActionTemplate, ActionType, AutomationLevel, MutexGroup, PromotionRule, PromotionRuleId,
    RuleScope,
};
pub use state_machine::{ActionStatus, AgentTaskStatus, GateStatus, InstanceStatus, StateMachine};
