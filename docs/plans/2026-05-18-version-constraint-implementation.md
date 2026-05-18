# ADAM Version Constraint Implementation Plan v3.0

> **MAJOR UPDATE**: Complete rewrite based on `2026-05-18-version-constraint-design.md`  
> **Key Changes**:
> - Removed Fork/VersionLine model entirely
> - Added VersionConstraint (^1.0.0, ~1.0.0)
> - Added Dirty aggregation (trigger_count)
> - Added CAS optimistic locking
> - Added idempotency design
> - Added unpublish/rollback support
> - Added performance optimizations

**Goal:** Implement version constraint model where assets have version history, dependencies use SemVer constraints (^1.0.0), and Dirty propagation is based on constraint matching.

**Architecture:** Each asset has multiple versions (1.0.0, 2.0.0). Dependencies declare constraints (^1.0.0) and effective_version. Dirty propagation uses constraint matching with aggregation.

**Tech Stack:** Rust 2024, PostgreSQL, sqlx, axum, async-trait, chrono, uuid, semver crate

**Timeline:** 8 weeks (increased from 5 due to additional features)

---

## Pre-Flight Checklist

Before starting:
- [ ] Review design document: `docs/plans/2026-05-18-version-constraint-design.md`
- [ ] Run `cargo test` to confirm baseline is green
- [ ] Create worktree: `git worktree add ../adam-version-constraint-work`
- [ ] Ensure semver crate is available: `cargo add semver`

---

## Phase 1: Core Domain - SemVer & Constraints

### Task 1: Implement SemVer type

**Files:**
- Create: `crates/adam-domain/src/version/semver.rs`
- Modify: `crates/adam-domain/src/version/mod.rs`
- Modify: `crates/adam-domain/src/lib.rs`

**Step 1: Create SemVer type**

```rust
//! Semantic versioning support

use serde::{Deserialize, Serialize};
use std::fmt;

/// Semantic version
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub struct SemVer {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub prerelease: Option<String>,
}

impl SemVer {
    pub fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
            prerelease: None,
        }
    }

    pub fn parse(version: &str) -> Result<Self, String> {
        let version = version.trim_start_matches('v');
        let parts: Vec<&str> = version.split('.').collect();
        if parts.len() != 3 {
            return Err("Invalid semver format".to_string());
        }
        Ok(Self::new(
            parts[0].parse::<u64>().map_err(|e| e.to_string())?,
            parts[1].parse::<u64>().map_err(|e| e.to_string())?,
            parts[2].parse::<u64>().map_err(|e| e.to_string())?,
        ))
    }

    pub fn is_compatible_with(&self, other: &SemVer) -> bool {
        self.major == other.major
    }

    pub fn next_major(&self) -> Self {
        Self::new(self.major + 1, 0, 0)
    }

    pub fn next_minor(&self) -> Self {
        Self::new(self.major, self.minor + 1, 0)
    }

    pub fn next_patch(&self) -> Self {
        Self::new(self.major, self.minor, self.patch + 1)
    }
}

impl fmt::Display for SemVer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_semver_parse_and_display() {
        let v = SemVer::parse("1.2.3").unwrap();
        assert_eq!(v.to_string(), "1.2.3");
    }

    #[test]
    fn test_semver_compatibility() {
        let v1 = SemVer::new(1, 0, 0);
        let v2 = SemVer::new(1, 5, 0);
        let v3 = SemVer::new(2, 0, 0);
        assert!(v1.is_compatible_with(&v2));
        assert!(!v1.is_compatible_with(&v3));
    }
}
```

**Step 2: Create module**

Create `crates/adam-domain/src/version/mod.rs`:

```rust
//! Version module
pub mod semver;
pub use semver::SemVer;
```

**Step 3: Export**

Modify `crates/adam-domain/src/lib.rs`:

```rust
pub mod version;
```

**Step 4: Test**

```bash
cargo test -p adam-domain version:: -- --nocapture
```

**Step 5: Commit**

```bash
git add crates/adam-domain/src/version/
git commit -m "feat: add SemVer type"
```

---

### Task 2: Implement VersionConstraint

**Files:**
- Create: `crates/adam-domain/src/version/constraint.rs`
- Modify: `crates/adam-domain/src/version/mod.rs`

**Step 1: Create constraint type**

