//! Dependency rules between asset types

use crate::asset::instance::AssetTypeId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for dependency rules
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DependencyRuleId(pub Uuid);

impl DependencyRuleId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Relationship type between asset types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RelationshipType {
    /// Direct dependency - downstream depends on upstream
    DependsOn,
    /// Reference - downstream references upstream (no dirty propagation)
    References,
}

/// Dependency rule between two asset types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyRule {
    pub id: DependencyRuleId,
    pub source_type_id: AssetTypeId, // downstream/dependent type
    pub target_type_id: AssetTypeId, // upstream/dependency type
    pub relationship: RelationshipType,
    pub is_transitive: bool, // for transitive query only, not dirty propagation
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl DependencyRule {
    pub fn new(
        source_type_id: AssetTypeId,
        target_type_id: AssetTypeId,
        relationship: RelationshipType,
        is_transitive: bool,
    ) -> Self {
        Self {
            id: DependencyRuleId::new(),
            source_type_id,
            target_type_id,
            relationship,
            is_transitive,
            created_at: chrono::Utc::now(),
        }
    }

    /// Check if this rule allows the given source-target type combination
    pub fn matches(&self, source_type: &AssetTypeId, target_type: &AssetTypeId) -> bool {
        &self.source_type_id == source_type && &self.target_type_id == target_type
    }
}

/// Repository trait for dependency rules
#[async_trait::async_trait]
pub trait DependencyRuleRepository: Send + Sync {
    /// Create a new dependency rule
    async fn create(&self, rule: &DependencyRule) -> Result<(), crate::RepositoryError>;

    /// Find all rules where source_type depends on any type
    async fn find_by_source_type(
        &self,
        type_id: &AssetTypeId,
    ) -> Result<Vec<DependencyRule>, crate::RepositoryError>;

    /// Find all rules that target a specific type
    async fn find_by_target_type(
        &self,
        type_id: &AssetTypeId,
    ) -> Result<Vec<DependencyRule>, crate::RepositoryError>;

    /// Check if dependency is allowed between types (for FR-004 validation)
    async fn is_dependency_allowed(
        &self,
        source_type: &AssetTypeId,
        target_type: &AssetTypeId,
    ) -> Result<bool, crate::RepositoryError>;

    /// Delete a dependency rule
    async fn delete(&self, rule_id: &DependencyRuleId) -> Result<(), crate::RepositoryError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dependency_rule_creation() {
        let source_id = AssetTypeId::new();
        let target_id = AssetTypeId::new();

        let rule = DependencyRule::new(
            source_id,
            target_id,
            RelationshipType::DependsOn,
            true,
        );

        assert_eq!(rule.source_type_id, source_id);
        assert_eq!(rule.target_type_id, target_id);
        assert_eq!(rule.relationship, RelationshipType::DependsOn);
        assert!(rule.is_transitive);
    }

    #[test]
    fn test_dependency_rule_matches() {
        let source_id = AssetTypeId::new();
        let target_id = AssetTypeId::new();

        let rule = DependencyRule::new(
            source_id,
            target_id,
            RelationshipType::DependsOn,
            false,
        );

        assert!(rule.matches(&source_id, &target_id));
        assert!(!rule.matches(&target_id, &source_id));
    }
}
