//! State machines for workflow automation entities.
//!
//! Each state machine is a pure transition table. Transitions are validated
//! at the service layer; illegal transitions return [`WorkflowError`] and do
//! not emit workflow events. See design §5.

use serde::{Deserialize, Serialize};

use crate::workflow::error::WorkflowError;

/// A typed state machine: an enum of states plus a `can_transition_to` table.
pub trait StateMachine: Sized + Copy + std::fmt::Display {
    /// Human-readable entity name used in error messages.
    const ENTITY: &'static str;

    /// Whether transitioning to `target` is legal from `self`.
    fn can_transition_to(self, target: Self) -> bool;

    /// Validate a transition, returning an error if illegal.
    fn validate_transition(self, target: Self) -> Result<(), WorkflowError> {
        if self.can_transition_to(target) {
            Ok(())
        } else {
            Err(WorkflowError::IllegalTransition {
                entity: Self::ENTITY,
                from: self.to_string(),
                to: target.to_string(),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// WorkflowInstance
// ---------------------------------------------------------------------------

/// Lifecycle state of a [`crate::workflow::instance::WorkflowInstance`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceStatus {
    Pending,
    Ready,
    InProgress,
    Blocked,
    WaitingReview,
    WaitingValidation,
    Completed,
    Failed,
    Cancelled,
}

impl InstanceStatus {
    /// Whether this state is terminal (no outgoing transitions).
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            InstanceStatus::Completed | InstanceStatus::Failed | InstanceStatus::Cancelled
        )
    }
}

impl StateMachine for InstanceStatus {
    const ENTITY: &'static str = "workflow_instance";

    fn can_transition_to(self, target: Self) -> bool {
        use InstanceStatus::*;
        matches!(
            (self, target),
            (Pending, Ready)
                | (Pending, Cancelled)
                | (Ready, InProgress)
                | (Ready, Blocked)
                | (Ready, Cancelled)
                | (
                    InProgress,
                    Blocked | WaitingReview | WaitingValidation | Completed | Failed | Cancelled,
                )
                | (Blocked, Ready | Failed | Cancelled)
                | (WaitingReview, Ready | Completed | Failed | Cancelled)
                | (WaitingValidation, Ready | Completed | Failed | Cancelled)
        )
    }
}

impl std::fmt::Display for InstanceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            InstanceStatus::Pending => "pending",
            InstanceStatus::Ready => "ready",
            InstanceStatus::InProgress => "in_progress",
            InstanceStatus::Blocked => "blocked",
            InstanceStatus::WaitingReview => "waiting_review",
            InstanceStatus::WaitingValidation => "waiting_validation",
            InstanceStatus::Completed => "completed",
            InstanceStatus::Failed => "failed",
            InstanceStatus::Cancelled => "cancelled",
        };
        f.write_str(s)
    }
}

// ---------------------------------------------------------------------------
// WorkflowAction
// ---------------------------------------------------------------------------

/// Lifecycle state of a [`crate::workflow::action::WorkflowAction`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStatus {
    Pending,
    Ready,
    InProgress,
    WaitingApproval,
    Blocked,
    Succeeded,
    Failed,
    Cancelled,
    Skipped,
}

impl ActionStatus {
    /// Whether this state is terminal for the current action attempt.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            ActionStatus::Succeeded | ActionStatus::Cancelled | ActionStatus::Skipped
        )
    }

    /// Whether a failed action may be retried (returns to Pending).
    /// The actual retry budget check lives in the service layer.
    pub fn can_retry(self) -> bool {
        matches!(self, ActionStatus::Failed)
    }
}

impl StateMachine for ActionStatus {
    const ENTITY: &'static str = "workflow_action";