```rust
//! Version constraint expressions

use serde::{Deserialize, Serialize};
use super::semver::SemVer;

/// Version constraint expression
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VersionConstraint {
    Exact(SemVer),           // =1.0.0
    Caret(SemVer),          // ^1.0.0 -> >=1.0.0, <2.0.0
    Tilde(SemVer),          // ~1.0.0 -> >=1.0.0, <1.1.0
    Range { min: Bound, max: Bound },
    Wildcard,               // *
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Bound {
    Inclusive(SemVer),
    Exclusive(SemVer),
}

impl VersionConstraint {
    /// Parse constraint from string
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        
        if s == "*" {
            return Ok(Self::Wildcard);
        }
        
        if let Some(version_str) = s.strip_prefix('^') {
            let version = SemVer::parse(version_str)?;
            return Ok(Self::Caret(version));
        }
        
        if let Some(version_str) = s.strip_prefix('~') {
            let version = SemVer::parse(version_str)?;
            return Ok(Self::Tilde(version));
        }
        
        if let Some(version_str) = s.strip_prefix('=') {
            let version = SemVer::parse(version_str)?;
            return Ok(Self::Exact(version));
        }
        
        // Try parsing as exact version
        let version = SemVer::parse(s)?;
        Ok(Self::Exact(version))
    }
    
    /// Check if version satisfies constraint
    pub fn matches(&self, version: &SemVer) -> bool {
        match self {
            Self::Exact(v) => version == v,
            Self::Caret(v) => {
                version >= v && version.major == v.major
            }
            Self::Tilde(v) => {
                version >= v && version.major == v.major && version.minor == v.minor
            }
            Self::Range { min, max } => {
                let min_satisfied = match min {
                    Bound::Inclusive(v) => version >= v,
                    Bound::Exclusive(v) => version > v,
                };
                let max_satisfied = match max {
                    Bound::Inclusive(v) => version <= v,
                    Bound::Exclusive(v) => version < v,
                };
                min_satisfied && max_satisfied
            }
            Self::Wildcard => true,
        }
    }
    
    pub fn to_string(&self) -> String {
        match self {
            Self::Exact(v) => format!("={}", v),
            Self::Caret(v) => format!("^{}", v),
            Self::Tilde(v) => format!("~{}", v),
            Self::Range { min, max } => {
                let min_str = match min {
                    Bound::Inclusive(v) => format!(">={}", v),
                    Bound::Exclusive(v) => format!(">{}", v),
                };
                let max_str = match max {
                    Bound::Inclusive(v) => format!(">={}", v),
                    Bound::Exclusive(v) => format!(">{}", v),
                };
                format!("{}, {}", min_str, max_str)
            }
            Self::Wildcard => "*".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_caret_constraint() {
        let c = VersionConstraint::parse("^1.0.0").unwrap();
        assert!(c.matches(&SemVer::new(1, 0, 0)));
        assert!(c.matches(&SemVer::new(1, 5, 0)));
        assert!(!c.matches(&SemVer::new(2, 0, 0)));
    }

    #[test]
    fn test_exact_constraint() {
        let c = VersionConstraint::parse("=1.0.0").unwrap();
        assert!(c.matches(&SemVer::new(1, 0, 0)));
        assert!(!c.matches(&SemVer::new(1, 0, 1)));
    }
}
```

**Step 2: Export**

Modify `crates/adam-domain/src/version/mod.rs`:

```rust
pub mod constraint;
pub use constraint::{VersionConstraint, Bound};
```

**Step 3: Test**

```bash
cargo test -p adam-domain constraint -- --nocapture
```

**Step 4: Commit**

```bash
git commit -m "feat: add VersionConstraint type"
```

---

## Phase 2: Domain Layer - Asset & Dependency Updates

### Task 3: Update AssetInstance with current_version

**Files:**
- Modify: `crates/adam-domain/src/asset/instance.rs`

**Step 1: Update struct**

```rust
use crate::version::SemVer;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetInstance {
    pub id: AssetId,
    pub name: String,
    pub asset_type_id: AssetTypeId,
    pub project_id: Option<ProjectId>,
    pub organization_id: OrganizationId,
    pub level: AssetLevel,
    pub current_version: SemVer,  // CHANGED: from Option<String>
    pub current_state: AssetState,
    pub external_ref: String,
    pub source: String,
    pub metadata: serde_json::Value,
    pub assignees: Vec<String>,
    pub publisher: Option<String>,
    pub lock_version: i64,  // NEW: for optimistic locking
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

**Step 2: Update constructors**

Update `new_project_level` and `new_organization_level` to accept `current_version: SemVer` and initialize `lock_version: 1`.

**Step 3: Update tests**

Update all tests to provide SemVer.

**Step 4: Commit**

```bash
git commit -m "feat: update AssetInstance with SemVer and lock_version"
```

---

### Task 4: Update AssetDependency with constraint

**Files:**
- Modify: `crates/adam-domain/src/dependency/rule.rs` or create new file

**Step 1: Update struct**

```rust
use crate::version::{SemVer, VersionConstraint};

/// Asset dependency with version constraint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetDependency {
    pub id: DependencyId,
    pub downstream_id: AssetId,
    pub upstream_id: AssetId,
    pub declared_constraint: VersionConstraint,  // ^1.0.0
    pub constraint_str: String,                  // "^1.0.0" for DB storage
    pub effective_version: SemVer,              // 1.0.0 (locked)
    pub upgrade_policy: UpgradePolicy,
    pub lock_version: i64,                      // optimistic locking
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpgradePolicy {
    AutoPatch,
    AutoMinor,
    Notify,
    Manual,
    Pin,
}

impl Default for UpgradePolicy {
    fn default() -> Self {
        UpgradePolicy::Notify
    }
}
```

**Step 2: Update constructor**

Add constructor that takes constraint and version.

**Step 3: Commit**

```bash
git commit -m "feat: update AssetDependency with version constraint"
```

---

## Phase 3: Infrastructure - Database Schema

### Task 5: Create database migration

**Files:**
- Create: `migrations/V3__version_constraints.sql`

**Step 1: Write migration**

```sql
-- Update asset_instances to use SemVer
ALTER TABLE asset_instances 
ADD COLUMN current_version_major INTEGER NOT NULL DEFAULT 0,
ADD COLUMN current_version_minor INTEGER NOT NULL DEFAULT 0,
ADD COLUMN current_version_patch INTEGER NOT NULL DEFAULT 0,
ADD COLUMN lock_version BIGINT NOT NULL DEFAULT 1;

-- Migrate existing data
UPDATE asset_instances SET
    current_version_major = CAST(SPLIT_PART(current_version, '.', 1) AS INTEGER),
    current_version_minor = CAST(SPLIT_PART(current_version, '.', 2) AS INTEGER),
    current_version_patch = CAST(SPLIT_PART(current_version, '.', 3) AS INTEGER)
