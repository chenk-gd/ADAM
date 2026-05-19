//! Pre-compiled dependency for performance
//!
//! Provides fast constraint matching by pre-compiling version constraints
//! using the semver crate's VersionReq.

use super::rule::DependencyRuleId;
use crate::repository::AssetDependencyRecord;
use crate::version::{SemVer, VersionConstraint};
use crate::asset::instance::{AssetId};
use crate::repository::UpgradePolicy;

/// Pre-compiled dependency with version constraint for fast matching
#[derive(Debug, Clone)]
pub struct CompiledDependency {
    /// Unique identifier for the dependency
    pub id: DependencyRuleId,
    /// Downstream asset ID (dependent)
    pub downstream_id: AssetId,
    /// Upstream asset ID (dependency)
    pub upstream_id: AssetId,
    /// Original constraint string (e.g., "^1.0.0")
    pub constraint_str: String,
    /// Pre-compiled semver constraint
    pub compiled_constraint: semver::VersionReq,
    /// Currently effective (locked) version
    pub effective_version: SemVer,
    /// Upgrade policy for this dependency
    pub upgrade_policy: UpgradePolicy,
    /// Version stamp for staleness detection
    pub constraint_version: i64,
}

/// Error types for compilation failures
#[derive(Debug, thiserror::Error)]
pub enum CompilationError {
    /// Invalid constraint format
    #[error("invalid constraint format: {0}")]
    InvalidConstraint(String),
    /// Semver parsing error
    #[error("semver error: {0}")]
    Semver(String),
}

impl CompiledDependency {
    /// Compile from an AssetDependencyRecord
    ///
    /// # Arguments
    /// * `record` - The dependency record to compile
    ///
    /// # Returns
    /// * `Ok(CompiledDependency)` - Successfully compiled dependency
    /// * `Err(CompilationError)` - Failed to compile constraint
    pub fn compile(record: &AssetDependencyRecord) -> Result<Self, CompilationError> {
        // Convert our VersionConstraint to semver VersionReq format
        let semver_req = Self::to_semver_req(&record.declared_constraint)?;

        let compiled = semver::VersionReq::parse(&semver_req)
            .map_err(|e| CompilationError::Semver(e.to_string()))?;

        Ok(Self {
            id: DependencyRuleId(record.id),
            downstream_id: record.source_id,
            upstream_id: record.target_id,
            constraint_str: record.constraint_str.clone(),
            compiled_constraint: compiled,
            effective_version: record.effective_version.clone(),
            upgrade_policy: record.upgrade_policy,
            constraint_version: record.lock_version,
        })
    }

    /// Compile from parts (for testing and direct construction)
    pub fn from_parts(
        id: DependencyRuleId,
        downstream_id: AssetId,
        upstream_id: AssetId,
        constraint: &VersionConstraint,
        effective_version: SemVer,
        upgrade_policy: UpgradePolicy,
        lock_version: i64,
    ) -> Result<Self, CompilationError> {
        let constraint_str = constraint.to_string();
        let semver_req = Self::to_semver_req(constraint)?;

        let compiled = semver::VersionReq::parse(&semver_req)
            .map_err(|e| CompilationError::Semver(e.to_string()))?;

        Ok(Self {
            id,
            downstream_id,
            upstream_id,
            constraint_str,
            compiled_constraint: compiled,
            effective_version,
            upgrade_policy,
            constraint_version: lock_version,
        })
    }

    /// Convert our VersionConstraint to semver VersionReq format
    fn to_semver_req(constraint: &VersionConstraint) -> Result<String, CompilationError> {
        match constraint {
            VersionConstraint::Exact(v) => Ok(format!("={}.{}.{}", v.major, v.minor, v.patch)),
            VersionConstraint::Caret(v) => Ok(format!("^{}.{}.{}", v.major, v.minor, v.patch)),
            VersionConstraint::Tilde(v) => Ok(format!("~{}.{}.{}", v.major, v.minor, v.patch)),
            VersionConstraint::Wildcard => Ok("*".to_string()),
            VersionConstraint::Range { min, max } => {
                let min_str = match min {
                    crate::version::Bound::Inclusive(v) => {
                        format!(">={}.{}.{}", v.major, v.minor, v.patch)
                    }
                    crate::version::Bound::Exclusive(v) => {
                        format!(">{}.{}.{}", v.major, v.minor, v.patch)
                    }
                };
                let max_str = match max {
                    crate::version::Bound::Inclusive(v) => {
                        format!(",<={}.{}.{}", v.major, v.minor, v.patch)
                    }
                    crate::version::Bound::Exclusive(v) => {
                        format!(",<{}.{}.{}", v.major, v.minor, v.patch)
                    }
                };
                Ok(format!("{}{}", min_str, max_str))
            }
        }
    }

