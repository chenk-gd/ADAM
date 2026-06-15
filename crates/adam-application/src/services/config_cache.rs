//! Configuration cache for layered config resolution
//!
//! Provides caching for dependency type rules and organization policies
//! to avoid repeated database lookups during dependency resolution.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use adam_domain::{AssetTypeId, OrganizationId, SemVer, UpgradePolicy, VersionConstraint};

/// Error types for config cache operations
#[derive(Debug, thiserror::Error)]
pub enum ConfigCacheError {
    /// Repository error
    #[error("Repository error: {0}")]
    Repository(String),
}

/// Constraint template for default constraint generation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ConstraintTemplate {
    /// Follow major version (^1.0.0)
    #[default]
    FollowMajor,
    /// Exact current version (=1.0.0)
    ExactCurrent,
    /// Follow minor version (~1.0.0)
    FollowMinor,
    /// Any version (*)
    Wildcard,
}

impl ConstraintTemplate {
    /// Apply template to a version to generate a constraint
    pub fn apply(&self, version: &SemVer) -> VersionConstraint {
        match self {
            ConstraintTemplate::FollowMajor => {
                VersionConstraint::Caret(SemVer::new(version.major, 0, 0))
            }
            ConstraintTemplate::ExactCurrent => VersionConstraint::Exact(version.clone()),
            ConstraintTemplate::FollowMinor => {
                VersionConstraint::Tilde(SemVer::new(version.major, version.minor, 0))
            }
            ConstraintTemplate::Wildcard => VersionConstraint::Wildcard,
        }
    }
}

/// Dependency type rule with default constraint and policy
#[derive(Debug, Clone)]
pub struct DependencyTypeRule {
    pub id: uuid::Uuid,
    pub organization_id: OrganizationId,
    pub downstream_type_id: AssetTypeId,
    pub upstream_type_id: AssetTypeId,
    pub default_template: ConstraintTemplate,
    pub default_policy: UpgradePolicy,
    pub is_active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl DependencyTypeRule {
    /// Create a new dependency type rule
    pub fn new(
        organization_id: OrganizationId,
        downstream_type_id: AssetTypeId,
        upstream_type_id: AssetTypeId,
        default_template: ConstraintTemplate,
        default_policy: UpgradePolicy,
    ) -> Self {
        let now = chrono::Utc::now();
        Self {
            id: uuid::Uuid::new_v4(),
            organization_id,
            downstream_type_id,
            upstream_type_id,
            default_template,
            default_policy,
            is_active: true,
            created_at: now,
            updated_at: now,
        }
    }
}

/// Organization-level policy configuration
#[derive(Debug, Clone)]
pub struct OrganizationPolicy {
    pub organization_id: OrganizationId,
    pub default_template: ConstraintTemplate,
    pub default_policy: UpgradePolicy,
    pub require_approval_for_major: bool,
    pub unpublish_policy: UnpublishPolicy,
    pub unpublish_propagation: UnpublishPropagation,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Policy for unpublish operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnpublishPolicy {
    /// Unpublish is never allowed
    Never,
    /// Unpublish is allowed within a duration after release
    AllowWithin(chrono::Duration),
    /// Unpublish requires approval
    RequireApproval,
}

/// Propagation strategy for unpublish operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnpublishPropagation {
    /// Do not propagate
    None,
    /// Notify downstream dependencies
    NotifyDownstream,
    /// Automatically unpublish downstream
    AutoUnpublishDownstream,
}

impl Default for OrganizationPolicy {
    fn default() -> Self {
        Self {
            organization_id: OrganizationId::new(),
            default_template: ConstraintTemplate::FollowMajor,
            default_policy: UpgradePolicy::Notify,
            require_approval_for_major: true,
            unpublish_policy: UnpublishPolicy::AllowWithin(chrono::Duration::hours(24)),
            unpublish_propagation: UnpublishPropagation::NotifyDownstream,
            updated_at: chrono::Utc::now(),
        }
    }
}

/// Repository trait for dependency type rules
#[async_trait::async_trait]
pub trait DependencyTypeRuleRepository: Send + Sync {
    /// Find all type rules for an organization
    async fn find_by_organization(
        &self,
        org_id: OrganizationId,
    ) -> Result<Vec<DependencyTypeRule>, ConfigCacheError>;

    /// Find specific type rule
    async fn find_by_types(
        &self,
        org_id: OrganizationId,
        downstream_type: AssetTypeId,
        upstream_type: AssetTypeId,
    ) -> Result<Option<DependencyTypeRule>, ConfigCacheError>;
}

/// Repository trait for organization policies
#[async_trait::async_trait]
pub trait OrganizationPolicyRepository: Send + Sync {
    /// Find policy for organization
    async fn find_by_organization(
        &self,
        org_id: OrganizationId,
    ) -> Result<Option<OrganizationPolicy>, ConfigCacheError>;
}

/// Cache entry with timestamp for TTL tracking
#[derive(Debug, Clone)]
struct CacheEntry<T> {
    value: T,
    inserted_at: chrono::DateTime<chrono::Utc>,
}

impl<T: Clone> CacheEntry<T> {
    fn new(value: T) -> Self {
        Self {
            value,
            inserted_at: chrono::Utc::now(),
        }
    }