WHERE current_version IS NOT NULL;

-- Drop old column
ALTER TABLE asset_instances DROP COLUMN current_version;

-- Update asset_dependencies with constraint
ALTER TABLE asset_dependencies
ADD COLUMN declared_constraint VARCHAR(255) NOT NULL DEFAULT '^1.0.0',
ADD COLUMN constraint_str VARCHAR(255) NOT NULL DEFAULT '^1.0.0',
ADD COLUMN effective_version_major INTEGER NOT NULL DEFAULT 0,
ADD COLUMN effective_version_minor INTEGER NOT NULL DEFAULT 0,
ADD COLUMN effective_version_patch INTEGER NOT NULL DEFAULT 0,
ADD COLUMN upgrade_policy VARCHAR(50) NOT NULL DEFAULT 'Notify',
ADD COLUMN lock_version BIGINT NOT NULL DEFAULT 1;

-- Migrate existing dependencies
UPDATE asset_dependencies SET
    declared_constraint = '^' || declared_version,
    constraint_str = '^' || declared_version,
    effective_version_major = CAST(SPLIT_PART(effective_version, '.', 1) AS INTEGER),
    effective_version_minor = CAST(SPLIT_PART(effective_version, '.', 2) AS INTEGER),
    effective_version_patch = CAST(SPLIT_PART(effective_version, '.', 3) AS INTEGER);

-- Drop old columns
ALTER TABLE asset_dependencies DROP COLUMN declared_version;
ALTER TABLE asset_dependencies DROP COLUMN effective_version;

-- Create asset_versions table
CREATE TABLE asset_versions (
    id UUID PRIMARY KEY,
    asset_id UUID NOT NULL REFERENCES asset_instances(id),
    major INTEGER NOT NULL,
    minor INTEGER NOT NULL,
    patch INTEGER NOT NULL,
    prerelease VARCHAR(50),
    content_ref TEXT NOT NULL,
    is_lts BOOLEAN DEFAULT FALSE,
    is_unpublished BOOLEAN DEFAULT FALSE,
    unpublished_at TIMESTAMP,
    unpublished_by VARCHAR(255),
    unpublished_reason TEXT,
    release_notes TEXT,
    released_by VARCHAR(255),
    released_at TIMESTAMP NOT NULL,
    UNIQUE(asset_id, major, minor, patch)
);

-- Update dirty_resolution_logs with aggregation
ALTER TABLE dirty_resolution_logs
ADD COLUMN latest_trigger_version_major INTEGER,
ADD COLUMN latest_trigger_version_minor INTEGER,
ADD COLUMN latest_trigger_version_patch INTEGER,
ADD COLUMN latest_triggered_at TIMESTAMP,
ADD COLUMN first_triggered_at TIMESTAMP,
ADD COLUMN trigger_count INTEGER DEFAULT 1,
ADD COLUMN aggregated BOOLEAN DEFAULT FALSE;

-- Migrate existing dirty logs
UPDATE dirty_resolution_logs SET
    first_triggered_at = triggered_at,
    latest_triggered_at = triggered_at,
    trigger_count = 1,
    aggregated = FALSE;

-- Create dependency_type_rules table
CREATE TABLE dependency_type_rules (
    id UUID PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id),
    downstream_type_id UUID NOT NULL REFERENCES asset_types(id),
    upstream_type_id UUID NOT NULL REFERENCES asset_types(id),
    default_template VARCHAR(50) NOT NULL DEFAULT 'FollowMajor',
    default_policy VARCHAR(50) NOT NULL DEFAULT 'Notify',
    is_active BOOLEAN DEFAULT TRUE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
    UNIQUE(organization_id, downstream_type_id, upstream_type_id)
);