    fn can_transition_to(self, target: Self) -> bool {
        use ActionStatus::*;
        matches!(
            (self, target),
            (Pending, Ready | Blocked | Cancelled | Skipped)
                | (Ready, InProgress | WaitingApproval | Blocked | Cancelled | Skipped)
                | (InProgress, Succeeded | Failed | Blocked | WaitingApproval | Cancelled)
                | (WaitingApproval, Ready | Blocked | Failed | Cancelled)
                | (Blocked, Ready | Failed | Cancelled | Skipped)
                // Retry: Failed -> Pending only when retry budget remains.
                | (Failed, Pending)
        )
    }
}

impl std::fmt::Display for ActionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ActionStatus::Pending => "pending",
            ActionStatus::Ready => "ready",
            ActionStatus::InProgress => "in_progress",
            ActionStatus::WaitingApproval => "waiting_approval",
            ActionStatus::Blocked => "blocked",
            ActionStatus::Succeeded => "succeeded",
            ActionStatus::Failed => "failed",
            ActionStatus::Cancelled => "cancelled",
            ActionStatus::Skipped => "skipped",
        };
        f.write_str(s)
    }
}

// ---------------------------------------------------------------------------
// AgentTask
// ---------------------------------------------------------------------------

/// Lifecycle state of an [`crate::workflow::agent_task::AgentTask`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTaskStatus {
    Queued,
    Claimed,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Expired,
}

impl AgentTaskStatus {
    /// Whether this state is terminal.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            AgentTaskStatus::Succeeded
                | AgentTaskStatus::Failed
                | AgentTaskStatus::Cancelled
                | AgentTaskStatus::Expired
        )
    }
}

impl StateMachine for AgentTaskStatus {
    const ENTITY: &'static str = "agent_task";

    fn can_transition_to(self, target: Self) -> bool {
        use AgentTaskStatus::*;
        matches!(
            (self, target),
            (Queued, Claimed | Cancelled | Expired)
                | (Claimed, Running | Succeeded | Failed | Cancelled | Expired)
                | (Running, Succeeded | Failed | Cancelled | Expired)
        )
    }
}

impl std::fmt::Display for AgentTaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            AgentTaskStatus::Queued => "queued",
            AgentTaskStatus::Claimed => "claimed",
            AgentTaskStatus::Running => "running",
            AgentTaskStatus::Succeeded => "succeeded",
            AgentTaskStatus::Failed => "failed",
            AgentTaskStatus::Cancelled => "cancelled",
            AgentTaskStatus::Expired => "expired",
        };
        f.write_str(s)
    }
}

// ---------------------------------------------------------------------------
// ApprovalGate
// ---------------------------------------------------------------------------

/// Lifecycle state of an [`crate::workflow::approval_gate::ApprovalGate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateStatus {
    Pending,
    Approved,
    Rejected,
    Expired,
    Cancelled,
}

impl GateStatus {
    /// Whether this state is terminal.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            GateStatus::Approved
                | GateStatus::Rejected
                | GateStatus::Expired
                | GateStatus::Cancelled
        )
    }
}

impl StateMachine for GateStatus {
    const ENTITY: &'static str = "approval_gate";

    fn can_transition_to(self, target: Self) -> bool {
        use GateStatus::*;
        matches!(
            (self, target),
            (Pending, Approved | Rejected | Expired | Cancelled)
        )
    }
}

