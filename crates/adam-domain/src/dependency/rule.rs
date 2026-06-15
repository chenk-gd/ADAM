//! Dependency rules between asset types

use crate::asset::instance::AssetTypeId;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

/// Unique identifier for dependency rules
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DependencyRuleId(pub Uuid);

impl DependencyRuleId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for DependencyRuleId {
    fn default() -> Self {
        Self::new()
    }
}

/// Relationship type between asset types
///
/// Describes *why* one asset is connected to another. The relationship itself
/// does not decide Dirty propagation — that is controlled by `PropagationPolicy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipType {
    /// Direct dependency — downstream depends on upstream
    DependsOn,
    /// Reference — downstream references upstream for context
    References,
    /// Downstream implements the upstream requirement or specification
    Implements,
    /// Downstream fixes the upstream defect or issue
    Fixes,
    /// Downstream verifies the upstream artifact (e.g., test verifies requirement)
    Verifies,
    /// Downstream executes the upstream artifact (e.g., test execution runs test case)
    Executes,
    /// Downstream produces the upstream as output (e.g., test report from execution)
    Produces,
    /// Downstream blocks the upstream from proceeding
    Blocks,
    /// Loose correlation without semantic commitment
    RelatesTo,
}

impl RelationshipType {
    /// Return the stable snake_case string used in API and database boundaries.
    pub fn as_str(self) -> &'static str {
        match self {
            RelationshipType::DependsOn => "depends_on",
            RelationshipType::References => "references",
            RelationshipType::Implements => "implements",
            RelationshipType::Fixes => "fixes",
            RelationshipType::Verifies => "verifies",
            RelationshipType::Executes => "executes",
            RelationshipType::Produces => "produces",
            RelationshipType::Blocks => "blocks",
            RelationshipType::RelatesTo => "relates_to",
        }
    }

    /// Default propagation policy when no explicit policy or matching rule is provided.
    pub fn default_propagation_policy(self) -> PropagationPolicy {
        match self {
            RelationshipType::DependsOn
            | RelationshipType::Implements
            | RelationshipType::Fixes
            | RelationshipType::Verifies => PropagationPolicy::Dirty,
            RelationshipType::References | RelationshipType::Executes => {
                PropagationPolicy::ContextOnly
            }
            RelationshipType::Produces | RelationshipType::Blocks | RelationshipType::RelatesTo => {
                PropagationPolicy::AuditOnly
            }
        }
    }
}

impl fmt::Display for RelationshipType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when parsing an unknown relationship string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown relationship type: {0}")]
pub struct UnknownRelationshipType(String);

impl FromStr for RelationshipType {
    type Err = UnknownRelationshipType;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "depends_on" => Ok(RelationshipType::DependsOn),
            "references" => Ok(RelationshipType::References),
            "implements" => Ok(RelationshipType::Implements),
            "fixes" => Ok(RelationshipType::Fixes),
            "verifies" => Ok(RelationshipType::Verifies),
            "executes" => Ok(RelationshipType::Executes),
            "produces" => Ok(RelationshipType::Produces),
            "blocks" => Ok(RelationshipType::Blocks),
            "relates_to" => Ok(RelationshipType::RelatesTo),
            _ => Err(UnknownRelationshipType(s.to_string())),
        }
    }
}

/// Propagation policy controlling how upstream publishes affect downstream assets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropagationPolicy {
    /// Upstream publish can mark downstream Dirty.
    #[default]
    Dirty,
    /// Usable for AI context and graph traversal; never creates Dirty queue entries.
    ContextOnly,
    /// Preserved for traceability; no Dirty propagation and excluded from default context expansion.
    AuditOnly,
}

impl PropagationPolicy {
    /// Return the stable snake_case string used in API and database boundaries.
    pub fn as_str(self) -> &'static str {
        match self {
            PropagationPolicy::Dirty => "dirty",
            PropagationPolicy::ContextOnly => "context_only",
            PropagationPolicy::AuditOnly => "audit_only",
        }
    }

    /// Whether this policy can mark a downstream asset Dirty.
    pub fn triggers_dirty(self) -> bool {
        matches!(self, PropagationPolicy::Dirty)
    }
}