    fn is_expired(&self, ttl: chrono::Duration) -> bool {
        chrono::Utc::now() - self.inserted_at > ttl
    }
}

/// Configuration cache for layered config resolution
type TypeRuleCache =
    Arc<RwLock<HashMap<(AssetTypeId, AssetTypeId), CacheEntry<DependencyTypeRule>>>>;
type OrganizationPolicyCache = Arc<RwLock<HashMap<OrganizationId, CacheEntry<OrganizationPolicy>>>>;

pub struct ConfigCache {
    type_rules: TypeRuleCache,
    org_policies: OrganizationPolicyCache,
    ttl: chrono::Duration,
    type_rule_repo: Arc<dyn DependencyTypeRuleRepository>,
    org_policy_repo: Arc<dyn OrganizationPolicyRepository>,
}

impl ConfigCache {
    /// Create a new ConfigCache
    pub fn new(
        type_rule_repo: Arc<dyn DependencyTypeRuleRepository>,
        org_policy_repo: Arc<dyn OrganizationPolicyRepository>,
    ) -> Self {
        Self {
            type_rules: Arc::new(RwLock::new(HashMap::new())),
            org_policies: Arc::new(RwLock::new(HashMap::new())),
            ttl: chrono::Duration::minutes(5),
            type_rule_repo,
            org_policy_repo,
        }
    }

    /// Create a new ConfigCache with custom TTL
    pub fn with_ttl(
        type_rule_repo: Arc<dyn DependencyTypeRuleRepository>,
        org_policy_repo: Arc<dyn OrganizationPolicyRepository>,
        ttl: chrono::Duration,
    ) -> Self {
        Self {
            type_rules: Arc::new(RwLock::new(HashMap::new())),
            org_policies: Arc::new(RwLock::new(HashMap::new())),
            ttl,
            type_rule_repo,
            org_policy_repo,
        }
    }

    /// Preload all configuration for an organization
    pub async fn preload(&self, org_id: OrganizationId) -> Result<(), ConfigCacheError> {
        // Load type rules
        let rules = self.type_rule_repo.find_by_organization(org_id).await?;

        let mut rules_map = self.type_rules.write().await;
        for rule in rules {
            rules_map.insert(
                (rule.downstream_type_id, rule.upstream_type_id),
                CacheEntry::new(rule),
            );
        }

        // Load org policy
        if let Some(policy) = self.org_policy_repo.find_by_organization(org_id).await? {
            let mut policy_map = self.org_policies.write().await;
            policy_map.insert(org_id, CacheEntry::new(policy));
        }

        Ok(())
    }

    /// Get type rule for a dependency pair (from cache or repository)
    pub async fn get_type_rule(
        &self,
        downstream: AssetTypeId,
        upstream: AssetTypeId,
    ) -> Option<DependencyTypeRule> {
        let key = (downstream, upstream);

        // Check cache first
        {
            let cache = self.type_rules.read().await;
            if let Some(entry) = cache.get(&key) {
                if !entry.is_expired(self.ttl) {
                    return Some(entry.value.clone());
                }
            }
        }

        // Cache miss or expired - load from repository
        // For now, return None if not in cache (repository lookup would require org_id)
        // In a full implementation, we'd fetch from repo here
        None
    }

    /// Get type rule by organization and types
    pub async fn get_type_rule_for_org(
        &self,
        org_id: OrganizationId,
        downstream: AssetTypeId,
        upstream: AssetTypeId,
    ) -> Result<Option<DependencyTypeRule>, ConfigCacheError> {
        let key = (downstream, upstream);

        // Check cache first
        {
            let cache = self.type_rules.read().await;
            if let Some(entry) = cache.get(&key) {
                if !entry.is_expired(self.ttl) {
                    return Ok(Some(entry.value.clone()));
                }
            }
        }

        // Cache miss - load from repository
        let rule = self
            .type_rule_repo
            .find_by_types(org_id, downstream, upstream)
            .await?;

        if let Some(ref r) = rule {
            let mut cache = self.type_rules.write().await;
            cache.insert(key, CacheEntry::new(r.clone()));
        }

        Ok(rule)
    }

