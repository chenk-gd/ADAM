//! Workflow automation application services (Slice 1: event → action core).
//!
//! Services orchestrate the domain workflow entities:
//! - [`event_service::WorkflowEventService`] appends events idempotently.
//! - [`rule_evaluator::PromotionRuleEvaluator`] evaluates enabled rules and
//!   idempotently creates workflow actions.
//! - [`instance_service::WorkflowInstanceService`] manages instance lifecycle.
//! - [`action_service::WorkflowActionService`] drives action state transitions.
//!
//! Transaction boundaries follow design §10: event append + rule evaluation +
//! action creation happen in a single logical transaction (here expressed via
//! repository call ordering; the Postgres implementation wraps them in one
//! `BEGIN/COMMIT`). Idempotency is enforced by unique keys with reload-on-
//! conflict semantics.

pub mod action_service;
pub mod agent_task_service;
pub mod approval_gate_service;
pub mod dead_letter_service;
pub mod event_service;
pub mod instance_service;
pub mod rule_evaluator;
mod rule_matching;

pub use action_service::{WorkflowActionService, WorkflowActionServiceError};
pub use agent_task_service::{AgentTaskService, AgentTaskServiceError};
pub use approval_gate_service::{ApprovalGateService, ApprovalGateServiceError, GateDecision};
pub use dead_letter_service::{DeadLetterService, DeadLetterServiceError};
pub use event_service::{AppendEventRequest, WorkflowEventService, WorkflowEventServiceError};
pub use instance_service::{WorkflowInstanceService, WorkflowInstanceServiceError};
pub use rule_evaluator::{EvaluationOutcome, PromotionRuleEvaluator, RuleEvaluatorError};

use std::sync::Arc;

/// Time port so tests can control timestamps deterministically.
pub trait Clock: Send + Sync {
    fn now(&self) -> chrono::DateTime<chrono::Utc>;
}

/// Production clock using the system wall clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now()
    }
}

/// Shared handle type for a clock.
pub type ClockRef = Arc<dyn Clock>;
