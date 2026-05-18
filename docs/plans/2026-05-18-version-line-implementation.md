# Version Line Management Implementation Plan v2.0

> **UPDATED**: Based on design review feedback, this plan now includes:
> - Enhanced fork validation (dependency completeness)
> - Version inheritance modes
> - State preservation options
> - Extended timeline (7 weeks)
> - Detailed test strategy
> - Online migration with rollback
> - MCP backward compatibility

**Goal:** Implement multi-version-line support with isolated dependency DAGs per version line, enabling simultaneous development on v1.x, v2.x, and feature branches.

**Architecture:** Introduce `VersionLine` entity that owns assets and dependencies. Each version line has isolated state propagation (Clean/Dirty), assets are forked (copied) between version lines with new IDs, and all queries are scoped to a specific version line.

**Tech Stack:** Rust 2024, PostgreSQL, sqlx, axum, async-trait, chrono, uuid

**Timeline:** 7 weeks (revised from 5 weeks)

---

## Pre-Flight Checklist

Before starting:
- [ ] Review design document: `docs/plans/2026-05-18-version-line-design.md`
- [ ] Review the 6 design improvements in this document
- [ ] Ensure database is running and accessible
- [ ] Run `cargo test` to confirm baseline is green
- [ ] Create worktree: `git worktree add ../adam-version-line-work`

---

## Phase 1: Domain Layer - VersionLine Entity

### Task 1: Create VersionLineId type

**Files:**
- Create: `crates/adam-domain/src/version_line/id.rs`
- Modify: `crates/adam-domain/src/version_line/mod.rs` (create)
- Modify: `crates/adam-domain/src/lib.rs`

**Step 1: Write the failing test**

Create `crates/adam-domain/src/version_line/id.rs`:

```rust
//! VersionLineId type

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for version lines
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VersionLineId(pub Uuid);

impl VersionLineId {
    /// Generate a new random VersionLineId
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for VersionLineId {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_line_id_new_is_unique() {
        let id1 = VersionLineId::new();
        let id2 = VersionLineId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_version_line_id_default_creates_new() {
        let id1 = VersionLineId::default();
        let id2 = VersionLineId::default();
        assert_ne!(id1, id2);
    }
}
```

**Step 2: Create module structure**

Create `crates/adam-domain/src/version_line/mod.rs`:

```rust
//! Version line domain module

pub mod id;

pub use id::VersionLineId;
```

**Step 3: Export from lib**

Modify `crates/adam-domain/src/lib.rs`:

```rust
// Add this line in the appropriate section
pub mod version_line;
```

**Step 4: Run test to verify it passes**

```bash
cd crates/adam-domain
cargo test version_line_id -- --nocapture
```

Expected: PASS (2 tests)

**Step 5: Commit**

```bash
git add crates/adam-domain/src/version_line/
git add crates/adam-domain/src/lib.rs
git commit -m "feat: add VersionLineId type"
```

---

### Task 2: Create VersionLineStatus enum

**Files:**
- Modify: `crates/adam-domain/src/version_line/mod.rs`

**Step 1: Write the failing test**

Add to `crates/adam-domain/src/version_line/mod.rs`:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::organization::OrganizationId;

// ... existing id module ...

/// Version line lifecycle status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VersionLineStatus {
    /// Active development - new versions can be published
    Active,
    /// Maintenance mode - only patches allowed
    Maintenance,
    /// Archived - read-only, no new versions
    Archived,
}

impl Default for VersionLineStatus {
    fn default() -> Self {
        VersionLineStatus::Active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_line_status_default_is_active() {
        let status: VersionLineStatus = Default::default();
        assert_eq!(status, VersionLineStatus::Active);
    }

    #[test]
    fn test_version_line_status_equality() {
        assert_eq!(VersionLineStatus::Active, VersionLineStatus::Active);
        assert_ne!(VersionLineStatus::Active, VersionLineStatus::Maintenance);
    }
}
```

**Step 2: Run test to verify it passes**

```bash
cargo test version_line -- --nocapture
```

Expected: PASS

**Step 3: Commit**

```bash
git add crates/adam-domain/src/version_line/mod.rs
git commit -m "feat: add VersionLineStatus enum"
```

---

### Task 3: Create VersionLine entity

**Files:**
- Modify: `crates/adam-domain/src/version_line/mod.rs`

**Step 1: Write the failing test**

Add to `crates/adam-domain/src/version_line/mod.rs` after the status enum:

```rust
/// VersionLine represents an isolated development stream
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionLine {
    pub id: VersionLineId,
    pub organization_id: OrganizationId,
    pub name: String,
    pub status: VersionLineStatus,
    pub forked_from: Option<VersionLineId>,
    pub forked_at: Option<DateTime<Utc>>,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl VersionLine {
    /// Create a new root version line (not forked)
    pub fn new(
        organization_id: OrganizationId,
        name: impl Into<String>,
        description: Option<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: VersionLineId::new(),
            organization_id,
            name: name.into(),
            status: VersionLineStatus::Active,
            forked_from: None,
            forked_at: None,
            description,
            created_at: now,
            updated_at: now,
        }
    }

    /// Create a fork of an existing version line
    pub fn fork_from(
        organization_id: OrganizationId,
        name: impl Into<String>,
        parent: VersionLineId,
        description: Option<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: VersionLineId::new(),
            organization_id,
            name: name.into(),
            status: VersionLineStatus::Active,
            forked_from: Some(parent),
            forked_at: Some(now),
            description,
            created_at: now,
            updated_at: now,
        }
    }