impl fmt::Display for PropagationPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when parsing an unknown propagation policy string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown propagation policy: {0}")]
pub struct UnknownPropagationPolicy(String);

impl FromStr for PropagationPolicy {
    type Err = UnknownPropagationPolicy;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "dirty" => Ok(PropagationPolicy::Dirty),
            "context_only" => Ok(PropagationPolicy::ContextOnly),
            "audit_only" => Ok(PropagationPolicy::AuditOnly),
            _ => Err(UnknownPropagationPolicy(s.to_string())),
        }
    }
}

/// Dependency rule between two asset types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyRule {
    pub id: DependencyRuleId,
    pub source_type_id: AssetTypeId, // downstream/dependent type
    pub target_type_id: AssetTypeId, // upstream/dependency type
    pub relationship: RelationshipType,
    pub is_transitive: bool, // for transitive query only, not dirty propagation
    /// Optional filter on source asset metadata keys for rule specificity.
    /// Only exact top-level key equality is supported in this iteration.
    #[serde(default)]
    pub source_metadata_filter: Option<serde_json::Value>,
    /// Optional filter on target asset metadata keys for rule specificity.
    #[serde(default)]
    pub target_metadata_filter: Option<serde_json::Value>,
    /// Propagation policy inferred from this rule when it matches.
    #[serde(default)]
    pub propagation_policy: PropagationPolicy,
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
            source_metadata_filter: None,
            target_metadata_filter: None,
            propagation_policy: relationship.default_propagation_policy(),
            created_at: chrono::Utc::now(),
        }
    }

    /// Check if this rule allows the given source-target type combination
    pub fn matches(&self, source_type: &AssetTypeId, target_type: &AssetTypeId) -> bool {
        &self.source_type_id == source_type && &self.target_type_id == target_type
    }

    /// Check if this rule matches the given types AND their metadata satisfies the filters.
    pub fn matches_with_metadata(
        &self,
        source_type: &AssetTypeId,
        source_metadata: &serde_json::Value,
        target_type: &AssetTypeId,
        target_metadata: &serde_json::Value,
    ) -> bool {
        self.matches(source_type, target_type)
            && metadata_matches(&self.source_metadata_filter, source_metadata)
            && metadata_matches(&self.target_metadata_filter, target_metadata)
    }

    /// Specificity score based on number of filter keys.
    /// More filter keys = more specific rule. Used to pick the best rule when multiple match.
    pub fn specificity(&self) -> usize {
        let source_keys = self
            .source_metadata_filter
            .as_ref()
            .and_then(|v| v.as_object())
            .map(|o| o.len())
            .unwrap_or(0);
        let target_keys = self
            .target_metadata_filter
            .as_ref()
            .and_then(|v| v.as_object())
            .map(|o| o.len())
            .unwrap_or(0);
        source_keys + target_keys
    }

    /// Builder: set source metadata filter.
    pub fn with_source_metadata_filter(mut self, filter: serde_json::Value) -> Self {
        self.source_metadata_filter = Some(filter);
        self
    }

    /// Builder: set target metadata filter.
    pub fn with_target_metadata_filter(mut self, filter: serde_json::Value) -> Self {
        self.target_metadata_filter = Some(filter);
        self
    }

    /// Builder: override propagation policy.
    pub fn with_propagation_policy(mut self, policy: PropagationPolicy) -> Self {
        self.propagation_policy = policy;
        self
    }
}

