//! Workflow automation domain errors.

use thiserror::Error;

/// Errors raised by the workflow automation domain.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WorkflowError {
    /// An illegal state transition was attempted.
    #[error("illegal {entity} transition: {from} -> {to}")]
    IllegalTransition {
        entity: &'static str,
        from: String,
        to: String,
    },

    /// A promotion rule conflict could not be resolved deterministically.
    #[error("unresolvable rule conflict for event {event_type}")]
    RuleConflict { event_type: String },

    /// Cascade depth limit exceeded; no action created.
    #[error("cascade depth exceeded: {depth} >= {max}")]
    CascadeDepthExceeded { depth: i32, max: i32 },

    /// A precondition for an action was not met.
    #[error("precondition unmet for action {action_type}: {detail}")]
    PreconditionUnmet { action_type: String, detail: String },

    /// Concurrent modification of a workflow entity (lock version mismatch).
    #[error("concurrent modification of {entity}")]
    ConcurrentModification { entity: &'static str },

    /// A referenced entity was not found.
    #[error("{entity} not found: {id}")]
    NotFound { entity: &'static str, id: String },

    /// Duplicate idempotency key; the operation was already applied.
    #[error("duplicate idempotency key: {0}")]
    DuplicateIdempotencyKey(String),

    /// An action cannot transition because it is blocked.
    #[error("action is blocked: {reason}")]
    Blocked { reason: String },

    /// Generic validation failure at the workflow boundary.
    #[error("validation error: {0}")]
    ValidationError(String),
}

impl WorkflowError {
    /// Whether this error represents a concurrent modification conflict.
    pub fn is_concurrent_modification(&self) -> bool {
        matches!(self, WorkflowError::ConcurrentModification { .. })
    }
}