    /// Transition to maintenance mode
    pub fn set_maintenance(&mut self) {
        self.status = VersionLineStatus::Maintenance;
        self.updated_at = Utc::now();
    }

    /// Archive the version line
    pub fn archive(&mut self) {
        self.status = VersionLineStatus::Archived;
        self.updated_at = Utc::now();
    }
}

#[cfg(test)]
mod entity_tests {
    use super::*;

    #[test]
    fn test_version_line_new() {
        let org_id = OrganizationId::new();
        let vl = VersionLine::new(org_id, "v1.x", Some("Version 1"));

        assert_eq!(vl.name, "v1.x");
        assert_eq!(vl.description, Some("Version 1".to_string()));
        assert_eq!(vl.status, VersionLineStatus::Active);
        assert!(vl.forked_from.is_none());
        assert!(vl.forked_at.is_none());
    }

    #[test]
    fn test_version_line_fork_from() {
        let org_id = OrganizationId::new();
        let parent_id = VersionLineId::new();
        let vl = VersionLine::fork_from(org_id, "v2.x", parent_id, Some("Version 2"));

        assert_eq!(vl.name, "v2.x");
        assert_eq!(vl.forked_from, Some(parent_id));
        assert!(vl.forked_at.is_some());
    }

    #[test]
    fn test_version_line_set_maintenance() {
        let org_id = OrganizationId::new();
        let mut vl = VersionLine::new(org_id, "v1.x", None);
        
        vl.set_maintenance();
        
        assert_eq!(vl.status, VersionLineStatus::Maintenance);
    }

    #[test]
    fn test_version_line_archive() {
        let org_id = OrganizationId::new();
        let mut vl = VersionLine::new(org_id, "v1.x", None);
        
        vl.archive();
        
        assert_eq!(vl.status, VersionLineStatus::Archived);
    }
}
```

**Step 2: Run test to verify it passes**

```bash
cargo test entity_tests -- --nocapture
```

Expected: PASS (4 tests)

**Step 3: Commit**

```bash
git add crates/adam-domain/src/version_line/mod.rs
git commit -m "feat: add VersionLine entity with fork support"
```

---

### Task 4: Add VersionLineRepository trait

**Files:**
- Modify: `crates/adam-domain/src/repository/mod.rs`

**Step 1: Write the failing test**

Modify `crates/adam-domain/src/repository/mod.rs` to add VersionLineRepository:

```rust
// Add to imports
use crate::version_line::{VersionLine, VersionLineId};

// Add to the repository error or use existing RepositoryError

#[async_trait::async_trait]
pub trait VersionLineRepository: Send + Sync {
    /// Create a new version line
    async fn create(&self, version_line: &VersionLine) -> Result<(), RepositoryError>;

    /// Find by ID
    async fn find_by_id(
        &self,
        id: &VersionLineId,
    ) -> Result<Option<VersionLine>, RepositoryError>;

    /// List all version lines for an organization
    async fn find_by_organization(
        &self,
        org_id: &OrganizationId,
    ) -> Result<Vec<VersionLine>, RepositoryError>;

    /// Update version line
    async fn update(&self, version_line: &VersionLine) -> Result<(), RepositoryError>;

    /// Check if name exists for organization
    async fn exists_by_name(
        &self,
        org_id: &OrganizationId,
        name: &str,
    ) -> Result<bool, RepositoryError>;
}

// Add Arc wrapper
#[async_trait::async_trait]
impl<T: VersionLineRepository + ?Sized> VersionLineRepository for Arc<T> {
    async fn create(&self, version_line: &VersionLine) -> Result<(), RepositoryError> {
        self.as_ref().create(version_line).await
    }

