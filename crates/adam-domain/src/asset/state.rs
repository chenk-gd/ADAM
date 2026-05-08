//! Asset state management module
//!
//! Defines the lifecycle states for assets and their valid transitions.

use serde::{Deserialize, Serialize};

/// Asset lifecycle states
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AssetState {
    /// Asset is up-to-date with upstream dependencies
    Clean,
    /// Upstream dependency has newer version, awaiting review/update
    Dirty,
    /// Asset no longer maintained, read-only
    Archived,
}

impl AssetState {
    /// Check if a transition to the target state is valid
    ///
    /// # State Transition Rules
    /// - Clean -> Dirty: When upstream dependency published
    /// - Clean -> Archived: Manual archival
    /// - Dirty -> Clean: Manual resolution
    /// - Dirty -> Archived: Manual archival while dirty
    /// - Archived -> *: No transitions allowed (terminal state)
    pub fn can_transition_to(&self, target: AssetState) -> bool {
        match (*self, target) {
            // Archived is terminal state - no transitions allowed
            (AssetState::Archived, _) => false,
            // Same state is always allowed (idempotent)
            (s, t) if s == t => true,
            // All other transitions are valid
            _ => true,
        }
    }

    /// Check if the asset is in Dirty state
    pub fn is_dirty(&self) -> bool {
        matches!(self, AssetState::Dirty)
    }

    /// Check if the asset is Archived
    pub fn is_archived(&self) -> bool {
        matches!(self, AssetState::Archived)
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
    }

    #[test]
    fn test_same_state_transition_is_allowed_except_archived() {
        assert!(AssetState::Clean.can_transition_to(AssetState::Clean));
        assert!(AssetState::Dirty.can_transition_to(AssetState::Dirty));
        // Archived is terminal - no transitions allowed, including self
        assert!(!AssetState::Archived.can_transition_to(AssetState::Archived));
    }
}