impl std::fmt::Display for GateStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            GateStatus::Pending => "pending",
            GateStatus::Approved => "approved",
            GateStatus::Rejected => "rejected",
            GateStatus::Expired => "expired",
            GateStatus::Cancelled => "cancelled",
        };
        f.write_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- InstanceStatus -----------------------------------------------------

    #[test]
    fn instance_pending_can_ready_or_cancel() {
        assert!(InstanceStatus::Pending.can_transition_to(InstanceStatus::Ready));
        assert!(InstanceStatus::Pending.can_transition_to(InstanceStatus::Cancelled));
        assert!(!InstanceStatus::Pending.can_transition_to(InstanceStatus::Completed));
    }

    #[test]
    fn instance_in_progress_reaches_all_driven_states() {
        for target in [
            InstanceStatus::Blocked,
            InstanceStatus::WaitingReview,
            InstanceStatus::WaitingValidation,
            InstanceStatus::Completed,
            InstanceStatus::Failed,
            InstanceStatus::Cancelled,
        ] {
            assert!(
                InstanceStatus::InProgress.can_transition_to(target),
                "expected in_progress -> {target}"
            );
        }
    }

    #[test]
    fn instance_terminal_states_have_no_outgoing() {
        for terminal in [
            InstanceStatus::Completed,
            InstanceStatus::Failed,
            InstanceStatus::Cancelled,
        ] {
            assert!(terminal.is_terminal());
            assert!(!terminal.can_transition_to(InstanceStatus::Ready));
        }
    }

    #[test]
    fn instance_illegal_transition_returns_error() {
        let err = InstanceStatus::Completed
            .validate_transition(InstanceStatus::Ready)
            .unwrap_err();
        assert!(matches!(err, WorkflowError::IllegalTransition { .. }));
        assert_eq!(
            err.to_string(),
            "illegal workflow_instance transition: completed -> ready"
        );
    }

    // --- ActionStatus -------------------------------------------------------

    #[test]
    fn action_pending_to_ready_blocked_cancelled_skipped() {
        for target in [
            ActionStatus::Ready,
            ActionStatus::Blocked,
            ActionStatus::Cancelled,
            ActionStatus::Skipped,
        ] {
            assert!(ActionStatus::Pending.can_transition_to(target));
        }
        assert!(!ActionStatus::Pending.can_transition_to(ActionStatus::Succeeded));
    }

    #[test]
    fn action_failed_can_retry_to_pending() {
        assert!(ActionStatus::Failed.can_transition_to(ActionStatus::Pending));
        assert!(ActionStatus::Failed.can_retry());
    }

    #[test]
    fn action_succeeded_is_terminal() {
        assert!(ActionStatus::Succeeded.is_terminal());
        assert!(!ActionStatus::Succeeded.can_transition_to(ActionStatus::Pending));
    }

    #[test]
    fn action_blocked_can_skip() {
        assert!(ActionStatus::Blocked.can_transition_to(ActionStatus::Skipped));
        assert!(ActionStatus::Blocked.can_transition_to(ActionStatus::Ready));
    }

    // --- AgentTaskStatus ----------------------------------------------------

    #[test]
    fn agent_task_queued_to_claimed_cancelled_expired() {
        for target in [
            AgentTaskStatus::Claimed,
            AgentTaskStatus::Cancelled,
            AgentTaskStatus::Expired,
        ] {
            assert!(AgentTaskStatus::Queued.can_transition_to(target));
        }
        // Cannot jump straight to succeeded from queued.
        assert!(!AgentTaskStatus::Queued.can_transition_to(AgentTaskStatus::Succeeded));
    }

    #[test]
    fn agent_task_claimed_may_skip_running() {
        assert!(AgentTaskStatus::Claimed.can_transition_to(AgentTaskStatus::Running));
        assert!(AgentTaskStatus::Claimed.can_transition_to(AgentTaskStatus::Succeeded));
    }

    #[test]
    fn agent_task_terminal_states() {
        for terminal in [
            AgentTaskStatus::Succeeded,
            AgentTaskStatus::Failed,
            AgentTaskStatus::Cancelled,
            AgentTaskStatus::Expired,
        ] {
            assert!(terminal.is_terminal());
        }
    }

    // --- GateStatus ---------------------------------------------------------

    #[test]
    fn gate_pending_to_all_terminals() {
        for target in [
            GateStatus::Approved,
            GateStatus::Rejected,
            GateStatus::Expired,
            GateStatus::Cancelled,
        ] {
            assert!(GateStatus::Pending.can_transition_to(target));
        }
    }

    #[test]
    fn gate_approved_is_terminal() {
        assert!(GateStatus::Approved.is_terminal());
        assert!(!GateStatus::Approved.can_transition_to(GateStatus::Pending));
    }
}