    /// Get organization policy (from cache or repository)
    pub async fn get_org_policy(
        &self,
        org_id: OrganizationId,
    ) -> Result<OrganizationPolicy, ConfigCacheError> {
        // Check cache first
        {
            let cache = self.org_policies.read().await;
            if let Some(entry) = cache.get(&org_id) {
                if !entry.is_expired(self.ttl) {
                    return Ok(entry.value.clone());
                }
            }
        }

        // Cache miss - load from repository
        let policy = self
            .org_policy_repo
            .find_by_organization(org_id)
            .await?
            .unwrap_or_default();

        // Update cache
        let mut cache = self.org_policies.write().await;
        cache.insert(org_id, CacheEntry::new(policy.clone()));

        Ok(policy)
    }

    /// Insert a type rule into the cache directly
    pub async fn insert_type_rule(&self, rule: DependencyTypeRule) {
        let mut cache = self.type_rules.write().await;
        cache.insert(
            (rule.downstream_type_id, rule.upstream_type_id),
            CacheEntry::new(rule),
        );
    }

    /// Insert an organization policy into the cache directly
    pub async fn insert_org_policy(&self, policy: OrganizationPolicy) {
        let mut cache = self.org_policies.write().await;
        cache.insert(policy.organization_id, CacheEntry::new(policy));
    }

    /// Clear all cached entries
    pub async fn clear(&self) {
        let mut rules = self.type_rules.write().await;
        rules.clear();
        let mut policies = self.org_policies.write().await;
        policies.clear();
    }

    /// Get cache statistics
    pub async fn stats(&self) -> CacheStats {
        let rules = self.type_rules.read().await;
        let policies = self.org_policies.read().await;
        CacheStats {
            type_rule_count: rules.len(),
            org_policy_count: policies.len(),
        }
    }
}

/// Cache statistics
#[derive(Debug, Clone, Copy)]
pub struct CacheStats {
    pub type_rule_count: usize,
    pub org_policy_count: usize,
}

/// In-memory implementation of DependencyTypeRuleRepository for testing
pub struct InMemoryDependencyTypeRuleRepository {
    rules: RwLock<Vec<DependencyTypeRule>>,
}

impl InMemoryDependencyTypeRuleRepository {
    pub fn new() -> Self {
        Self {
            rules: RwLock::new(Vec::new()),
        }
    }

    pub async fn save(&self, rule: DependencyTypeRule) {
        let mut rules = self.rules.write().await;
        rules.push(rule);
    }
}

impl Default for InMemoryDependencyTypeRuleRepository {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl DependencyTypeRuleRepository for InMemoryDependencyTypeRuleRepository {
    async fn find_by_organization(
        &self,
        org_id: OrganizationId,
    ) -> Result<Vec<DependencyTypeRule>, ConfigCacheError> {
        let rules = self.rules.read().await;
        Ok(rules
            .iter()
            .filter(|r| r.organization_id == org_id && r.is_active)
            .cloned()
            .collect())
    }

    async fn find_by_types(
        &self,
        org_id: OrganizationId,
        downstream_type: AssetTypeId,
        upstream_type: AssetTypeId,
    ) -> Result<Option<DependencyTypeRule>, ConfigCacheError> {
        let rules = self.rules.read().await;
        Ok(rules
            .iter()
            .find(|r| {
                r.organization_id == org_id
                    && r.downstream_type_id == downstream_type
                    && r.upstream_type_id == upstream_type
                    && r.is_active
            })
            .cloned())
    }
}

/// In-memory implementation of OrganizationPolicyRepository for testing
pub struct InMemoryOrganizationPolicyRepository {
    policies: RwLock<HashMap<OrganizationId, OrganizationPolicy>>,
}

impl InMemoryOrganizationPolicyRepository {
    pub fn new() -> Self {
        Self {
            policies: RwLock::new(HashMap::new()),
        }
    }