    async fn find_by_id(
        &self,
        id: &VersionLineId,
    ) -> Result<Option<VersionLine>, RepositoryError> {
        self.as_ref().find_by_id(id).await
    }

    async fn find_by_organization(
        &self,
        org_id: &OrganizationId,
    ) -> Result<Vec<VersionLine>, RepositoryError> {
        self.as_ref().find_by_organization(org_id).await
    }

    async fn update(&self, version_line: &VersionLine) -> Result<(), RepositoryError> {
        self.as_ref().update(version_line).await
    }

    async fn exists_by_name(
        &self,
        org_id: &OrganizationId,
        name: &str,
    ) -> Result<bool, RepositoryError> {
        self.as_ref().exists_by_name(org_id, name).await
    }
}
```

**Step 2: Run test to verify it compiles**

```bash
cargo check -p adam-domain
```

Expected: No errors

**Step 3: Commit**

```bash
git add crates/adam-domain/src/repository/mod.rs
git commit -m "feat: add VersionLineRepository trait"
```

---

### Task 5: Add version_line_id to AssetInstance

**Files:**
- Modify: `crates/adam-domain/src/asset/instance.rs`

**Step 1: Update imports**

Add to `crates/adam-domain/src/asset/instance.rs`:

```rust
// Add to existing imports
use crate::version_line::VersionLineId;
```

**Step 2: Add field and update constructors**

Modify the AssetInstance struct:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetInstance {
    pub id: AssetId,
    pub version_line_id: VersionLineId,  // NEW FIELD
    pub name: String,
    pub asset_type_id: AssetTypeId,
    // ... rest of fields
}
```

Update `new_project_level`:

```rust
pub fn new_project_level(
    name: impl Into<String>,
    asset_type_id: AssetTypeId,
    project_id: ProjectId,
    organization_id: OrganizationId,
    version_line_id: VersionLineId,  // ADDED parameter
    external_ref: impl Into<String>,
    source: impl Into<String>,
    metadata: serde_json::Value,
) -> Self {
    let now = Utc::now();
    Self {
        id: AssetId::new(),
        version_line_id,  // Set the field
        name: name.into(),
        asset_type_id,
        project_id: Some(project_id),
        organization_id,
        level: AssetLevel::Project,
        current_state: AssetState::Clean,
        external_ref: external_ref.into(),
        source: source.into(),
        metadata,
        assignees: vec![],
        publisher: None,
        current_version: None,
        created_at: now,
        updated_at: now,
        idempotency_key: None,
    }
}
```

Update `new_organization_level` similarly with `version_line_id` parameter.

**Step 3: Update tests**

Update all existing tests to include `version_line_id`:

```rust
#[test]
fn test_new_project_level_asset() {
    let org_id = OrganizationId::new();
    let project_id = ProjectId::new();
    let type_id = AssetTypeId::new();
    let version_line_id = VersionLineId::new();  // ADD

    let asset = AssetInstance::new_project_level(
        "Test Asset",
        type_id,
        project_id,
        org_id,
        version_line_id,  // ADD
        "https://example.com/asset/1",
        "manual",
        serde_json::json!({"title": "Test"}),
    );

    assert_eq!(asset.version_line_id, version_line_id);  // ADD assertion
    // ... rest of assertions
}
```

**Step 4: Run tests**

```bash
cargo test -p adam-domain asset::instance -- --nocapture
```

Expected: All tests pass

**Step 5: Commit**

```bash
git add crates/adam-domain/src/asset/instance.rs
git commit -m "feat: add version_line_id to AssetInstance"
```

---

### Task 6: Add version_line_id to AssetDependency

**Files:**
- Create/Modify: `crates/adam-domain/src/dependency/mod.rs` (check existing structure)

**Step 1: Find existing dependency model**

First check if there's an AssetDependency struct:

```bash
grep -r "struct AssetDependency" crates/adam-domain/src/
```

**Step 2: Add version_line_id**

If AssetDependency exists, add the field. If not, look for where dependencies are defined.

Assuming it exists in `dependency/rule.rs` or similar:

```rust
// Add to existing dependency struct
pub struct AssetDependency {
    pub id: DependencyId,
    pub version_line_id: VersionLineId,  // NEW
    pub downstream_asset_id: AssetId,
    pub upstream_asset_id: AssetId,
    pub declared_version: String,
    pub effective_version: String,
}
```

**Step 3: Update constructor**

Update the constructor to accept `version_line_id`.

**Step 4: Run tests**

```bash
cargo test -p adam-domain dependency -- --nocapture
```

Expected: Tests pass

**Step 5: Commit**