-- Create organization_policies table
CREATE TABLE organization_policies (
    organization_id UUID PRIMARY KEY REFERENCES organizations(id),
    default_template VARCHAR(50) NOT NULL DEFAULT 'FollowMajor',
    default_policy VARCHAR(50) NOT NULL DEFAULT 'Notify',
    require_approval_for_major BOOLEAN DEFAULT TRUE,
    unpublish_policy VARCHAR(50) DEFAULT 'AllowWithin24h',
    unpublish_propagation VARCHAR(50) DEFAULT 'NotifyDownstream',
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Create unpublish_approvals table
CREATE TABLE unpublish_approvals (
    id UUID PRIMARY KEY,
    asset_id UUID NOT NULL REFERENCES asset_instances(id),
    version VARCHAR(50) NOT NULL,
    requested_by VARCHAR(255) NOT NULL,
    requested_at TIMESTAMP NOT NULL,
    reason TEXT NOT NULL,
    status VARCHAR(50) NOT NULL,
    approved_by VARCHAR(255),
    approved_at TIMESTAMP,
    UNIQUE(asset_id, version)
);

-- Create dependency_snapshots table
CREATE TABLE dependency_snapshots (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL,
    asset_id UUID NOT NULL REFERENCES asset_instances(id),
    dependency_id UUID NOT NULL REFERENCES asset_dependencies(id),
    upstream_id UUID NOT NULL,
    declared_constraint VARCHAR(255) NOT NULL,
    effective_version_major INTEGER NOT NULL,
    effective_version_minor INTEGER NOT NULL,
    effective_version_patch INTEGER NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Create major_upgrade_operations table
CREATE TABLE major_upgrade_operations (
    id UUID PRIMARY KEY,
    asset_id UUID NOT NULL REFERENCES asset_instances(id),
    from_version_major INTEGER NOT NULL,
    from_version_minor INTEGER NOT NULL,
    from_version_patch INTEGER NOT NULL,
    to_version_major INTEGER NOT NULL,
    to_version_minor INTEGER NOT NULL,
    to_version_patch INTEGER NOT NULL,
    snapshot_id UUID NOT NULL,
    status VARCHAR(50) NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMP
);

-- Indexes
CREATE INDEX idx_versions_asset ON asset_versions(asset_id);
CREATE INDEX idx_versions_unpublished ON asset_versions(is_unpublished) WHERE is_unpublished = TRUE;
CREATE INDEX idx_deps_upstream ON asset_dependencies(upstream_asset_id);
CREATE INDEX idx_deps_downstream ON asset_dependencies(downstream_asset_id);
CREATE INDEX idx_deps_version_lookup ON asset_dependencies(
    upstream_asset_id, 
    effective_version_major, 
    effective_version_minor, 
    effective_version_patch
);
CREATE INDEX idx_dirty_logs_asset ON dirty_resolution_logs(asset_id);
CREATE INDEX idx_dirty_logs_unresolved ON dirty_resolution_logs(asset_id, resolved_at) WHERE resolved_at IS NULL;
CREATE INDEX idx_type_rules_org ON dependency_type_rules(organization_id);
```

**Step 2: Commit**

```bash
git add migrations/V3__version_constraints.sql
git commit -m "db: add version constraints schema migration"
```

---

## Phase 4: Application Layer - Core Services

### Task 6: Implement DependencyService with constraint resolution

**Files:**
- Create: `crates/adam-application/src/services/dependency_service.rs`

**Step 1: Implement service**

```rust
//! Dependency service with version constraint support

use std::collections::HashMap;
use std::sync::Arc;
use chrono::Utc;

use adam_domain::{
    asset::instance::{AssetId, AssetInstance},
    dependency::rule::{AssetDependency, DependencyId, UpgradePolicy},
    error::RepositoryError,
    organization::OrganizationId,
    repository::{AssetDependencyRepository, AssetInstanceRepository},
    version::{SemVer, VersionConstraint},
};

use crate::services::config::ConfigCache;

pub struct DependencyService {
    asset_repo: Arc<dyn AssetInstanceRepository>,
    dependency_repo: Arc<dyn AssetDependencyRepository>,
    config_cache: Arc<ConfigCache>,
}

impl DependencyService {
    pub fn new(
        asset_repo: Arc<dyn AssetInstanceRepository>,
        dependency_repo: Arc<dyn AssetDependencyRepository>,
        config_cache: Arc<ConfigCache>,
    ) -> Self {
        Self {
            asset_repo,
            dependency_repo,
            config_cache,
        }
    }

    /// Create dependency with layered configuration
    pub async fn create_dependency(
        &self,
        downstream_id: AssetId,
        upstream_id: AssetId,
        explicit_constraint: Option<VersionConstraint>,
        explicit_policy: Option<UpgradePolicy>,
    ) -> Result<AssetDependency, DependencyError> {
        // Get asset info
        let downstream = self.asset_repo.find_by_id(&downstream_id).await?;
        let upstream = self.asset_repo.find_by_id(&upstream_id).await?;

        // Layer 1: Explicit
        let constraint = if let Some(c) = explicit_constraint {
            c
        } else {
            // Try cache for type rule
            let type_rule = self.config_cache
                .get_type_rule(downstream.asset_type_id, upstream.asset_type_id)
                .await;
            
            if let Some(rule) = type_rule {
                self.apply_template(&rule.default_template, &upstream.current_version)
            } else {
                // Fallback to org policy
                let org_policy = self.config_cache
                    .get_org_policy(downstream.organization_id)
                    .await;
                self.apply_template(&org_policy.default_template, &upstream.current_version)
            }
        };

        let policy = explicit_policy
            .or_else(|| {
                self.config_cache
                    .get_type_rule(downstream.asset_type_id, upstream.asset_type_id)
                    .await
                    .map(|r| r.default_policy)
            })
            .unwrap_or(UpgradePolicy::Notify);

        let dependency = AssetDependency {
            id: DependencyId::new(),
            downstream_id,
            upstream_id,
            declared_constraint: constraint,
            constraint_str: constraint.to_string(),
            effective_version: upstream.current_version.clone(),
            upgrade_policy: policy,
            lock_version: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        self.dependency_repo.create(&dependency).await?;
        Ok(dependency)
    }

    fn apply_template(&self, template: &ConstraintTemplate, current: &SemVer) -> VersionConstraint {
        match template {
            ConstraintTemplate::FollowMajor => {
                VersionConstraint::Caret(SemVer::new(current.major, 0, 0))
            }
            ConstraintTemplate::ExactCurrent => {
                VersionConstraint::Exact(current.clone())
            }
            ConstraintTemplate::FollowMinor => {
                VersionConstraint::Tilde(SemVer::new(current.major, current.minor, 0))
            }
            ConstraintTemplate::Wildcard => VersionConstraint::Wildcard,
        }
    }
}
```

**Step 2: Commit**

```bash
git commit -m "feat: add DependencyService with constraint resolution"
```

---

### Task 7: Implement StatePropagationService with transaction

**Files:**
- Create: `crates/adam-application/src/services/state_propagation.rs`

**Step 1: Implement with transactions**

```rust
//! State propagation with transaction support

use sqlx::{Postgres, Transaction};

pub struct StatePropagationService {
    pool: PgPool,
    dependency_repo: Arc<dyn AssetDependencyRepository>,
    dirty_log_repo: Arc<dyn DirtyResolutionLogRepository>,
}

impl StatePropagationService {
    /// Propagate with explicit transaction
    pub async fn propagate_on_publish(
        &self,
        upstream_id: &AssetId,
        new_version: &SemVer,
    ) -> Result<PropagationResult, StateError> {
        let mut tx = self.pool.begin().await?;
        
        let result = self
            .propagate_in_transaction(&mut tx, upstream_id, new_version)
            .await?;
        
        tx.commit().await?;
        Ok(result)
    }

    async fn propagate_in_transaction(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        upstream_id: &AssetId,
        new_version: &SemVer,
    ) -> Result<PropagationResult, StateError> {
        // Find dependencies in transaction
        let deps = self.dependency_repo
            .find_by_upstream_in_tx(tx, upstream_id)
            .await?;
        
        let mut affected = Vec::new();
        
        for dep in deps {
            // Check constraint match
            if !dep.declared_constraint.matches(new_version) {
                continue;
            }
            
            // Check if should mark dirty
            if self.should_mark_dirty(&dep, new_version) {
                // Mark dirty in transaction
                self.mark_dirty_in_tx(tx, &dep.downstream_id).await?;
                
                // Create aggregated dirty log
                self.create_or_update_dirty_log_in_tx(
                    tx,
                    &dep.downstream_id,
                    upstream_id,
                    new_version,
                ).await?;
                
                affected.push(dep.downstream_id);
            } else {
                // Auto-accept: update effective_version in transaction
                self.dependency_repo
                    .update_effective_version_in_tx(tx, &dep.id, new_version)
                    .await?;
            }
        }
        
        Ok(PropagationResult { 
            affected_count: affected.len(),
            assets: affected,
        })
    }

    async fn create_or_update_dirty_log_in_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        asset_id: &AssetId,
        upstream_id: &AssetId,
        version: &SemVer,
    ) -> Result<(), StateError> {
        // Check for existing unresolved log
        let existing = self.dirty_log_repo
            .find_unresolved_in_tx(tx, asset_id, upstream_id)
            .await?;
        
        if let Some(mut log) = existing {
            // Update aggregation
            log.latest_trigger_version = version.clone();
            log.latest_triggered_at = Utc::now();
            log.trigger_count += 1;
            log.aggregated = log.trigger_count > 1;
            
            self.dirty_log_repo.update_in_tx(tx, &log).await?;
        } else {
            // Create new
            let now = Utc::now();
            let log = DirtyResolutionLog {
                id: DirtyLogId::new(),
                asset_id: *asset_id,
                upstream_id: *upstream_id,
                latest_trigger_version: version.clone(),
                latest_triggered_at: now,
                first_triggered_at: now,
                trigger_count: 1,
                aggregated: false,
                resolution_type: ResolutionType::Pending,
                resolved_by: None,
                resolved_at: None,
                notes: None,
            };
            self.dirty_log_repo.create_in_tx(tx, &log).await?;
        }
        
        Ok(())
    }
}
```

**Step 2: Commit**

```bash
git commit -m "feat: add StatePropagationService with transaction support"
```

---

## Phase 5: Advanced Features

### Task 8: Implement CAS optimistic locking

**Files:**
- Create: `crates/adam-application/src/services/asset_lifecycle.rs`

**Step 1: Implement CAS publish**

```rust
//! Asset lifecycle with CAS optimistic locking

use sqlx::PgPool;

pub struct AssetLifecycleService {
    pool: PgPool,
    asset_repo: Arc<dyn AssetInstanceRepository>,
    version_repo: Arc<dyn AssetVersionRepository>,
    propagation_service: Arc<StatePropagationService>,
}

impl AssetLifecycleService {
    /// Publish with CAS (Compare-And-Swap)
    pub async fn publish_version_cas(
        &self,
        asset_id: AssetId,
        new_version: SemVer,
        content_ref: String,
        expected_lock_version: i64,
    ) -> Result<AssetVersion, AssetError> {
        // CAS update
        let result = sqlx::query(
            r#"
            UPDATE asset_instances 
            SET current_version_major = $2,
                current_version_minor = $3,
                current_version_patch = $4,
                lock_version = lock_version + 1,
                updated_at = NOW()
            WHERE id = $1 AND lock_version = $5
              AND (
                  current_version_major < $2 OR
                  (current_version_major = $2 AND current_version_minor < $3) OR
                  (current_version_major = $2 AND current_version_minor = $3 AND current_version_patch < $4)
              )
            RETURNING lock_version
            "#
        )
        .bind(&asset_id)
        .bind(new_version.major as i32)
        .bind(new_version.minor as i32)
        .bind(new_version.patch as i32)
        .bind(expected_lock_version)
        .fetch_optional(&self.pool)
        .await?;
        
        match result {
            Some(row) => {
                // Success
                let version = AssetVersion::new(
                    asset_id,
                    new_version.clone(),
                    content_ref,
                );
                self.version_repo.create(&version).await?;
                
                // Propagate
                self.propagation_service
                    .propagate_on_publish(&asset_id, &new_version)
                    .await?;
                
                Ok(version)
            }
            None => {
                // Conflict
                let asset = self.asset_repo.find_by_id(&asset_id).await?;
                Err(AssetError::ConcurrentModification {
                    asset_id,
                    expected: expected_lock_version,
                    actual: asset.lock_version,
                })
            }
        }
    }

    /// Publish with retry and exponential backoff
    pub async fn publish_with_retry(
        &self,
        asset_id: AssetId,
        new_version: SemVer,
        content_ref: String,
        config: RetryConfig,
    ) -> Result<AssetVersion, AssetError> {
        let mut attempt = 0;
        let mut expected_version = self
            .asset_repo
            .find_by_id(&asset_id)
            .await?
            .lock_version;
        
        loop {
            match self.publish_version_cas(
                asset_id,
                new_version.clone(),
                content_ref.clone(),
                expected_version,
            ).await {
                Ok(version) => return Ok(version),
                Err(AssetError::ConcurrentModification) if attempt < config.max_retries => {
                    // Exponential backoff
                    let delay = std::cmp::min(
                        config.base_delay * 2u32.pow(attempt),
                        config.max_delay,
                    );
                    tokio::time::sleep(delay).await;
                    
                    // Refresh expected version
                    expected_version = self
                        .asset_repo
                        .find_by_id(&asset_id)
                        .await?
                        .lock_version;
                    
                    attempt += 1;
                }
                Err(e) => return Err(e),
            }
        }
    }
}
```

**Step 2: Commit**

```bash
git commit -m "feat: add CAS optimistic locking with retry"
```

---

### Task 9: Implement idempotent publish

**Files:**
- Modify: `crates/adam-application/src/services/asset_lifecycle.rs`

**Step 1: Add idempotency**

```rust
/// Idempotent publish request
#[derive(Debug, Clone)]
pub struct PublishVersionRequest {
    pub asset_id: AssetId,
    pub new_version: SemVer,
    pub content_ref: String,
    pub idempotency_key: String,
    pub expected_lock_version: i64,
}

impl AssetLifecycleService {
    /// Idempotent publish
    pub async fn publish_version_idempotent(
        &self,
        request: PublishVersionRequest,
    ) -> Result<AssetVersion, AssetError> {
        // Check idempotency
        if let Some(existing) = self.idempotency_repo
            .find_by_key(&request.idempotency_key)
            .await?
        {
            // Verify request matches
            let request_hash = self.compute_request_hash(&request);
            if existing.request_hash == request_hash {
                return self.version_repo.find_by_id(&existing.response_id).await?;
            } else {
                return Err(AssetError::IdempotencyKeyConflict);
            }
        }
        
        // Execute
        let version = self.publish_version_cas(
            request.asset_id,
            request.new_version.clone(),
            request.content_ref.clone(),
            request.expected_lock_version,
        ).await?;
        
        // Record idempotency
        let record = IdempotencyRecord {
            key: request.idempotency_key,
            request_hash: self.compute_request_hash(&request),
            response_id: version.id,
            created_at: Utc::now(),
            expires_at: Utc::now() + Duration::hours(24),
        };
        self.idempotency_repo.save(&record).await?;
        
        Ok(version)
    }

    fn compute_request_hash(&self, request: &PublishVersionRequest) -> String {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(request.asset_id.to_string());
        hasher.update(request.new_version.to_string());
        hasher.update(&request.content_ref);
        format!("{:x}", hasher.finalize())
    }
}
```

**Step 2: Commit**

```bash
git commit -m "feat: add idempotent publish with idempotency_key"
```

---

### Task 10: Implement unpublish with propagation

**Files:**
- Create: `crates/adam-application/src/services/unpublish.rs`

**Step 1: Implement unpublish service**

```rust
//! Version unpublish service

pub struct UnpublishService {
    version_repo: Arc<dyn AssetVersionRepository>,
    dependency_repo: Arc<dyn AssetDependencyRepository>,
    dirty_log_repo: Arc<dyn DirtyResolutionLogRepository>,
}

impl UnpublishService {
    /// Unpublish a version
    pub async fn unpublish_version(
        &self,
        asset_id: AssetId,
        version: SemVer,
        reason: String,
    ) -> Result<(), UnpublishError> {
        // Check policy
        let config = self.get_org_config(asset_id).await?;
        
        match config.policy {
            UnpublishPolicy::Never => {
                return Err(UnpublishError::NotAllowed);
            }
            UnpublishPolicy::AllowWithin(duration) => {
                let version_record = self.version_repo
                    .find_by_version(&asset_id, &version)
                    .await?;
                
                if Utc::now() - version_record.released_at > duration {
                    return Err(UnpublishError::WindowExpired);
                }
            }
            UnpublishPolicy::RequireApproval => {
                self.create_approval(asset_id, version, reason).await?;
                return Ok(());
            }
        }
        
        // Mark as unpublished
        self.version_repo
            .mark_unpublished(&asset_id, &version, &reason)
            .await?;
        
        // Propagate to downstream
        match config.propagation {
            UnpublishPropagation::NotifyDownstream => {
                self.propagate_unpublish(&asset_id, &version).await?;
            }
            _ => {}
        }
        
        Ok(())
    }

    async fn propagate_unpublish(
        &self,
        upstream_id: &AssetId,
        version: &SemVer,
    ) -> Result<(), UnpublishError> {
        // Find downstreams using this version
        let deps = self.dependency_repo
            .find_by_upstream_and_version(upstream_id, version)
            .await?;
        
        for dep in deps {
            // Mark as dirty
            self.dirty_log_repo
                .mark_dirty(&dep.downstream_id, upstream_id, version)
                .await?;
        }
        
        Ok(())
    }
}
```

**Step 2: Commit**

```bash
git commit -m "feat: add unpublish service with propagation"
```

---

### Task 11: Implement Major upgrade rollback

**Files:**
- Create: `crates/adam-application/src/services/major_upgrade.rs`

**Step 1: Implement with snapshots**

```rust
//! Major upgrade with snapshot-based rollback

pub struct MajorUpgradeService {
    asset_repo: Arc<dyn AssetInstanceRepository>,
    dependency_repo: Arc<dyn AssetDependencyRepository>,
    snapshot_repo: Arc<dyn DependencySnapshotRepository>,
    operation_repo: Arc<dyn MajorUpgradeOperationRepository>,
}

impl MajorUpgradeService {
    /// Upgrade with snapshot for rollback
    pub async fn upgrade_major_with_snapshot(
        &self,
        asset_id: AssetId,
        target_version: SemVer,
    ) -> Result<UpgradeResult, UpgradeError> {
        let asset = self.asset_repo.find_by_id(&asset_id).await?;
        
        // Validate major upgrade
        if target_version.major <= asset.current_version.major {
            return Err(UpgradeError::NotMajorUpgrade);
        }
        
        // Create dependency snapshot
        let snapshot = self.create_snapshot(asset_id).await?;
        
        // Create operation record
        let operation = MajorUpgradeOperation {
            id: OperationId::new(),
            asset_id,
            from_version: asset.current_version.clone(),
            to_version: target_version.clone(),
            snapshot_id: snapshot.id,
            status: OperationStatus::InProgress,
            created_at: Utc::now(),
            completed_at: None,
        };
        self.operation_repo.create(&operation).await?;
        
        // Perform upgrade
        let result = self.perform_upgrade(asset_id, target_version).await;
        
        // Update operation status
        match &result {
            Ok(_) => {
                self.operation_repo
                    .mark_completed(&operation.id)
                    .await?;
            }
            Err(e) => {
                self.operation_repo
                    .mark_failed(&operation.id, &e.to_string())
                    .await?;
                
                // Auto-rollback
                self.rollback_from_snapshot(&snapshot.id).await?;
            }
        }
        
        result
    }

    async fn create_snapshot(
        &self,
        asset_id: AssetId,
    ) -> Result<DependencySnapshot, UpgradeError> {
        let deps = self.dependency_repo
            .find_by_downstream(&asset_id)
            .await?;
        
        let snapshot = DependencySnapshot {
            id: SnapshotId::new(),
            asset_id,
            created_at: Utc::now(),
            dependencies: deps.iter().map(|d| SnapshotDependency {
                dependency_id: d.id,
                upstream_id: d.upstream_id,
                declared_constraint: d.declared_constraint.clone(),
                effective_version: d.effective_version.clone(),
            }).collect(),
        };
        
        self.snapshot_repo.create(&snapshot).await?;
        Ok(snapshot)
    }

    pub async fn rollback_from_snapshot(
        &self,
        snapshot_id: &SnapshotId,
    ) -> Result<(), UpgradeError> {
        let snapshot = self.snapshot_repo.find_by_id(snapshot_id).await?;
        
        // Restore dependencies
        for dep in &snapshot.dependencies {
            self.dependency_repo
                .rollback_to_snapshot(
                    &dep.dependency_id,
                    &dep.declared_constraint,
                    &dep.effective_version,
                )
                .await?;
        }
        
        // Mark asset as dirty
        self.asset_repo.mark_dirty(&snapshot.asset_id).await?;
        
        tracing::info!("Rolled back asset {} from snapshot {}", 
            snapshot.asset_id, snapshot_id);
        
        Ok(())
    }
}
```

**Step 2: Commit**

```bash
git commit -m "feat: add Major upgrade with snapshot rollback"
```

---

## Phase 6: Performance Optimizations

### Task 12: Implement ConfigCache

**Files:**
- Create: `crates/adam-application/src/services/config_cache.rs`

**Step 1: Implement caching**

```rust
//! Configuration cache for layered config resolution

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use chrono::{DateTime, Duration, Utc};

use adam_domain::{
    asset::asset_type::AssetTypeId,
    organization::OrganizationId,
};

pub struct ConfigCache {
    type_rules: Arc<RwLock<HashMap<(AssetTypeId, AssetTypeId), DependencyTypeRule>>>,
    org_policies: Arc<RwLock<HashMap<OrganizationId, OrganizationPolicy>>>,
    last_updated: Arc<RwLock<DateTime<Utc>>>,
    type_rule_repo: Arc<dyn DependencyTypeRuleRepository>,
    org_policy_repo: Arc<dyn OrganizationPolicyRepository>,
}

impl ConfigCache {
    pub fn new(
        type_rule_repo: Arc<dyn DependencyTypeRuleRepository>,
        org_policy_repo: Arc<dyn OrganizationPolicyRepository>,
    ) -> Self {
        Self {
            type_rules: Arc::new(RwLock::new(HashMap::new())),
            org_policies: Arc::new(RwLock::new(HashMap::new())),
            last_updated: Arc::new(RwLock::new(Utc::now() - Duration::days(1))),
            type_rule_repo,
            org_policy_repo,
        }
    }

    /// Preload organization config
    pub async fn preload(&self, org_id: OrganizationId) -> Result<(), Error> {
        // Load type rules
        let rules = self.type_rule_repo
            .find_by_organization(org_id)
            .await?;
        
        let mut rules_map = self.type_rules.write().await;
        for rule in rules {
            rules_map.insert(
                (rule.downstream_type, rule.upstream_type),
                rule
            );
        }
        
        // Load org policy
        let policy = self.org_policy_repo
            .find_by_organization(org_id)
            .await?;
        
        let mut policy_map = self.org_policies.write().await;
        policy_map.insert(org_id, policy);
        
        *self.last_updated.write().await = Utc::now();
        
        Ok(())
    }

    /// Get type rule (from cache)
    pub async fn get_type_rule(
        &self,
        downstream: AssetTypeId,
        upstream: AssetTypeId,
    ) -> Option<DependencyTypeRule> {
        self.type_rules
            .read()
            .await
            .get(&(downstream, upstream))
            .cloned()
    }

    /// Get org policy (from cache)
    pub async fn get_org_policy(
        &self,
        org_id: OrganizationId,
    ) -> OrganizationPolicy {
        self.org_policies
            .read()
            .await
            .get(&org_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Check if cache needs refresh
    pub async fn should_refresh(&self) -> bool {
        let last = *self.last_updated.read().await;
        Utc::now() - last > Duration::minutes(5)
    }
}
```

**Step 2: Commit**

```bash
git commit -m "feat: add ConfigCache for layered config resolution"
```

---

### Task 13: Implement CompiledDependency

**Files:**
- Create: `crates/adam-domain/src/dependency/compiled.rs`

**Step 1: Implement compiled dependency**

```rust
//! Pre-compiled dependency for performance

use semver::VersionReq;
use super::{AssetDependency, DependencyId, UpgradePolicy};
use crate::asset::instance::{AssetId, AssetInstance};
use crate::version::{SemVer, VersionConstraint};

/// Pre-compiled dependency with version stamp
#[derive(Debug, Clone)]
pub struct CompiledDependency {
    pub id: DependencyId,
    pub downstream_id: AssetId,
    pub upstream_id: AssetId,
    pub constraint_str: String,
    pub compiled_constraint: VersionReq,
    pub effective_version: SemVer,
    pub upgrade_policy: UpgradePolicy,
    pub constraint_version: i64,  // version stamp for staleness check
}

impl CompiledDependency {
    /// Compile from raw dependency
    pub fn compile(dep: &AssetDependency) -> Result<Self, semver::Error> {
        let compiled = VersionReq::parse(&dep.constraint_str)?;
        
        Ok(Self {
            id: dep.id,
            downstream_id: dep.downstream_id,
            upstream_id: dep.upstream_id,
            constraint_str: dep.constraint_str.clone(),
            compiled_constraint: compiled,
            effective_version: dep.effective_version.clone(),
            upgrade_policy: dep.upgrade_policy,
            constraint_version: dep.lock_version,
        })
    }
    
    /// Check if compiled constraint is stale
    pub fn is_stale(&self, db_constraint: &str, db_version: i64) -> bool {
        self.constraint_str != db_constraint || 
        self.constraint_version != db_version
    }
    
    /// Fast match using compiled constraint
    pub fn matches(&self, version: &SemVer) -> bool {
        let semver_version = semver::Version::new(
            version.major,
            version.minor,
            version.patch,
        );
        self.compiled_constraint.matches(&semver_version)
    }
}
```

**Step 2: Commit**

```bash
git commit -m "feat: add CompiledDependency for fast constraint matching"
```

---

## Phase 7: Testing & Validation

### Task 14: Write comprehensive tests

**Files:**
- Create: `tests/version_constraint_integration_test.rs`

**Step 1: Integration tests**

```rust
//! Integration tests for version constraint model

#[tokio::test]
async fn test_constraint_matches_versions() {
    // Setup
    let constraint = VersionConstraint::parse("^1.0.0").unwrap();
    
    // Test matching
    assert!(constraint.matches(&SemVer::new(1, 0, 0)));
    assert!(constraint.matches(&SemVer::new(1, 5, 0)));
    assert!(!constraint.matches(&SemVer::new(2, 0, 0)));
}

#[tokio::test]
async fn test_dirty_propagation_with_constraint() {
    // Setup: A depends on B ^1.0.0
    // B publishes 1.0.1 → A should NOT be dirty (AutoPatch)
    // B publishes 2.0.0 → A should NOT be dirty (constraint not satisfied)
}

#[tokio::test]
async fn test_cas_optimistic_locking() {
    // Test concurrent modification detection
}

#[tokio::test]
async fn test_idempotent_publish() {
    // Test duplicate publish with same idempotency key
}

#[tokio::test]
async fn test_major_upgrade_rollback() {
    // Test snapshot and rollback
}
```

**Step 2: Commit**

```bash
git add tests/
git commit -m "test: add comprehensive integration tests"
```

---

## Phase 8: Documentation

### Task 15: Update all documentation

**Files:**
- Update: `docs/api.md`
- Update: `docs/architecture.md`
- Create: `docs/migration-v2-to-v3.md`

**Step 1: Write migration guide**

Create `docs/migration-v2-to-v3.md` with detailed steps for migrating from Fork model to Constraint model.

**Step 2: Update API docs**

Update REST API documentation with new endpoints.

**Step 3: Commit**

```bash
git add docs/
git commit -m "docs: update documentation for version constraint model"
```

---

## Final Checklist

- [ ] All tests pass: `cargo test --workspace`
- [ ] Code compiles: `cargo build --release`
- [ ] No clippy warnings: `cargo clippy -- -D warnings`
- [ ] Code formatted: `cargo fmt`
- [ ] Migration tested on staging database
- [ ] Performance benchmarks pass
- [ ] Documentation complete

---

## Timeline Summary

| Phase | Tasks | Duration |
|-------|-------|----------|
| Phase 1 | SemVer & VersionConstraint | Week 1 |
| Phase 2 | Asset & Dependency Updates | Week 2 |
| Phase 3 | Database Schema | Week 2-3 |
| Phase 4 | Core Services | Week 3-4 |
| Phase 5 | Advanced Features | Week 5-6 |
| Phase 6 | Performance Optimizations | Week 6-7 |
| Phase 7 | Testing | Week 7-8 |
| Phase 8 | Documentation | Week 8 |

**Total: 8 weeks**

---

*End of Implementation Plan*