    /// Check if compiled constraint is stale compared to database record
    ///
    /// Returns true if the constraint has been modified in the database
    pub fn is_stale(&self, db_constraint: &str, db_version: i64) -> bool {
        self.constraint_str != db_constraint || self.constraint_version != db_version
    }

    /// Fast match using compiled semver constraint
    ///
    /// # Arguments
    /// * `version` - The version to check
    ///
    /// # Returns
    /// * `true` - The version satisfies the constraint
    /// * `false` - The version does not satisfy the constraint
    pub fn matches(&self, version: &SemVer) -> bool {
        let semver_version = semver::Version::new(version.major, version.minor, version.patch);
        self.compiled_constraint.matches(&semver_version)
    }

    /// Check if this dependency should auto-update to the given version
    ///
    /// Based on upgrade_policy and constraint matching
    pub fn should_auto_update(&self, new_version: &SemVer) -> bool {
        if !self.matches(new_version) {
            return false;
        }

        match self.upgrade_policy {
            UpgradePolicy::AutoPatch => {
                // Auto-update if only patch changed
                new_version.major == self.effective_version.major
                    && new_version.minor == self.effective_version.minor
            }
            UpgradePolicy::AutoMinor => {
                // Auto-update if major is same
                new_version.major == self.effective_version.major
            }
            UpgradePolicy::Notify | UpgradePolicy::Manual | UpgradePolicy::Pin => false,
        }
    }

    /// Get the next auto-update version based on policy
    ///
    /// Returns the version that would be auto-updated to, if any
    pub fn next_auto_version(&self, available_versions: &[SemVer]) -> Option<SemVer> {
        let mut candidates: Vec<&SemVer> = available_versions
            .iter()
            .filter(|v| self.matches(v) && *v > &self.effective_version)
            .collect();

        if candidates.is_empty() {
            return None;
        }

        // Sort by version (highest first)
        candidates.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

        match self.upgrade_policy {
            UpgradePolicy::AutoPatch => {
                // Find highest patch for current minor
                candidates
                    .into_iter()
                    .find(|v| {
                        v.major == self.effective_version.major
                            && v.minor == self.effective_version.minor
                    })
                    .cloned()
            }
            UpgradePolicy::AutoMinor => {
                // Find highest version for current major
                candidates
                    .into_iter()
                    .find(|v| v.major == self.effective_version.major)
                    .cloned()
            }
            _ => None,
        }
    }
}

/// Cache for compiled dependencies to avoid recompilation
pub struct CompiledDependencyCache {
    cache: std::collections::HashMap<DependencyRuleId, CompiledDependency>,
}

impl CompiledDependencyCache {
    /// Create a new empty cache
    pub fn new() -> Self {
        Self {
            cache: std::collections::HashMap::new(),
        }
    }

    /// Get a cached dependency if not stale
    pub fn get(&self, record: &AssetDependencyRecord) -> Option<&CompiledDependency> {
        let id = DependencyRuleId(record.id);
        self.cache.get(&id).filter(|c| !c.is_stale(&record.constraint_str, record.lock_version))
    }

    /// Compile and insert a dependency into cache, returning the id
    fn compile_and_insert(
        &mut self,
        record: &AssetDependencyRecord,
    ) -> Result<DependencyRuleId, CompilationError> {
        let compiled = CompiledDependency::compile(record)?;
        let id = compiled.id;
        self.cache.insert(id, compiled);
        Ok(id)
    }

    /// Ensure a dependency is compiled and cached
    ///
    /// If the dependency is already cached and not stale, does nothing.
    /// Otherwise, recompiles and caches.
    pub fn ensure_compiled(
        &mut self,
        record: &AssetDependencyRecord,
    ) -> Result<(), CompilationError> {
        // Check if cached and not stale
        if self.get(record).is_some() {
            return Ok(());
        }

        // Compile and cache
        self.compile_and_insert(record)?;
        Ok(())
    }

    /// Insert a pre-compiled dependency
    pub fn insert(&mut self, compiled: CompiledDependency) {
        self.cache.insert(compiled.id, compiled);
    }

    /// Remove a dependency from cache
    pub fn remove(&mut self, id: &DependencyRuleId) -> Option<CompiledDependency> {
        self.cache.remove(id)
    }

    /// Clear all cached entries
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Get cache size
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Check if cache is empty
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

impl Default for CompiledDependencyCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::{Bound, VersionConstraint};

    fn create_test_dependency(
        constraint: VersionConstraint,
        effective_version: SemVer,
        policy: UpgradePolicy,
    ) -> CompiledDependency {
        CompiledDependency::from_parts(
            DependencyRuleId(uuid::Uuid::new_v4()),
            AssetId(uuid::Uuid::new_v4()),
            AssetId(uuid::Uuid::new_v4()),
            &constraint,
            effective_version,
            policy,
            1,
        )
        .unwrap()
    }