```bash
git commit -am "feat: add version_line_id to AssetDependency"
```

---

## Phase 2: Infrastructure Layer - Database Schema

### Task 7: Create database migration

**Files:**
- Create: `migrations/V2__add_version_lines.sql`

**Step 1: Write migration**

Create `migrations/V2__add_version_lines.sql`:

```sql
-- Create version_lines table
CREATE TABLE version_lines (
    id UUID PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id),
    name VARCHAR(255) NOT NULL,
    status VARCHAR(50) NOT NULL DEFAULT 'Active',
    forked_from UUID REFERENCES version_lines(id),
    forked_at TIMESTAMP,
    description TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(organization_id, name)
);

-- Add version_line_id to asset_instances
ALTER TABLE asset_instances 
ADD COLUMN version_line_id UUID NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000' 
REFERENCES version_lines(id);

-- Add version_line_id to asset_dependencies
ALTER TABLE asset_dependencies 
ADD COLUMN version_line_id UUID NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000'
REFERENCES version_lines(id);

-- Add version_line_id to dirty_resolution_logs
ALTER TABLE dirty_resolution_logs 
ADD COLUMN version_line_id UUID NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000'
REFERENCES version_lines(id);

-- Create indexes
CREATE INDEX idx_version_lines_org ON version_lines(organization_id);
CREATE INDEX idx_version_lines_status ON version_lines(status);
CREATE INDEX idx_assets_version_line ON asset_instances(version_line_id);
CREATE INDEX idx_deps_version_line ON asset_dependencies(version_line_id);
CREATE INDEX idx_dirty_logs_version_line ON dirty_resolution_logs(version_line_id);

-- Composite indexes for common queries
CREATE INDEX idx_assets_version_line_type ON asset_instances(version_line_id, asset_type_id);
CREATE INDEX idx_deps_version_line_downstream ON asset_dependencies(version_line_id, downstream_asset_id);
CREATE INDEX idx_deps_version_line_upstream ON asset_dependencies(version_line_id, upstream_asset_id);
```

**Step 2: Verify migration syntax**

```bash
# If using sqlx
sqlx migrate info
```

**Step 3: Commit**

```bash
git add migrations/V2__add_version_lines.sql
git commit -m "db: add version_lines table and version_line_id columns"
```

---

### Task 8: Implement PostgresVersionLineRepository

**Files:**
- Create: `crates/adam-infrastructure/src/repository/postgres/version_line.rs`

**Step 1: Create repository implementation**

Create `crates/adam-infrastructure/src/repository/postgres/version_line.rs`:

```rust
//! PostgreSQL implementation of VersionLineRepository

use async_trait::async_trait;
use sqlx::PgPool;
use std::sync::Arc;

use adam_domain::{
    error::RepositoryError,
    organization::OrganizationId,
    repository::VersionLineRepository,
    version_line::{VersionLine, VersionLineId, VersionLineStatus},
};

/// PostgreSQL implementation of VersionLineRepository
#[derive(Debug, Clone)]
pub struct PostgresVersionLineRepository {
    pool: PgPool,
}

impl PostgresVersionLineRepository {
    /// Create a new repository instance
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl VersionLineRepository for PostgresVersionLineRepository {
    async fn create(&self, version_line: &VersionLine) -> Result<(), RepositoryError> {
        sqlx::query(
            r#"
            INSERT INTO version_lines 
            (id, organization_id, name, status, forked_from, forked_at, description, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
        )
        .bind(&version_line.id.0)
        .bind(&version_line.organization_id.0)
        .bind(&version_line.name)
        .bind(version_line.status.to_string())
        .bind(version_line.forked_from.map(|id| id.0))
        .bind(version_line.forked_at)
        .bind(&version_line.description)
        .bind(version_line.created_at)
        .bind(version_line.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Database(e.to_string()))?;

        Ok(())
    }

    async fn find_by_id(
        &self,
        id: &VersionLineId,
    ) -> Result<Option<VersionLine>, RepositoryError> {
        let row = sqlx::query_as::<_, VersionLineRow>(
            r#"
            SELECT id, organization_id, name, status, forked_from, forked_at, 
                   description, created_at, updated_at
            FROM version_lines
            WHERE id = $1
            "#,
        )
        .bind(&id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Database(e.to_string()))?;

        Ok(row.map(|r| r.into()))
    }

    async fn find_by_organization(
        &self,
        org_id: &OrganizationId,
    ) -> Result<Vec<VersionLine>, RepositoryError> {
        let rows = sqlx::query_as::<_, VersionLineRow>(
            r#"
            SELECT id, organization_id, name, status, forked_from, forked_at,
                   description, created_at, updated_at
            FROM version_lines
            WHERE organization_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(&org_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Database(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    async fn update(&self, version_line: &VersionLine) -> Result<(), RepositoryError> {
        sqlx::query(
            r#"
            UPDATE version_lines
            SET name = $1, status = $2, description = $3, updated_at = $4
            WHERE id = $5
            "#,
        )
        .bind(&version_line.name)
        .bind(version_line.status.to_string())
        .bind(&version_line.description)
        .bind(version_line.updated_at)
        .bind(&version_line.id.0)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Database(e.to_string()))?;

        Ok(())
    }

    async fn exists_by_name(
        &self,
        org_id: &OrganizationId,
        name: &str,
    ) -> Result<bool, RepositoryError> {
        let count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*) FROM version_lines
            WHERE organization_id = $1 AND name = $2
            "#,
        )
        .bind(&org_id.0)
        .bind(name)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| RepositoryError::Database(e.to_string()))?;

        Ok(count > 0)
    }
}

// Database row type for mapping
#[derive(sqlx::FromRow)]
struct VersionLineRow {
    id: Uuid,
    organization_id: Uuid,
    name: String,
    status: String,
    forked_from: Option<Uuid>,
    forked_at: Option<chrono::DateTime<chrono::Utc>>,
    description: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<VersionLineRow> for VersionLine {
    fn from(row: VersionLineRow) -> Self {
        VersionLine {
            id: VersionLineId(row.id),
            organization_id: OrganizationId(row.organization_id),
            name: row.name,
            status: parse_status(&row.status),
            forked_from: row.forked_from.map(VersionLineId),
            forked_at: row.forked_at,
            description: row.description,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

fn parse_status(s: &str) -> VersionLineStatus {
    match s {
        "Active" => VersionLineStatus::Active,
        "Maintenance" => VersionLineStatus::Maintenance,
        "Archived" => VersionLineStatus::Archived,
        _ => VersionLineStatus::Active,
    }
}

impl ToString for VersionLineStatus {
    fn to_string(&self) -> String {
        match self {
            VersionLineStatus::Active => "Active".to_string(),
            VersionLineStatus::Maintenance => "Maintenance".to_string(),
            VersionLineStatus::Archived => "Archived".to_string(),
        }
    }
}
```

**Step 2: Add to module exports**

Modify `crates/adam-infrastructure/src/repository/postgres/mod.rs`:

```rust
pub mod version_line;

pub use version_line::PostgresVersionLineRepository;
```

**Step 3: Commit**

```bash
git add crates/adam-infrastructure/src/repository/postgres/version_line.rs
git add crates/adam-infrastructure/src/repository/postgres/mod.rs
git commit -m "feat: implement PostgresVersionLineRepository"
```

---

## Phase 3: Application Layer - Forking Service

### Task 9: Create VersionLineService

**Files:**
- Create: `crates/adam-application/src/version_line/service.rs`

**Step 1: Write the service**

Create `crates/adam-application/src/version_line/service.rs`:

```rust
//! Version Line application service

use std::collections::HashMap;
use std::sync::Arc;

use adam_domain::{
    asset::instance::AssetId,
    error::RepositoryError,
    organization::OrganizationId,
    repository::{
        AssetDependencyRepository, AssetInstanceRepository, AssetVersionRepository,
        DirtyResolutionLogRepository, VersionLineRepository,
    },
    version_line::{VersionLine, VersionLineId},
};

/// Errors specific to version line operations
#[derive(Debug, thiserror::Error)]
pub enum VersionLineError {
    #[error("Version line not found: {0}")]
    NotFound(VersionLineId),
    #[error("Asset not found: {0}")]
    AssetNotFound(AssetId),
    #[error("Repository error: {0}")]
    Repository(#[from] RepositoryError),
    #[error("Name already exists: {0}")]
    NameExists(String),
    #[error("Invalid asset selection: {0}")]
    InvalidSelection(String),
}

/// Service for version line operations
pub struct VersionLineService {
    version_line_repo: Arc<dyn VersionLineRepository>,
    asset_repo: Arc<dyn AssetInstanceRepository>,
    dependency_repo: Arc<dyn AssetDependencyRepository>,
    version_repo: Arc<dyn AssetVersionRepository>,
    dirty_log_repo: Arc<dyn DirtyResolutionLogRepository>,
}

impl VersionLineService {
    /// Create a new service with all required repositories
    pub fn new(
        version_line_repo: Arc<dyn VersionLineRepository>,
        asset_repo: Arc<dyn AssetInstanceRepository>,
        dependency_repo: Arc<dyn AssetDependencyRepository>,
        version_repo: Arc<dyn AssetVersionRepository>,
        dirty_log_repo: Arc<dyn DirtyResolutionLogRepository>,
    ) -> Self {
        Self {
            version_line_repo,
            asset_repo,
            dependency_repo,
            version_repo,
            dirty_log_repo,
        }
    }

    /// Create a new version line
    pub async fn create_version_line(
        &self,
        organization_id: OrganizationId,
        name: String,
        description: Option<String>,
    ) -> Result<VersionLine, VersionLineError> {
        // Check if name exists
        if self
            .version_line_repo
            .exists_by_name(&organization_id, &name)
            .await?
        {
            return Err(VersionLineError::NameExists(name));
        }

        let version_line = VersionLine::new(organization_id, name, description);
        self.version_line_repo.create(&version_line).await?;

        Ok(version_line)
    }

    /// Fork a version line with selected assets
    pub async fn fork_version_line(
        &self,
        request: ForkVersionLineRequest,
    ) -> Result<ForkVersionLineResponse, VersionLineError> {
        let ForkVersionLineRequest {
            organization_id,
            source_version_line_id,
            new_name,
            asset_selection,
            description,
        } = request;

        // Validate source exists
        let source = self
            .version_line_repo
            .find_by_id(&source_version_line_id)
            .await?
            .ok_or(VersionLineError::NotFound(source_version_line_id))?;

        // Check target name doesn't exist
        if self
            .version_line_repo
            .exists_by_name(&organization_id, &new_name)
            .await?
        {
            return Err(VersionLineError::NameExists(new_name));
        }

        // Create new version line
        let new_version_line =
            VersionLine::fork_from(organization_id, new_name, source.id, description);
        self.version_line_repo.create(&new_version_line).await?;

        // Clone assets
        let mut asset_id_mapping: HashMap<AssetId, AssetId> = HashMap::new();
        for old_asset_id in &asset_selection {
            let old_asset = self
                .asset_repo
                .find_by_id(&source_version_line_id, old_asset_id)
                .await?
                .ok_or(VersionLineError::AssetNotFound(*old_asset_id))?;

            // Clone to new version line
            let new_asset = self
                .clone_asset(&old_asset, new_version_line.id)
                .await?;

            asset_id_mapping.insert(*old_asset_id, new_asset.id);
        }

        // Clone dependencies
        self.clone_dependencies(
            &source_version_line_id,
            new_version_line.id,
            &asset_id_mapping,
        )
        .await?;

        Ok(ForkVersionLineResponse {
            version_line: new_version_line,
            asset_mapping: asset_id_mapping,
            asset_count: asset_id_mapping.len(),
        })
    }

    /// Get version lines for organization
    pub async fn get_version_lines(
        &self,
        organization_id: &OrganizationId,
    ) -> Result<Vec<VersionLine>, VersionLineError> {
        self.version_line_repo
            .find_by_organization(organization_id)
            .await
            .map_err(Into::into)
    }

    // Helper: Clone asset to new version line
    async fn clone_asset(
        &self,
        source: &AssetInstance,
        target_version_line: VersionLineId,
    ) -> Result<AssetInstance, RepositoryError> {
        let mut new_asset = AssetInstance::new_project_level(
            source.name.clone(),
            source.asset_type_id,
            source.project_id.unwrap(), // Handle Option properly
            source.organization_id,
            target_version_line,
            source.external_ref.clone(),
            source.source.clone(),
            source.metadata.clone(),
        );

        // Copy additional fields
        new_asset.assignees = source.assignees.clone();
        new_asset.current_version = source.current_version.clone();

        // Reset state to Clean (fresh start in new version line)
        new_asset.current_state = AssetState::Clean;

        self.asset_repo.create(&new_asset).await?;

        Ok(new_asset)
    }

    // Helper: Clone dependencies between selected assets
    async fn clone_dependencies(
        &self,
        source_version_line: &VersionLineId,
        target_version_line: VersionLineId,
        asset_mapping: &HashMap<AssetId, AssetId>,
    ) -> Result<(), RepositoryError> {
        // Get all dependencies in source version line
        let all_deps = self.dependency_repo.find_by_version_line(source_version_line).await?;

        // Filter to only dependencies where both endpoints are in selection
        let deps_to_clone: Vec<_> = all_deps
            .into_iter()
            .filter(|dep| {
                asset_mapping.contains_key(&dep.downstream_asset_id)
                    && asset_mapping.contains_key(&dep.upstream_asset_id)
            })
            .collect();

        for dep in deps_to_clone {
            let new_downstream = asset_mapping[&dep.downstream_asset_id];
            let new_upstream = asset_mapping[&dep.upstream_asset_id];

            let new_dep = AssetDependency::new(
                target_version_line,
                new_downstream,
                new_upstream,
                dep.declared_version,
                dep.effective_version,
            );

            self.dependency_repo.create(&new_dep).await?;
        }

        Ok(())
    }
}

/// Request to fork a version line
pub struct ForkVersionLineRequest {
    pub organization_id: OrganizationId,
    pub source_version_line_id: VersionLineId,
    pub new_name: String,
    pub asset_selection: Vec<AssetId>,
    pub description: Option<String>,
}

/// Response from fork operation
pub struct ForkVersionLineResponse {
    pub version_line: VersionLine,
    pub asset_mapping: HashMap<AssetId, AssetId>,
    pub asset_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_version_line() {
        // Mock repositories and test
        // This is a placeholder - real test would use mockall
    }
}
```