    pub async fn save(&self, policy: OrganizationPolicy) {
        let mut policies = self.policies.write().await;
        policies.insert(policy.organization_id, policy);
    }
}

impl Default for InMemoryOrganizationPolicyRepository {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl OrganizationPolicyRepository for InMemoryOrganizationPolicyRepository {
    async fn find_by_organization(
        &self,
        org_id: OrganizationId,
    ) -> Result<Option<OrganizationPolicy>, ConfigCacheError> {
        let policies = self.policies.read().await;
        Ok(policies.get(&org_id).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_constraint_template_apply() {
        let version = SemVer::new(1, 2, 3);

        let caret = ConstraintTemplate::FollowMajor.apply(&version);
        assert!(matches!(caret, VersionConstraint::Caret(_)));

        let exact = ConstraintTemplate::ExactCurrent.apply(&version);
        assert_eq!(exact, VersionConstraint::Exact(version.clone()));

        let tilde = ConstraintTemplate::FollowMinor.apply(&version);
        assert!(matches!(tilde, VersionConstraint::Tilde(_)));

        let wildcard = ConstraintTemplate::Wildcard.apply(&version);
        assert_eq!(wildcard, VersionConstraint::Wildcard);
    }

    #[tokio::test]
    async fn test_config_cache_preload() {
        let type_repo = Arc::new(InMemoryDependencyTypeRuleRepository::new());
        let policy_repo = Arc::new(InMemoryOrganizationPolicyRepository::new());

        let cache = ConfigCache::new(type_repo.clone(), policy_repo.clone());

        let org_id = OrganizationId::new();
        let downstream_type = AssetTypeId::new();
        let upstream_type = AssetTypeId::new();

        // Create and save a rule
        let rule = DependencyTypeRule::new(
            org_id,
            downstream_type,
            upstream_type,
            ConstraintTemplate::FollowMajor,
            UpgradePolicy::AutoPatch,
        );
        type_repo.save(rule.clone()).await;

        // Create and save a policy
        let policy = OrganizationPolicy {
            organization_id: org_id,
            default_template: ConstraintTemplate::FollowMinor,
            default_policy: UpgradePolicy::Notify,
            require_approval_for_major: false,
            unpublish_policy: UnpublishPolicy::Never,
            unpublish_propagation: UnpublishPropagation::None,
            updated_at: chrono::Utc::now(),
        };
        policy_repo.save(policy.clone()).await;

        // Preload cache
        cache.preload(org_id).await.unwrap();

        // Verify stats
        let stats = cache.stats().await;
        assert_eq!(stats.type_rule_count, 1);
        assert_eq!(stats.org_policy_count, 1);

        // Verify cache hits
        let cached_rule = cache.get_type_rule(downstream_type, upstream_type).await;
        assert!(cached_rule.is_some());
        assert_eq!(
            cached_rule.unwrap().default_policy,
            UpgradePolicy::AutoPatch
        );

        let cached_policy = cache.get_org_policy(org_id).await.unwrap();
        assert_eq!(cached_policy.default_policy, UpgradePolicy::Notify);
    }

    #[tokio::test]
    async fn test_config_cache_miss_then_load() {
        let type_repo = Arc::new(InMemoryDependencyTypeRuleRepository::new());
        let policy_repo = Arc::new(InMemoryOrganizationPolicyRepository::new());

        let cache = ConfigCache::new(type_repo.clone(), policy_repo.clone());

        let org_id = OrganizationId::new();
        let downstream_type = AssetTypeId::new();
        let upstream_type = AssetTypeId::new();

        // Create and save a rule
        let rule = DependencyTypeRule::new(
            org_id,
            downstream_type,
            upstream_type,
            ConstraintTemplate::ExactCurrent,
            UpgradePolicy::Manual,
        );
        type_repo.save(rule.clone()).await;

        // Load via repository (cache miss)
        let loaded_rule = cache
            .get_type_rule_for_org(org_id, downstream_type, upstream_type)
            .await
            .unwrap();

        assert!(loaded_rule.is_some());
        let loaded = loaded_rule.unwrap();
        assert_eq!(loaded.default_template, ConstraintTemplate::ExactCurrent);
        assert_eq!(loaded.default_policy, UpgradePolicy::Manual);

        // Now should be in cache
        let cached = cache.get_type_rule(downstream_type, upstream_type).await;
        assert!(cached.is_some());
    }

    #[tokio::test]
    async fn test_config_cache_clear() {
        let type_repo = Arc::new(InMemoryDependencyTypeRuleRepository::new());
        let policy_repo = Arc::new(InMemoryOrganizationPolicyRepository::new());

        let cache = ConfigCache::new(type_repo, policy_repo);

        let org_id = OrganizationId::new();
        let downstream_type = AssetTypeId::new();
        let upstream_type = AssetTypeId::new();

        // Insert directly into cache
        let rule = DependencyTypeRule::new(
            org_id,
            downstream_type,
            upstream_type,
            ConstraintTemplate::FollowMajor,
            UpgradePolicy::AutoMinor,
        );
        cache.insert_type_rule(rule).await;

        // Verify
        let stats = cache.stats().await;
        assert_eq!(stats.type_rule_count, 1);

        // Clear
        cache.clear().await;

        // Verify cleared
        let stats = cache.stats().await;
        assert_eq!(stats.type_rule_count, 0);
    }

    #[tokio::test]
    async fn test_organization_policy_default() {
        let policy = OrganizationPolicy::default();
        assert_eq!(policy.default_template, ConstraintTemplate::FollowMajor);
        assert_eq!(policy.default_policy, UpgradePolicy::Notify);
        assert!(policy.require_approval_for_major);
    }

    #[tokio::test]
    async fn test_cache_entry_expiration() {
        let entry = CacheEntry::new("test");

        // Should not be expired immediately
        assert!(!entry.is_expired(chrono::Duration::seconds(1)));

        // Should be expired with 0 duration
        assert!(entry.is_expired(chrono::Duration::seconds(0)));
    }
}