/// Check whether all keys in `filter` are present in `metadata` with the same values.
/// If `filter` is `None`, it always matches (no filter = wildcard).
fn metadata_matches(filter: &Option<serde_json::Value>, metadata: &serde_json::Value) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    let Some(filter_obj) = filter.as_object() else {
        return false;
    };
    for (key, expected) in filter_obj {
        if metadata.get(key) != Some(expected) {
            return false;
        }
    }
    true
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
    /// Default implementation delegates to `find_allowed_type_rules`.
    async fn is_dependency_allowed(
        &self,
        source_type: &AssetTypeId,
        target_type: &AssetTypeId,
    ) -> Result<bool, crate::RepositoryError> {
        let rules = self
            .find_allowed_type_rules(source_type, target_type)
            .await?;
        Ok(!rules.is_empty())
    }

    /// Delete a dependency rule
    async fn delete(&self, rule_id: &DependencyRuleId) -> Result<(), crate::RepositoryError>;

    /// Find all type-level rules matching the given source-target type pair.
    /// This is the legality gate — if no type-level rule exists, the dependency is not allowed.
    async fn find_allowed_type_rules(
        &self,
        source_type: &AssetTypeId,
        target_type: &AssetTypeId,
    ) -> Result<Vec<DependencyRule>, crate::RepositoryError>;

    /// Find rules matching both type pair and metadata filters.
    /// Used for policy inference — the most specific matching rule supplies
    /// relationship and propagation policy.
    async fn find_matching_rules(
        &self,
        source_type: &AssetTypeId,
        source_metadata: &serde_json::Value,
        target_type: &AssetTypeId,
        target_metadata: &serde_json::Value,
    ) -> Result<Vec<DependencyRule>, crate::RepositoryError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dependency_rule_creation() {
        let source_id = AssetTypeId::new();
        let target_id = AssetTypeId::new();

        let rule = DependencyRule::new(source_id, target_id, RelationshipType::DependsOn, true);

        assert_eq!(rule.source_type_id, source_id);
        assert_eq!(rule.target_type_id, target_id);
        assert_eq!(rule.relationship, RelationshipType::DependsOn);
        assert!(rule.is_transitive);
    }

    #[test]
    fn test_dependency_rule_matches() {
        let source_id = AssetTypeId::new();
        let target_id = AssetTypeId::new();

        let rule = DependencyRule::new(source_id, target_id, RelationshipType::DependsOn, false);

        assert!(rule.matches(&source_id, &target_id));
        assert!(!rule.matches(&target_id, &source_id));
    }

    // --- RelationshipType tests ---

    #[test]
    fn relationship_type_round_trips_snake_case() {
        let all = [
            RelationshipType::DependsOn,
            RelationshipType::References,
            RelationshipType::Implements,
            RelationshipType::Fixes,
            RelationshipType::Verifies,
            RelationshipType::Executes,
            RelationshipType::Produces,
            RelationshipType::Blocks,
            RelationshipType::RelatesTo,
        ];
        for rt in all {
            let s = rt.as_str();
            assert_eq!(RelationshipType::from_str(s).unwrap(), rt);
        }
    }

    #[test]
    fn relationship_type_rejects_invalid_strings() {
        assert!(RelationshipType::from_str("Fixes").is_err());
        assert!(RelationshipType::from_str("DEPENDS_ON").is_err());
        assert!(RelationshipType::from_str("unknown").is_err());
    }

    #[test]
    fn relationship_type_display_matches_as_str() {
        assert_eq!(format!("{}", RelationshipType::Fixes), "fixes");
        assert_eq!(format!("{}", RelationshipType::DependsOn), "depends_on");
    }

    #[test]
    fn relationship_type_supports_work_item_semantics() {
        assert_ne!(RelationshipType::Implements, RelationshipType::DependsOn);
        assert_ne!(RelationshipType::Fixes, RelationshipType::References);
        assert_ne!(RelationshipType::Executes, RelationshipType::Produces);
    }

    // --- PropagationPolicy tests ---

    #[test]
    fn propagation_policy_round_trips_snake_case() {
        let all = [
            PropagationPolicy::Dirty,
            PropagationPolicy::ContextOnly,
            PropagationPolicy::AuditOnly,
        ];
        for pp in all {
            let s = pp.as_str();
            assert_eq!(PropagationPolicy::from_str(s).unwrap(), pp);
        }
    }

    #[test]
    fn propagation_policy_rejects_invalid_strings() {
        assert!(PropagationPolicy::from_str("Dirty").is_err());
        assert!(PropagationPolicy::from_str("CONTEXT_ONLY").is_err());
        assert!(PropagationPolicy::from_str("unknown").is_err());
    }

    #[test]
    fn propagation_policy_distinguishes_dirty_from_non_dirty() {
        assert!(PropagationPolicy::Dirty.triggers_dirty());
        assert!(!PropagationPolicy::ContextOnly.triggers_dirty());
        assert!(!PropagationPolicy::AuditOnly.triggers_dirty());
    }

    #[test]
    fn propagation_policy_default_is_dirty() {
        assert_eq!(PropagationPolicy::default(), PropagationPolicy::Dirty);
    }

    #[test]
    fn propagation_policy_display_matches_as_str() {
        assert_eq!(format!("{}", PropagationPolicy::Dirty), "dirty");
        assert_eq!(
            format!("{}", PropagationPolicy::ContextOnly),
            "context_only"
        );
        assert_eq!(format!("{}", PropagationPolicy::AuditOnly), "audit_only");
    }

    // --- default_propagation_policy tests ---

    #[test]
    fn relationship_default_propagation_dirty_group() {
        assert_eq!(
            RelationshipType::DependsOn.default_propagation_policy(),
            PropagationPolicy::Dirty
        );
        assert_eq!(
            RelationshipType::Implements.default_propagation_policy(),
            PropagationPolicy::Dirty
        );
        assert_eq!(
            RelationshipType::Fixes.default_propagation_policy(),
            PropagationPolicy::Dirty
        );
        assert_eq!(
            RelationshipType::Verifies.default_propagation_policy(),
            PropagationPolicy::Dirty
        );
    }

    #[test]
    fn relationship_default_propagation_context_only_group() {
        assert_eq!(
            RelationshipType::References.default_propagation_policy(),
            PropagationPolicy::ContextOnly
        );
        assert_eq!(
            RelationshipType::Executes.default_propagation_policy(),
            PropagationPolicy::ContextOnly
        );
    }

    #[test]
    fn relationship_default_propagation_audit_only_group() {
        assert_eq!(
            RelationshipType::Produces.default_propagation_policy(),
            PropagationPolicy::AuditOnly
        );
        assert_eq!(
            RelationshipType::Blocks.default_propagation_policy(),
            PropagationPolicy::AuditOnly
        );
        assert_eq!(
            RelationshipType::RelatesTo.default_propagation_policy(),
            PropagationPolicy::AuditOnly
        );
    }

    // --- Metadata filter and specificity tests ---

    #[test]
    fn dependency_rule_matches_source_metadata_filter() {
        let work_item_type = AssetTypeId::new();
        let requirement_type = AssetTypeId::new();

        let rule = DependencyRule::new(
            work_item_type,
            requirement_type,
            RelationshipType::Implements,
            true,
        )
        .with_source_metadata_filter(serde_json::json!({
            "work_item_kind": "feature"
        }))
        .with_propagation_policy(PropagationPolicy::Dirty);

        // Matching metadata
        assert!(rule.matches_with_metadata(
            &work_item_type,
            &serde_json::json!({"work_item_kind": "feature", "priority": "high"}),
            &requirement_type,
            &serde_json::json!({})
        ));

        // Non-matching metadata
        assert!(!rule.matches_with_metadata(
            &work_item_type,
            &serde_json::json!({"work_item_kind": "bugfix"}),
            &requirement_type,
            &serde_json::json!({})
        ));
    }

    #[test]
    fn dependency_rule_matches_target_metadata_filter() {
        let work_item_type = AssetTypeId::new();
        let test_report_type = AssetTypeId::new();

        let rule = DependencyRule::new(
            work_item_type,
            test_report_type,
            RelationshipType::References,
            false,
        )
        .with_target_metadata_filter(serde_json::json!({
            "status": "failed"
        }))
        .with_propagation_policy(PropagationPolicy::ContextOnly);

        assert!(rule.matches_with_metadata(
            &work_item_type,
            &serde_json::json!({"work_item_kind": "bugfix"}),
            &test_report_type,
            &serde_json::json!({"status": "failed"})
        ));

        assert!(!rule.matches_with_metadata(
            &work_item_type,
            &serde_json::json!({"work_item_kind": "bugfix"}),
            &test_report_type,
            &serde_json::json!({"status": "passed"})
        ));
    }

    #[test]
    fn dependency_rule_no_filter_matches_any_metadata() {
        let source_type = AssetTypeId::new();
        let target_type = AssetTypeId::new();

        let rule = DependencyRule::new(source_type, target_type, RelationshipType::DependsOn, true);

        assert!(rule.matches_with_metadata(
            &source_type,
            &serde_json::json!({"anything": "goes"}),
            &target_type,
            &serde_json::json!({})
        ));
    }

    #[test]
    fn dependency_rule_specificity_counts_metadata_keys() {
        let work_item_type = AssetTypeId::new();
        let requirement_type = AssetTypeId::new();

        let base = DependencyRule::new(
            work_item_type,
            requirement_type,
            RelationshipType::DependsOn,
            true,
        );
        let specific = base.clone().with_source_metadata_filter(serde_json::json!({
            "work_item_kind": "feature"
        }));
        let more_specific = base
            .clone()
            .with_source_metadata_filter(serde_json::json!({
                "work_item_kind": "feature"
            }))
            .with_target_metadata_filter(serde_json::json!({
                "priority": "high"
            }));

        assert!(specific.specificity() > base.specificity());
        assert!(more_specific.specificity() > specific.specificity());
        assert_eq!(base.specificity(), 0);
        assert_eq!(specific.specificity(), 1);
        assert_eq!(more_specific.specificity(), 2);
    }

    #[test]
    fn dependency_rule_new_defaults_propagation_from_relationship() {
        let source_type = AssetTypeId::new();
        let target_type = AssetTypeId::new();

        let rule_depends =
            DependencyRule::new(source_type, target_type, RelationshipType::DependsOn, true);
        assert_eq!(rule_depends.propagation_policy, PropagationPolicy::Dirty);

        let rule_refs =
            DependencyRule::new(source_type, target_type, RelationshipType::References, true);
        assert_eq!(rule_refs.propagation_policy, PropagationPolicy::ContextOnly);
    }

    #[test]
    fn dependency_rule_with_propagation_policy_overrides_default() {
        let source_type = AssetTypeId::new();
        let target_type = AssetTypeId::new();

        let rule = DependencyRule::new(source_type, target_type, RelationshipType::DependsOn, true)
            .with_propagation_policy(PropagationPolicy::AuditOnly);

        assert_eq!(rule.propagation_policy, PropagationPolicy::AuditOnly);
    }

    #[test]
    fn dependency_rule_type_mismatch_always_fails_metadata_match() {
        let source_type = AssetTypeId::new();
        let target_type = AssetTypeId::new();
        let wrong_type = AssetTypeId::new();

        let rule =
            DependencyRule::new(source_type, target_type, RelationshipType::Implements, true)
                .with_source_metadata_filter(serde_json::json!({
                    "work_item_kind": "feature"
                }));

        assert!(!rule.matches_with_metadata(
            &wrong_type,
            &serde_json::json!({"work_item_kind": "feature"}),
            &target_type,
            &serde_json::json!({})
        ));
    }

    // --- InMemoryDependencyRuleRepository tests ---

    use crate::repository::in_memory::InMemoryDependencyRuleRepository;

    #[tokio::test]
    async fn rule_repo_type_level_rule_allows_dependency() {
        let work_item_type = AssetTypeId::new();
        let requirement_type = AssetTypeId::new();

        let rule = DependencyRule::new(
            work_item_type,
            requirement_type,
            RelationshipType::DependsOn,
            true,
        );

        let repo = InMemoryDependencyRuleRepository::with_rules(vec![rule]);
        assert!(
            repo.is_dependency_allowed(&work_item_type, &requirement_type)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn rule_repo_no_type_rule_rejects_dependency() {
        let type_a = AssetTypeId::new();
        let type_b = AssetTypeId::new();

        let repo = InMemoryDependencyRuleRepository::new();
        assert!(!repo.is_dependency_allowed(&type_a, &type_b).await.unwrap());
    }

    #[tokio::test]
    async fn rule_repo_find_allowed_type_rules_returns_type_match() {
        let work_item_type = AssetTypeId::new();
        let requirement_type = AssetTypeId::new();
        let design_type = AssetTypeId::new();

        let rule1 = DependencyRule::new(
            work_item_type,
            requirement_type,
            RelationshipType::Implements,
            true,
        )
        .with_source_metadata_filter(serde_json::json!({"work_item_kind": "feature"}))
        .with_propagation_policy(PropagationPolicy::Dirty);

        let rule2 = DependencyRule::new(
            work_item_type,
            requirement_type,
            RelationshipType::DependsOn,
            true,
        );

        let rule3 = DependencyRule::new(
            work_item_type,
            design_type,
            RelationshipType::References,
            false,
        );

        let repo = InMemoryDependencyRuleRepository::with_rules(vec![rule1, rule2, rule3]);
        let allowed = repo
            .find_allowed_type_rules(&work_item_type, &requirement_type)
            .await
            .unwrap();

        assert_eq!(allowed.len(), 2);
    }

    #[tokio::test]
    async fn rule_repo_metadata_rule_matches_when_filter_satisfied() {
        let work_item_type = AssetTypeId::new();
        let requirement_type = AssetTypeId::new();

        let rule = DependencyRule::new(
            work_item_type,
            requirement_type,
            RelationshipType::Implements,
            true,
        )
        .with_source_metadata_filter(serde_json::json!({"work_item_kind": "feature"}))
        .with_propagation_policy(PropagationPolicy::Dirty);

        let repo = InMemoryDependencyRuleRepository::with_rules(vec![rule]);

        let matching = repo
            .find_matching_rules(
                &work_item_type,
                &serde_json::json!({"work_item_kind": "feature"}),
                &requirement_type,
                &serde_json::json!({}),
            )
            .await
            .unwrap();

        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].relationship, RelationshipType::Implements);
        assert_eq!(matching[0].propagation_policy, PropagationPolicy::Dirty);
    }

    #[tokio::test]
    async fn rule_repo_metadata_rule_does_not_match_when_filter_unsatisfied() {
        let work_item_type = AssetTypeId::new();
        let requirement_type = AssetTypeId::new();

        let rule = DependencyRule::new(
            work_item_type,
            requirement_type,
            RelationshipType::Implements,
            true,
        )
        .with_source_metadata_filter(serde_json::json!({"work_item_kind": "feature"}));

        let repo = InMemoryDependencyRuleRepository::with_rules(vec![rule]);

        let matching = repo
            .find_matching_rules(
                &work_item_type,
                &serde_json::json!({"work_item_kind": "bugfix"}),
                &requirement_type,
                &serde_json::json!({}),
            )
            .await
            .unwrap();

        assert!(matching.is_empty());
    }

    #[tokio::test]
    async fn rule_repo_type_level_rule_allows_even_when_metadata_rule_does_not_match() {
        let work_item_type = AssetTypeId::new();
        let requirement_type = AssetTypeId::new();

        let type_rule = DependencyRule::new(
            work_item_type,
            requirement_type,
            RelationshipType::DependsOn,
            true,
        );
        let specific_rule = DependencyRule::new(
            work_item_type,
            requirement_type,
            RelationshipType::Implements,
            true,
        )
        .with_source_metadata_filter(serde_json::json!({"work_item_kind": "feature"}));

        let repo = InMemoryDependencyRuleRepository::with_rules(vec![type_rule, specific_rule]);

        // bugfix metadata does not match the specific rule, but type-level still allows
        assert!(
            repo.is_dependency_allowed(&work_item_type, &requirement_type)
                .await
                .unwrap()
        );

        let allowed = repo
            .find_allowed_type_rules(&work_item_type, &requirement_type)
            .await
            .unwrap();
        assert_eq!(allowed.len(), 2);

        // Only type_rule matches metadata (no filter), specific_rule does not match
        let matching = repo
            .find_matching_rules(
                &work_item_type,
                &serde_json::json!({"work_item_kind": "bugfix"}),
                &requirement_type,
                &serde_json::json!({}),
            )
            .await
            .unwrap();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].relationship, RelationshipType::DependsOn);
    }
}
