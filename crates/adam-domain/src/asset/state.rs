//! Asset state management module
//!
//! Defines the lifecycle states for assets and their valid transitions.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors that can occur in state operations
#[derive(Debug, Error, Clone, PartialEq)]
pub enum StateError {
    /// Invalid state transition
    #[error("Invalid state transition: cannot transition from {from} to {to}")]
    InvalidTransition {
        /// Current state
        from: AssetState,
        /// Target state
        to: AssetState,
    },
    /// Operation not allowed in current state
    #[error("Operation not allowed in state {state}")]
    OperationNotAllowed {
        /// Current state
        state: AssetState,
    },
}

/// Asset lifecycle states
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AssetState {
    /// Asset is up-to-date with upstream dependencies
    Clean,
    /// Upstream dependency has newer version, awaiting review/update
    Dirty,
    /// Asset no longer maintained, read-only
    Archived,
    /// Immutable asset (code_commit, pipeline_run) - final state, no transitions
    Final,
}

impl AssetState {
    /// Check if a transition to the target state is valid
    ///
    /// # State Transition Rules
    /// - Clean -> Dirty: When upstream dependency published
    /// - Clean -> Archived: Manual archival
    /// - Clean -> Clean: Idempotent
    /// - Dirty -> Clean: Manual resolution
    /// - Dirty -> Archived: Manual archival while dirty
    /// - Archived -> *: No transitions allowed (terminal state)
    /// - Final -> *: No transitions allowed (immutable, terminal state)
    pub fn can_transition_to(&self, target: AssetState) -> bool {
        match (*self, target) {
            // Archived is terminal state - no transitions allowed
            (AssetState::Archived, _) => false,
            // Final is terminal state for immutable assets - no transitions allowed
            (AssetState::Final, _) => false,
            // Same state is always allowed (idempotent)
            (s, t) if s == t => true,
            // All other transitions are valid
            _ => true,
        }
    }

    /// Validate transition and return error if invalid
    pub fn validate_transition(&self, target: AssetState) -> Result<(), StateError> {
        if self.can_transition_to(target) {
            Ok(())
        } else {
            Err(StateError::InvalidTransition {
                from: *self,
                to: target,
            })
        }
    }

    /// Check if the asset can be published
    /// Publishing is allowed for Clean and Dirty states
    /// Final and Archived assets cannot be published (they are immutable)
    pub fn can_publish(&self) -> bool {
        matches!(self, AssetState::Clean | AssetState::Dirty)
    }

    /// Check if the asset can be depended upon
    /// All states can be depended upon except Archived
    pub fn can_depend(&self) -> bool {
        !self.is_archived()
    }

    /// Check if the asset is in Dirty state
    pub fn is_dirty(&self) -> bool {
        matches!(self, AssetState::Dirty)
    }

    /// Check if the asset is Archived
    pub fn is_archived(&self) -> bool {
        matches!(self, AssetState::Archived)
    }

    /// Check if the asset is in Final state (immutable)
    pub fn is_final(&self) -> bool {
        matches!(self, AssetState::Final)
    }

    /// Check if the asset can receive dirty propagation
    /// Only Clean assets can become Dirty
    /// Final assets are immutable and never become Dirty
    pub fn can_receive_dirty(&self) -> bool {
        matches!(self, AssetState::Clean)
    }
}

impl std::fmt::Display for AssetState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssetState::Clean => write!(f, "Clean"),
            AssetState::Dirty => write!(f, "Dirty"),
            AssetState::Archived => write!(f, "Archived"),
            AssetState::Final => write!(f, "Final"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_can_transition_clean_to_dirty() {
        assert!(AssetState::Clean.can_transition_to(AssetState::Dirty));
    }

    #[test]
    fn test_can_transition_dirty_to_clean() {
        assert!(AssetState::Dirty.can_transition_to(AssetState::Clean));
    }

    #[test]
    fn test_cannot_transition_from_archived() {
        assert!(!AssetState::Archived.can_transition_to(AssetState::Clean));
        assert!(!AssetState::Archived.can_transition_to(AssetState::Dirty));
        assert!(!AssetState::Archived.can_transition_to(AssetState::Archived));
        assert!(!AssetState::Archived.can_transition_to(AssetState::Final));
    }

    #[test]
    fn test_cannot_transition_from_final() {
        // Final is terminal state for immutable assets
        assert!(!AssetState::Final.can_transition_to(AssetState::Clean));
        assert!(!AssetState::Final.can_transition_to(AssetState::Dirty));
        assert!(!AssetState::Final.can_transition_to(AssetState::Archived));
        assert!(!AssetState::Final.can_transition_to(AssetState::Final));
    }

    #[test]
    fn test_same_state_transition_is_allowed_except_archived_and_final() {
        assert!(AssetState::Clean.can_transition_to(AssetState::Clean));
        assert!(AssetState::Dirty.can_transition_to(AssetState::Dirty));
        // Archived is terminal - no transitions allowed, including self
        assert!(!AssetState::Archived.can_transition_to(AssetState::Archived));
        // Final is terminal - no transitions allowed, including self
        assert!(!AssetState::Final.can_transition_to(AssetState::Final));
    }

    #[test]
    fn test_can_publish() {
        assert!(AssetState::Clean.can_publish());
        assert!(AssetState::Dirty.can_publish());
        assert!(!AssetState::Archived.can_publish());
        // Final assets cannot be published (they are immutable)
        assert!(!AssetState::Final.can_publish());
    }

    #[test]
    fn test_can_depend() {
        assert!(AssetState::Clean.can_depend());
        assert!(AssetState::Dirty.can_depend());
        assert!(!AssetState::Archived.can_depend());
        // Final assets can be depended upon
        assert!(AssetState::Final.can_depend());
    }

    #[test]
    fn test_is_final() {
        assert!(!AssetState::Clean.is_final());
        assert!(!AssetState::Dirty.is_final());
        assert!(!AssetState::Archived.is_final());
        assert!(AssetState::Final.is_final());
    }

    #[test]
    fn test_can_receive_dirty() {
        // Only Clean assets can receive dirty propagation
        assert!(AssetState::Clean.can_receive_dirty());
        assert!(!AssetState::Dirty.can_receive_dirty());
        assert!(!AssetState::Archived.can_receive_dirty());
        // Final assets are immutable and never become Dirty
        assert!(!AssetState::Final.can_receive_dirty());
    }

    #[test]
    fn test_validate_transition_success() {
        assert!(
            AssetState::Clean
                .validate_transition(AssetState::Dirty)
                .is_ok()
        );
        assert!(
            AssetState::Dirty
                .validate_transition(AssetState::Clean)
                .is_ok()
        );
        assert!(
            AssetState::Clean
                .validate_transition(AssetState::Clean)
                .is_ok()
        );
    }

    #[test]
    fn test_validate_transition_failure() {
        let result = AssetState::Archived.validate_transition(AssetState::Clean);
        assert!(result.is_err());
        match result {
            Err(StateError::InvalidTransition { from, to }) => {
                assert_eq!(from, AssetState::Archived);
                assert_eq!(to, AssetState::Clean);
            }
            _ => panic!("Expected InvalidTransition error"),
        }
    }

    #[test]
    fn test_state_error_display() {
        let error = StateError::InvalidTransition {
            from: AssetState::Archived,
            to: AssetState::Clean,
        };
        assert!(error.to_string().contains("Archived"));
        assert!(error.to_string().contains("Clean"));
    }
}