    #[test]
    fn test_compile_caret_constraint() {
        let constraint = VersionConstraint::Caret(SemVer::new(1, 0, 0));
        let dep = create_test_dependency(constraint, SemVer::new(1, 0, 0), UpgradePolicy::Notify);

        assert_eq!(dep.constraint_str, "^1.0.0");
        assert!(dep.matches(&SemVer::new(1, 0, 0)));
        assert!(dep.matches(&SemVer::new(1, 5, 0)));
        assert!(!dep.matches(&SemVer::new(2, 0, 0)));
    }

    #[test]
    fn test_compile_exact_constraint() {
        let constraint = VersionConstraint::Exact(SemVer::new(1, 2, 3));
        let dep = create_test_dependency(constraint, SemVer::new(1, 2, 3), UpgradePolicy::Pin);

        assert!(dep.matches(&SemVer::new(1, 2, 3)));
        assert!(!dep.matches(&SemVer::new(1, 2, 4)));
        assert!(!dep.matches(&SemVer::new(1, 3, 0)));
    }

    #[test]
    fn test_compile_tilde_constraint() {
        let constraint = VersionConstraint::Tilde(SemVer::new(1, 2, 0));
        let dep = create_test_dependency(constraint, SemVer::new(1, 2, 0), UpgradePolicy::Notify);

        assert!(dep.matches(&SemVer::new(1, 2, 0)));
        assert!(dep.matches(&SemVer::new(1, 2, 5)));
        assert!(!dep.matches(&SemVer::new(1, 3, 0)));
        assert!(!dep.matches(&SemVer::new(2, 0, 0)));
    }

    #[test]
    fn test_compile_wildcard_constraint() {
        let constraint = VersionConstraint::Wildcard;
        let dep = create_test_dependency(constraint, SemVer::new(1, 0, 0), UpgradePolicy::AutoMinor);

        assert!(dep.matches(&SemVer::new(1, 0, 0)));
        assert!(dep.matches(&SemVer::new(2, 5, 3)));
        assert!(dep.matches(&SemVer::new(99, 99, 99)));
    }

    #[test]
    fn test_compile_range_constraint() {
        let constraint = VersionConstraint::Range {
            min: Bound::Inclusive(SemVer::new(1, 0, 0)),
            max: Bound::Exclusive(SemVer::new(2, 0, 0)),
        };
        let dep = create_test_dependency(constraint, SemVer::new(1, 0, 0), UpgradePolicy::Notify);

        assert!(dep.matches(&SemVer::new(1, 0, 0)));
        assert!(dep.matches(&SemVer::new(1, 5, 0)));
        assert!(!dep.matches(&SemVer::new(2, 0, 0)));
    }

    #[test]
    fn test_staleness_check() {
        let constraint = VersionConstraint::Caret(SemVer::new(1, 0, 0));
        let dep = create_test_dependency(constraint, SemVer::new(1, 0, 0), UpgradePolicy::Notify);

        // Not stale
        assert!(!dep.is_stale("^1.0.0", 1));

        // Stale - constraint changed
        assert!(dep.is_stale("^2.0.0", 1));

        // Stale - version changed
        assert!(dep.is_stale("^1.0.0", 2));

        // Stale - both changed
        assert!(dep.is_stale("^2.0.0", 2));
    }

    #[test]
    fn test_should_auto_update_policy() {
        let constraint = VersionConstraint::Caret(SemVer::new(1, 0, 0));

        // AutoPatch policy
        let patch_dep = create_test_dependency(
            constraint.clone(),
            SemVer::new(1, 2, 3),
            UpgradePolicy::AutoPatch,
        );
        assert!(patch_dep.should_auto_update(&SemVer::new(1, 2, 5))); // patch bump
        assert!(!patch_dep.should_auto_update(&SemVer::new(1, 3, 0))); // minor bump
        assert!(!patch_dep.should_auto_update(&SemVer::new(2, 0, 0))); // major bump

        // AutoMinor policy
        let minor_dep = create_test_dependency(
            constraint.clone(),
            SemVer::new(1, 2, 3),
            UpgradePolicy::AutoMinor,
        );
        assert!(minor_dep.should_auto_update(&SemVer::new(1, 2, 5))); // patch bump
        assert!(minor_dep.should_auto_update(&SemVer::new(1, 3, 0))); // minor bump
        assert!(!minor_dep.should_auto_update(&SemVer::new(2, 0, 0))); // major bump

        // Manual policy - never auto updates
        let manual_dep =
            create_test_dependency(constraint, SemVer::new(1, 0, 0), UpgradePolicy::Manual);
        assert!(!manual_dep.should_auto_update(&SemVer::new(1, 0, 1)));
    }