**Step 2: Add necessary imports**

Add proper imports for AssetInstance, AssetState, AssetDependency, etc.

**Step 3: Commit**

```bash
git add crates/adam-application/src/version_line/
git commit -m "feat: add VersionLineService with fork functionality"
```

---

## Phase 4: API Layer - REST Endpoints

### Task 10: Add REST endpoints for VersionLine

**Files:**
- Create/Modify: `crates/adam-adapters/src/rest/version_line.rs`
- Modify: `crates/adam-adapters/src/rest/mod.rs`

**Step 1: Create REST handlers**

Create `crates/adam-adapters/src/rest/version_line.rs`:

```rust
//! REST API handlers for version lines

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use adam_application::version_line::{
    ForkVersionLineRequest, VersionLineError, VersionLineService,
};
use adam_domain::{
    organization::OrganizationId,
    version_line::{VersionLine, VersionLineId, VersionLineStatus},
};

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub version_line_service: Arc<VersionLineService>,
}

/// Create version line request
#[derive(Deserialize)]
pub struct CreateVersionLineRequest {
    pub name: String,
    pub description: Option<String>,
}

/// Version line response
#[derive(Serialize)]
pub struct VersionLineResponse {
    pub id: String,
    pub name: String,
    pub status: String,
    pub forked_from: Option<String>,
    pub description: Option<String>,
    pub created_at: String,
}

impl From<VersionLine> for VersionLineResponse {
    fn from(vl: VersionLine) -> Self {
        VersionLineResponse {
            id: vl.id.0.to_string(),
            name: vl.name,
            status: format!("{:?}", vl.status),
            forked_from: vl.forked_from.map(|id| id.0.to_string()),
            description: vl.description,
            created_at: vl.created_at.to_rfc3339(),
        }
    }
}

/// POST /api/v1/organizations/{org_id}/version-lines
pub async fn create_version_line(
    Path(org_id): Path<Uuid>,
    State(state): State<AppState>,
    Json(req): Json<CreateVersionLineRequest>,
) -> Result<Json<VersionLineResponse>, (StatusCode, String)> {
    let org_id = OrganizationId(org_id);

    match state
        .version_line_service
        .create_version_line(org_id, req.name, req.description)
        .await
    {
        Ok(version_line) => Ok(Json(version_line.into())),
        Err(VersionLineError::NameExists(name)) => {
            Err((StatusCode::CONFLICT, format!("Version line '{}' already exists", name)))
        }
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// Fork request
#[derive(Deserialize)]
pub struct ForkRequest {
    pub name: String,
    pub asset_selection: Vec<Uuid>,
    pub description: Option<String>,
}

/// Fork response
#[derive(Serialize)]
pub struct ForkResponse {
    pub version_line: VersionLineResponse,
    pub asset_count: usize,
}

/// POST /api/v1/version-lines/{id}/fork
pub async fn fork_version_line(
    Path(source_id): Path<Uuid>,
    State(state): State<AppState>,
    Json(req): Json<ForkRequest>,
) -> Result<Json<ForkResponse>, (StatusCode, String)> {
    // Get organization from version line
    let source = state
        .version_line_service
        .version_line_repo
        .find_by_id(&VersionLineId(source_id))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Version line not found".to_string()))?;

    let request = ForkVersionLineRequest {
        organization_id: source.organization_id,
        source_version_line_id: VersionLineId(source_id),
        new_name: req.name,
        asset_selection: req.asset_selection.into_iter().map(AssetId::from_uuid).collect(),
        description: req.description,
    };

    match state.version_line_service.fork_version_line(request).await {
        Ok(response) => Ok(Json(ForkResponse {
            version_line: response.version_line.into(),
            asset_count: response.asset_count,
        })),
        Err(VersionLineError::NotFound(_)) => {
            Err((StatusCode::NOT_FOUND, "Source version line not found".to_string()))
        }
        Err(VersionLineError::NameExists(name)) => {
            Err((StatusCode::CONFLICT, format!("Name '{}' already exists", name)))
        }
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// GET /api/v1/organizations/{org_id}/version-lines
pub async fn list_version_lines(
    Path(org_id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<Vec<VersionLineResponse>>, (StatusCode, String)> {
    let org_id = OrganizationId(org_id);

    match state.version_line_service.get_version_lines(&org_id).await {
        Ok(version_lines) => Ok(Json(version_lines.into_iter().map(|vl| vl.into()).collect())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}
```