    #[test]
    fn test_next_auto_version() {
        let constraint = VersionConstraint::Caret(SemVer::new(1, 0, 0));
        let available = vec![
            SemVer::new(1, 0, 1),
            SemVer::new(1, 0, 5),
            SemVer::new(1, 1, 0),
            SemVer::new(1, 2, 0),
            SemVer::new(2, 0, 0),
        ];

        // AutoPatch - should return 1.0.5 (highest patch)
        let patch_dep = create_test_dependency(
            constraint.clone(),
            SemVer::new(1, 0, 0),
            UpgradePolicy::AutoPatch,
        );
        assert_eq!(patch_dep.next_auto_version(&available), Some(SemVer::new(1, 0, 5)));

        // AutoMinor - should return 1.2.0 (highest for major=1)
        let minor_dep = create_test_dependency(
            constraint.clone(),
            SemVer::new(1, 0, 0),
            UpgradePolicy::AutoMinor,
        );
        assert_eq!(minor_dep.next_auto_version(&available), Some(SemVer::new(1, 2, 0)));

        // Manual - should return None
        let manual_dep =
            create_test_dependency(constraint, SemVer::new(1, 0, 0), UpgradePolicy::Manual);
        assert_eq!(manual_dep.next_auto_version(&available), None);
    }

    #[test]
    fn test_compiled_cache() {
        let mut cache = CompiledDependencyCache::new();
        assert!(cache.is_empty());

        // Create a mock record
        let record = AssetDependencyRecord {
            id: uuid::Uuid::new_v4(),
            source_id: AssetId(uuid::Uuid::new_v4()),
            target_id: AssetId(uuid::Uuid::new_v4()),
            relationship: "depends_on".to_string(),
            declared_constraint: VersionConstraint::Caret(SemVer::new(1, 0, 0)),
            constraint_str: "^1.0.0".to_string(),
            effective_version: SemVer::new(1, 0, 0),
            effective_updated_by: "test".to_string(),
            effective_updated_at: chrono::Utc::now(),
            effective_reason: crate::repository::EffectiveUpdateReason::ManualClean,
            upgrade_policy: UpgradePolicy::Notify,
            lock_version: 1,
            created_at: chrono::Utc::now(),
        };

        // Compile and cache
        cache.ensure_compiled(&record).unwrap();
        assert_eq!(cache.len(), 1);

        // Get the cached value
        let cached = cache.get(&record).unwrap();
        assert_eq!(cached.constraint_str, "^1.0.0");

        // Get again - should be cached, no change
        cache.ensure_compiled(&record).unwrap();
        assert_eq!(cache.len(), 1);

        // Clear
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn test_cache_stale_detection() {
        let mut cache = CompiledDependencyCache::new();

        // Create initial record
        let id = uuid::Uuid::new_v4();
        let record = AssetDependencyRecord {
            id,
            source_id: AssetId(uuid::Uuid::new_v4()),
            target_id: AssetId(uuid::Uuid::new_v4()),
            relationship: "depends_on".to_string(),
            declared_constraint: VersionConstraint::Caret(SemVer::new(1, 0, 0)),
            constraint_str: "^1.0.0".to_string(),
            effective_version: SemVer::new(1, 0, 0),
            effective_updated_by: "test".to_string(),
            effective_updated_at: chrono::Utc::now(),
            effective_reason: crate::repository::EffectiveUpdateReason::ManualClean,
            upgrade_policy: UpgradePolicy::Notify,
            lock_version: 1,
            created_at: chrono::Utc::now(),
        };

        // Compile and cache
        cache.ensure_compiled(&record).unwrap();

        // Create modified record (stale)
        let stale_record = AssetDependencyRecord {
            id,
            source_id: AssetId(uuid::Uuid::new_v4()),
            target_id: AssetId(uuid::Uuid::new_v4()),
            relationship: "depends_on".to_string(),
            declared_constraint: VersionConstraint::Caret(SemVer::new(2, 0, 0)),
            constraint_str: "^2.0.0".to_string(),
            effective_version: SemVer::new(2, 0, 0),
            effective_updated_by: "test".to_string(),
            effective_updated_at: chrono::Utc::now(),
            effective_reason: crate::repository::EffectiveUpdateReason::ManualClean,
            upgrade_policy: UpgradePolicy::Notify,
            lock_version: 2,
            created_at: chrono::Utc::now(),
        };

        // Should detect staleness and recompile
        cache.ensure_compiled(&stale_record).unwrap();
        let recompiled = cache.get(&stale_record).unwrap();
        assert_eq!(recompiled.constraint_str, "^2.0.0");
        assert_eq!(recompiled.constraint_version, 2);
    }
}