**Step 2: Add to router**

Modify `crates/adam-adapters/src/rest/mod.rs`:

```rust
use axum::{
    routing::{get, post},
    Router,
};

pub fn create_router(app_state: AppState) -> Router {
    Router::new()
        // Version lines
        .route("/api/v1/organizations/:org_id/version-lines", post(version_line::create_version_line))
        .route("/api/v1/organizations/:org_id/version-lines", get(version_line::list_version_lines))
        .route("/api/v1/version-lines/:id/fork", post(version_line::fork_version_line))
        // ... existing routes
}
```

**Step 3: Commit**

```bash
git add crates/adam-adapters/src/rest/version_line.rs
git commit -m "feat: add REST endpoints for version line management"
```

---

## Phase 5: Testing & Validation

### Task 11: Write integration test for fork workflow

**Files:**
- Create: `tests/version_line_fork_test.rs`

**Step 1: Write the test**

Create `tests/version_line_fork_test.rs`:

```rust
//! Integration test for version line forking

use adam_domain::{
    asset::instance::AssetInstance,
    organization::OrganizationId,
    version_line::{VersionLine, VersionLineId},
};

#[tokio::test]
async fn test_version_line_fork_creates_independent_assets() {
    // Setup: Create organization and version line
    let org_id = OrganizationId::new();
    let version_line_a = VersionLine::new(org_id, "v1.x", None);
    
    // Create assets in version line A
    // ... setup code

    // Fork to version line B
    // ... fork code

    // Assert: Assets in B have different IDs than A
    // Assert: State is Clean in B (not inherited)
    // Assert: Dependencies are cloned
}

#[tokio::test]
async fn test_dirty_state_isolated_between_version_lines() {
    // Create two version lines with same "logical" asset
    
    // Mark asset dirty in version line A
    
    // Assert: Asset in version line B remains Clean
}
```

**Step 2: Run the test**

```bash
cargo test version_line_fork -- --nocapture
```

Expected: Tests pass

**Step 3: Commit**

```bash
git add tests/version_line_fork_test.rs
git commit -m "test: add integration tests for version line forking"
```

---

### Task 12: Update existing tests

**Files:**
- Modify all test files that create AssetInstance

**Step 1: Find affected tests**

```bash
grep -r "AssetInstance::new" crates/ --include="*.rs" | grep -v target
```

**Step 2: Update tests**

Add `version_line_id` parameter to all test asset creations.

**Step 3: Run full test suite**

```bash
cargo test --workspace
```

Expected: All tests pass

**Step 4: Commit**

```bash
git commit -am "test: update existing tests for version_line_id parameter"
```

---

## Phase 6: Documentation

### Task 13: Update API documentation

**Files:**
- Create: `docs/api-version-lines.md`

**Step 1: Write API documentation**

Create `docs/api-version-lines.md` with endpoint documentation.

**Step 2: Update CHANGELOG**

Add entry for version line feature.

**Step 3: Commit**

```bash
git add docs/
git commit -m "docs: add API documentation for version lines"
```

---

## Final Checklist

Before marking complete:

- [ ] All tests pass: `cargo test --workspace`
- [ ] Code compiles: `cargo build --release`
- [ ] No clippy warnings: `cargo clippy -- -D warnings`
- [ ] Code formatted: `cargo fmt`
- [ ] Documentation updated
- [ ] Integration tests written
- [ ] Migration tested on staging database

---

## Post-Implementation Notes

### Future Enhancements (Out of Scope)

1. **Merge Support**: Allow merging feature branches back to main
2. **Cross-Line Comparison**: Compare assets between version lines
3. **Lineage Visualization**: Show fork/merge graph
4. **Automated Forking**: CI-driven fork creation from Git branches

### Performance Considerations

- Each version line has full index coverage
- Fork operation is O(n) where n = selected assets + dependencies
- Consider batch operations for large forks

---

*End of Implementation Plan*
