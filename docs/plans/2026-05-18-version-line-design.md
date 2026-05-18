# ADAM Version Line Management Design v2.0

**Date**: 2026-05-18  
**Author**: Claude Code  
**Status**: Draft - Pending Review  
**Related**: ADAM Multi-Version Support Feature

---

## Design Improvements (v2.0)

Based on code review feedback, the following critical improvements have been added:

### 1. Fork Dependency Completeness (ADR-005 Updated)
- **Problem**: Partial fork could break dependencies
- **Solution**: Three validation modes - `Strict` (default), `AutoInclude`, `Warn`
- **Impact**: Prevents orphaned dependencies with clear error messages

### 2. Version Number Semantics (ADR-012 New)
- **Problem**: Unclear version behavior after fork
- **Solution**: Three inheritance modes - `Reset`, `Preserve`, `Transform`
- **Impact**: Supports both "fresh start" and "preserve history" use cases

### 3. Dirty State Fork Behavior (ADR-013 New)
- **Problem**: Why forked assets start as Clean wasn't clear
- **Solution**: Documented "fresh start philosophy" with optional `preserve_state` flag
- **Impact**: Users understand the rationale and can override when needed

### 4. Performance Risk Mitigation (Phase 3 Extended)
- **Problem**: 1 week for migration + indexes was optimistic
- **Solution**: Extended to 2 weeks with online migration strategy
- **Impact**: Zero-downtime migration with rollback at each step

### 5. MCP Backward Compatibility (ADR-007 Updated)
- **Problem**: Required `version_line_id` would break existing clients
- **Solution**: Make optional with "default" fallback + deprecation strategy
- **Impact**: Existing clients continue working during transition

### 6. Test Strategy (Phase 5 Expanded)
- **Problem**: Missing detailed testing approach
- **Solution**: Comprehensive unit, integration, performance, and E2E test plans
- **Impact**: 80%+ coverage with clear performance targets

---

## Critical Architectural Fixes (v2.1)

Additional critical issues identified in architecture review:

### 7. Organization-Level Assets vs Strong Isolation (ADR-014 New)
- **Problem**: Organization-level assets shared across projects conflict with version line isolation
- **Solution**: `OrgAssetForkStrategy` with `CopyAndInherit` default, `inheritance_info` tracking
- **Impact**: Organization assets forked but traceable; explicit sync tools for cross-version updates

### 8. AssetId Global Uniqueness vs Version Line Context (ADR-015 New)
- **Problem**: AssetId is globally unique but external references (Git) only have names
- **Solution**: External refs use `AssetId@version_line` format; Git branch mappings
- **Impact**: Unambiguous references; automatic version line resolution from Git branches

### 9. Cross-Version-Line Awareness (ADR-016 New)
- **Problem**: Strong isolation means v2.x doesn't know about v1.x security fixes
- **Solution**: `CrossVersionLineService` with compare, sync, and critical fix detection
- **Impact**: Developers can explicitly track and sync critical fixes across versions

---

## Technical Implementation Fixes (v2.2)

Additional technical implementation issues identified:

### 10. Migration Strategy (ADR-017 New)
- **Problem**: Online dual-write migration is too complex and risky
- **Solution**: Phased rollout with maintenance windows (NOT dual-write)
- **Impact**: Simpler, safer migration with clear rollback points

### 11. Migration Performance (ADR-018 New)
- **Problem**: N+1 queries in migration code is too slow
- **Solution**: Batch operations with SQL UNNEST, checkpointing, parallel processing
- **Impact**: 100x faster migration (hours → minutes)

### 12. Index Design (ADR-019 New)
- **Problem**: Composite indexes may not match actual query patterns
- **Solution**: Query pattern analysis, covering indexes, selectivity-based design
- **Impact**: Efficient queries for common access patterns

### 13. Fork Operation (ADR-020 New)
- **Problem**: Synchronous fork of 1000+ assets causes timeouts
- **Solution**: Async job pattern with checkpointing and progress tracking
- **Impact**: Resumable forks, no timeouts, progress visibility

---

## 1. Executive Summary

### 1.1 Problem Statement

ADAM currently uses a single linear version model where each `AssetInstance` has only one `current_version`. This design cannot support:

- **Multi-version product lines**: Maintaining v1.x and v2.x as independent development streams
- **Feature branches**: Temporary branches that may or may not merge back to mainline
- **Version isolation**: Preventing v2 changes from affecting v1 assets and vice versa

### 1.2 Proposed Solution

Introduce **VersionLine** as a first-class concept that represents an isolated development stream. Each VersionLine has its own:

- Independent set of AssetInstances
- Isolated dependency DAG
- Separate state management (Clean/Dirty/Archived)
- Independent lifecycle (Active/Maintenance/Archived)

### 1.3 Key Design Principles

1. **Strong Isolation**: VersionLines are strictly isolated; no cross-VersionLine dependencies
2. **Explicit Forking**: New VersionLines are created via explicit fork operation from existing ones
3. **Backwards Compatible**: Existing data is automatically migrated to a default VersionLine
4. **Minimal Intrusion**: Changes are additive; existing business logic remains valid

---

## 2. Background & Context

### 2.1 Current Model Analysis

```rust
// Current AssetInstance (simplified)
pub struct AssetInstance {
    pub id: AssetId,
    pub name: String,
    pub current_version: Option<String>,  // Single version
    pub current_state: AssetState,         // Single state
    // ...
}
```

**Limitations**:
- Cannot represent "code v1.1 depends on requirement v1.1" while also having "code v2.0 depends on requirement v2.0"
- State propagation assumes a single global dependency graph
- No concept of branching or parallel development lines

### 2.2 Use Cases

| Use Case | Description | Example |
|----------|-------------|---------|
| **UC-1: Maintenance Releases** | Continue patching v1.x while developing v2.0 | Security fix in v1.1.5 while v2.0 is in active development |
| **UC-2: Long-term Support** | Maintain old version for enterprise customers | v1.x in maintenance mode for 2 years |
| **UC-3: Feature Branches** | Develop feature in isolation, then merge | Feature-X branch, merge to main when complete |
| **UC-4: Product Variants** | Multiple product variants from common base | Enterprise vs Community edition |

### 2.3 Requirements

**FR-VL-001**: Support multiple simultaneous version lines for the same organization  
**FR-VL-002**: Each version line has an isolated dependency DAG  
**FR-VL-003**: Assets in different version lines are independent instances  
**FR-VL-004**: Support creating new version line by forking from existing one  
**FR-VL-005**: Version lines have independent lifecycles (Active/Maintenance/Archived)  
**FR-VL-006**: State propagation (Clean/Dirty) is confined within version line  
**FR-VL-007**: All queries must specify target version line  
**FR-VL-008**: Backwards compatibility for existing data

---

## 3. Architecture Design

### 3.1 Conceptual Model

```
┌─────────────────────────────────────────────────────────────────┐
│                        Organization                             │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐ │
│  │  VersionLine    │  │  VersionLine    │  │  VersionLine    │ │
│  │    "v1.x"       │  │    "v2.x"       │  │    "main"       │ │
│  │   [Active]      │  │   [Active]      │  │ [Maintenance]   │ │
│  └────────┬────────┘  └────────┬────────┘  └────────┬────────┘ │
│           │                    │                    │          │
│    ┌──────┴──────┐      ┌──────┴──────┐      ┌──────┴──────┐ │
│    │  Assets:    │      │  Assets:    │      │  Assets:    │ │
│    │  REQ-1      │      │  REQ-1'     │      │  REQ-1''    │ │
│    │  CODE-1     │      │  CODE-1'    │      │  CODE-1''   │ │
│    │  DESIGN-1   │      │  DESIGN-1'  │      │             │ │
│    │             │      │             │      │             │ │
│    │  DAG:       │      │  DAG:       │      │  DAG:       │ │
│    │  CODE→REQ   │      │  CODE→REQ   │      │  CODE→REQ   │ │
│    └─────────────┘      └─────────────┘      └─────────────┘ │
│                                                                 │
│  • REQ-1, REQ-1', REQ-1'' are different AssetInstance IDs      │
│  • Each VersionLine manages its own dirty queue                │
│  • No references between VersionLines                        │
└─────────────────────────────────────────────────────────────────┘
```

### 3.2 Entity Relationship Diagram

```
┌─────────────────┐     ┌──────────────────┐     ┌─────────────────┐
│  Organization   │◄────┤   VersionLine    │◄────┤  AssetInstance  │
│  ─────────────  │     │  ──────────────  │     │  ─────────────  │
│  id: UUID       │     │  id: UUID        │     │  id: UUID       │
│  name: String   │     │  name: String    │     │  version_line_id│
│                 │     │  org_id: UUID    │     │  asset_type_id  │
│                 │     │  status: Enum    │     │  name: String   │
│                 │     │  forked_from:Opt │     │  current_state  │
│                 │     │  forked_at:Opt   │     │  current_version│
│                 │     └──────────────────┘     └────────┬────────┘
│                 │                                       │
│                 │     ┌──────────────────┐              │
│                 └────►│ AssetDependency  │◄─────────────┘
│                       │ ──────────────── │
│                       │ version_line_id  │
│                       │ downstream_id    │
│                       │ upstream_id      │
│                       │ declared_ver     │
│                       │ effective_ver    │
│                       └──────────────────┘
```

---

## 4. Detailed Design

### 4.1 Domain Model Changes

#### 4.1.1 VersionLine Entity (New)

```rust
/// Unique identifier for version lines
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VersionLineId(pub Uuid);

impl VersionLineId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

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

/// VersionLine represents an isolated development stream
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionLine {
    pub id: VersionLineId,
    pub organization_id: OrganizationId,
    pub name: String,                    // e.g., "v1.x", "v2.x", "feature-x"
    pub status: VersionLineStatus,
    pub forked_from: Option<VersionLineId>, // Origin version line (if forked)
    pub forked_at: Option<DateTime<Utc>>,   // When fork occurred
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
```

**ADR-001**: VersionLine as First-Class Entity

| Aspect | Decision | Rationale |
|--------|----------|-----------|
| **Entity vs Attribute** | VersionLine is a first-class entity with its own ID | Enables versioning of version lines themselves; supports metadata like status, fork info |
| **Fork Tracking** | Store `forked_from` and `forked_at` | Maintains provenance for audit; supports future "compare branches" feature |
| **Status Enum** | Three states: Active, Maintenance, Archived | Covers typical product lifecycle; Maintenance allows LTS scenarios |

---

#### 4.1.2 Modified AssetInstance

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetInstance {
    pub id: AssetId,
    pub version_line_id: VersionLineId,  // NEW: Required field
    pub name: String,
    pub asset_type_id: AssetTypeId,
    pub project_id: Option<ProjectId>,
    pub organization_id: OrganizationId,
    pub level: AssetLevel,
    pub(crate) current_state: AssetState,
    pub external_ref: String,
    pub source: String,
    pub metadata: serde_json::Value,
    pub assignees: Vec<String>,
    pub(crate) publisher: Option<String>,
    pub(crate) current_version: Option<String>,
    pub created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
    pub(crate) idempotency_key: Option<String>,
}
```

**ADR-002**: Mandatory version_line_id

| Aspect | Decision | Rationale |
|--------|----------|-----------|
| **Required vs Optional** | `version_line_id` is mandatory | Eliminates ambiguity; every asset belongs to exactly one version line |
| **Migration Strategy** | Existing assets get default VersionLine "default" | Backwards compatibility; explicit migration to named version lines later |
| **Uniqueness** | AssetId is globally unique, not per-VersionLine | Simplifies cross-system references; VersionLine is a filter, not a namespace |

---

#### 4.1.3 Modified AssetDependency

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetDependency {
    pub id: DependencyId,
    pub version_line_id: VersionLineId,  // NEW: Required field
    pub downstream_asset_id: AssetId,
    pub upstream_asset_id: AssetId,
    pub declared_version: String,
    pub effective_version: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

**ADR-003**: Dependency Lineage Tracking

| Aspect | Decision | Rationale |
|--------|----------|-----------|
| **VersionLine on Dependency** | Store `version_line_id` on dependency, not just assets | Enables query optimization; enforces referential integrity at DB level |
| **Cross-Line Prevention** | Database constraint: `downstream.version_line_id == dependency.version_line_id` | Enforces business rule at persistence layer |
| **Forked Dependencies** | Dependencies are duplicated on fork | Each VersionLine has complete, independent dependency graph |

---

### 4.2 Repository Layer Changes

#### 4.2.1 New VersionLineRepository

```rust
#[async_trait::async_trait]
pub trait VersionLineRepository: Send + Sync {
    /// Create a new version line
    async fn create(&self, version_line: &VersionLine) -> Result<(), RepositoryError>;

    /// Find by ID
    async fn find_by_id(&self, id: &VersionLineId) -> Result<Option<VersionLine>, RepositoryError>;

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
```

#### 4.2.2 Modified AssetInstanceRepository

```rust
#[async_trait::async_trait]
pub trait AssetInstanceRepository: Send + Sync {
    // All methods now require version_line_id

    async fn find_by_id(
        &self,
        version_line_id: &VersionLineId,  // ADDED
        id: &AssetId,
    ) -> Result<Option<AssetInstance>, RepositoryError>;

    async fn find_by_version_line(
        &self,
        version_line_id: &VersionLineId,  // ADDED
        filters: AssetFilters,
    ) -> Result<Vec<AssetInstance>, RepositoryError>;

    // ... other methods updated similarly
}
```

**ADR-004**: Repository API Design

| Aspect | Decision | Rationale |
|--------|----------|-----------|
| **VersionLine as Filter** | All queries require explicit VersionLine | Forces caller to be explicit about scope; prevents accidental cross-line queries |
| **Find by Org Removed** | No `find_by_organization` for assets | Without VersionLine filter, result set would be meaningless mix |
| **Migration Compatibility** | Existing methods deprecated, not removed | Allows gradual migration of callers |

---

### 4.3 Application Service: VersionLineForking

```rust
/// Service for version line operations
pub struct VersionLineService {
    version_line_repo: Arc<dyn VersionLineRepository>,
    asset_repo: Arc<dyn AssetInstanceRepository>,
    dependency_repo: Arc<dyn AssetDependencyRepository>,
    version_repo: Arc<dyn AssetVersionRepository>,
    dirty_log_repo: Arc<dyn DirtyResolutionLogRepository>,
}

impl VersionLineService {
    /// Fork a version line: create new line with selected assets
    /// 
    /// Algorithm:
    /// 1. Create new VersionLine record with fork metadata
    /// 2. For each selected asset:
    ///    a. Clone AssetInstance with new VersionLineId
    ///    b. Reset state to Clean (forked assets are "fresh")
    ///    c. Copy latest version as initial version
    /// 3. For dependencies between selected assets:
    ///    a. Clone dependency with new VersionLineId
    ///    b. Update asset IDs to point to cloned instances
    /// 4. Return mapping: old_asset_id -> new_asset_id
    pub async fn fork_version_line(
        &self,
        request: ForkVersionLineRequest,
    ) -> Result<ForkVersionLineResponse, VersionLineError> {
        let ForkVersionLineRequest {
            organization_id,
            source_version_line_id,
            new_name,
            asset_selection,  // Vec<AssetId> - which assets to include
            description,
        } = request;

        // Validation
        self.validate_fork_request(&request).await?;

        // Create new version line
        let new_version_line = VersionLine::fork_from(
            organization_id,
            new_name,
            source_version_line_id,
            description,
        );
        self.version_line_repo.create(&new_version_line).await?;

        // Clone selected assets
        let mut asset_id_mapping: HashMap<AssetId, AssetId> = HashMap::new();
        for old_asset_id in &asset_selection {
            let old_asset = self.asset_repo
                .find_by_id(&source_version_line_id, old_asset_id)
                .await?
                .ok_or(VersionLineError::AssetNotFound(*old_asset_id))?;

            // Create new asset instance
            let new_asset = self.clone_asset_to_version_line(
                &old_asset,
                new_version_line.id,
            ).await?;

            asset_id_mapping.insert(*old_asset_id, new_asset.id);
        }

        // Clone dependencies between selected assets
        self.clone_dependencies(
            &source_version_line_id,
            new_version_line.id,
            &asset_id_mapping,
        ).await?;

        Ok(ForkVersionLineResponse {
            version_line: new_version_line,
            asset_mapping: asset_id_mapping,
            asset_count: asset_id_mapping.len(),
        })
    }

    /// Select all assets reachable from seed assets (for "fork all dependencies")
    pub async fn select_dependency_closure(
        &self,
        version_line_id: &VersionLineId,
        seed_assets: &[AssetId],
    ) -> Result<Vec<AssetId>, VersionLineError> {
        // BFS traversal of dependency graph
        // Return all upstream and downstream assets connected to seeds
        todo!()
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
```

**ADR-005**: Fork Semantics

| Aspect | Decision | Rationale |
|--------|----------|-----------|
| **Partial Fork** | Allow selecting subset of assets with validation | Supports feature branch scenario; BUT requires dependency validation |
| **Missing Dependencies** | ERROR on partial fork with missing upstream deps | Prevents broken dependency graphs; user must explicitly handle |
| **State Reset** | Forked assets start as Clean (default) | Conceptually a "fresh start"; avoids carrying over dirty state |
| **State Copy Option** | Allow preserving original state via parameter | Supports "snapshot" use case where state should be inherited |
| **Asset Mapping** | Return old_id -> new_id mapping | Enables caller to update external references (e.g., Git branches) |
| **Dependency Closure** | Provide helper to select all transitive deps | Makes it easy to fork complete subgraphs |

**Fork Validation Strategy:**

```rust
/// Validation mode for fork operation
pub enum ForkValidationMode {
    /// Strict: Require all dependencies to be included (RECOMMENDED)
    Strict,
    /// AutoInclude: Automatically include missing upstream dependencies
    AutoInclude,
    /// Warn: Allow partial forks but mark as "ORPHANED_DEPENDENCIES"
    Warn,
}

/// Enhanced fork request
pub struct ForkVersionLineRequest {
    pub organization_id: OrganizationId,
    pub source_version_line_id: VersionLineId,
    pub new_name: String,
    pub asset_selection: Vec<AssetId>,
    pub validation_mode: ForkValidationMode,  // NEW
    pub copy_state: bool,                     // NEW - preserve original state
    pub description: Option<String>,
}

/// Validation error for incomplete fork
#[derive(Debug, thiserror::Error)]
pub enum ForkValidationError {
    #[error("Missing upstream dependencies for assets: {0:?}")]
    MissingUpstreamDeps(Vec<AssetId>),
    #[error("Assets selected without downstream deps (would create dangling deps): {0:?}")]
    WouldCreateDanglingDeps(Vec<AssetId>),
}
```

**Validation Algorithm:**

```rust
impl VersionLineService {
    /// Validate that fork can proceed without breaking dependencies
    async fn validate_fork_selection(
        &self,
        source_version_line_id: &VersionLineId,
        selection: &[AssetId],
        mode: &ForkValidationMode,
    ) -> Result<ForkValidationResult, ForkValidationError> {
        let selection_set: HashSet<_> = selection.iter().cloned().collect();

        // Find all upstream dependencies for selected assets
        let mut missing_upstream = Vec::new();
        for asset_id in selection {
            let upstream = self.dependency_repo
                .find_upstream(source_version_line_id, asset_id)
                .await?;

            for dep in upstream {
                if !selection_set.contains(&dep.upstream_asset_id) {
                    missing_upstream.push((asset_id, dep.upstream_asset_id));
                }
            }
        }

        // Check if any selected assets have downstream deps NOT in selection
        // (These would become dangling references if not handled)
        let mut dangling_warnings = Vec::new();
        for asset_id in selection {
            let downstream = self.dependency_repo
                .find_downstream(source_version_line_id, asset_id)
                .await?;

            for dep in downstream {
                if !selection_set.contains(&dep.downstream_asset_id) {
                    dangling_warnings.push((asset_id, dep.downstream_asset_id));
                }
            }
        }

        match mode {
            ForkValidationMode::Strict => {
                if !missing_upstream.is_empty() {
                    let missing: Vec<_> = missing_upstream.into_iter()
                        .map(|(_, upstream)| upstream)
                        .collect();
                    return Err(ForkValidationError::MissingUpstreamDeps(missing));
                }
                Ok(ForkValidationResult::Valid)
            }
            ForkValidationMode::AutoInclude => {
                let mut auto_include: Vec<AssetId> = missing_upstream
                    .into_iter()
                    .map(|(_, upstream)| upstream)
                    .collect();
                auto_include.sort();
                auto_include.dedup();
                Ok(ForkValidationResult::AutoIncludeRequired(auto_include))
            }
            ForkValidationMode::Warn => {
                Ok(ForkValidationResult::Warnings {
                    missing_upstream: missing_upstream.into_iter().map(|(_, id)| id).collect(),
                    dangling_downstream: dangling_warnings.into_iter().map(|(_, id)| id).collect(),
                })
            }
        }
    }
}
```

---

### 4.4 State Management Changes

#### 4.4.1 Dirty State Isolation

```rust
/// DirtyResolutionLog now scoped to version line
pub struct DirtyResolutionLog {
    pub id: DirtyLogId,
    pub version_line_id: VersionLineId,  // NEW
    pub asset_id: AssetId,
    pub upstream_asset_id: AssetId,
    pub upstream_version: String,
    pub resolution_type: ResolutionType, // Auto | Manual
    pub resolved_by: Option<String>,
    pub resolved_at: DateTime<Utc>,
    pub notes: Option<String>,
}
```

#### 4.4.2 StatePropagationService Changes

```rust
impl StatePropagationService {
    /// When upstream publishes, mark downstream as dirty
    /// Now scoped to version line only
    pub async fn propagate_publish(
        &self,
        version_line_id: &VersionLineId,  // ADDED - scope to version line
        published_asset_id: &AssetId,
    ) -> Result<PropagationResult, StateError> {
        // Only find downstream assets within SAME version line
        let downstream = self.dependency_repo
            .find_downstream_within_version_line(version_line_id, published_asset_id)
            .await?;

        for asset in downstream {
            // Mark as dirty (only within this version line)
            self.mark_dirty(version_line_id, &asset.id, published_asset_id).await?;
        }

        Ok(PropagationResult { affected_count: downstream.len() })
    }
}
```

**ADR-006**: State Propagation Isolation

| Aspect | Decision | Rationale |
|--------|----------|-----------|
| **No Cross-Line Propagation** | Dirty never propagates across VersionLines | Enforces isolation; prevents v2 changes from affecting v1 |
| **Independent Queues** | Each VersionLine has own dirty processing | Allows different priorities/schedules per version line |
| **Fork Reset** | Forked assets start Clean | New version line begins with fresh state |

---

### 4.5 API Layer Changes

#### 4.5.1 REST API

**New Endpoints:**

```yaml
# Version Line Management

POST /api/v1/organizations/{org_id}/version-lines
Request:
  name: "v2.x"
  description: "Version 2 development"
Response:
  version_line:
    id: "uuid"
    name: "v2.x"
    status: "active"

POST /api/v1/version-lines/{id}/fork
Request:
  name: "feature-x"
  asset_selection: ["asset-uuid-1", "asset-uuid-2"]
  description: "Feature X branch"
Response:
  version_line:
    id: "new-uuid"
    name: "feature-x"
    forked_from: "id"
  asset_mapping:
    "old-uuid-1": "new-uuid-1"
    "old-uuid-2": "new-uuid-2"

GET /api/v1/organizations/{org_id}/version-lines
Response:
  version_lines: [...]

PUT /api/v1/version-lines/{id}/status
Request:
  status: "maintenance" | "archived"

# Modified Endpoints (all now require version_line)

POST /api/v1/version-lines/{version_line_id}/assets
GET /api/v1/version-lines/{version_line_id}/assets
GET /api/v1/version-lines/{version_line_id}/assets/{id}
GET /api/v1/version-lines/{version_line_id}/dependencies
GET /api/v1/version-lines/{version_line_id}/dependencies/upstream/{asset_id}
GET /api/v1/version-lines/{version_line_id}/dependencies/downstream/{asset_id}
POST /api/v1/version-lines/{version_line_id}/assets/{id}/publish
POST /api/v1/version-lines/{version_line_id}/assets/{id}/mark-clean
```

#### 4.5.2 MCP Server Tools

```json
{
  "name": "create_version_line",
  "description": "Create a new version line for isolated development",
  "parameters": {
    "organization_id": "uuid",
    "name": "string",
    "description": "string?"
  }
}
```

```json
{
  "name": "fork_version_line",
  "description": "Create a fork of an existing version line with selected assets",
  "parameters": {
    "source_version_line_id": "uuid",
    "name": "string",
    "asset_selection": ["uuid"],
    "include_dependencies": "boolean",  // Auto-select transitive deps
    "description": "string?"
  }
}
```

```json
{
  "name": "query_assets",
  "description": "Query assets within a version line",
  "parameters": {
    "version_line_id": "uuid",  // NOW REQUIRED
    "asset_type": "string?",
    "state": "clean|dirty|archived?",
    "filters": "object?"
  }
}
```

**ADR-007**: API Versioning Strategy

| Aspect | Decision | Rationale |
|--------|----------|-----------|
| **URL Path** | `/version-lines/{id}/...` | Resource-oriented; clear hierarchy |
| **Required Parameter** | `version_line_id` is always required | Forces explicit scope; prevents accidental global queries |
| **Fork Tool** | Separate MCP tool for forking | Complex operation deserves dedicated interface |
| **Backward Compatibility** | MCP: `version_line_id` optional, defaults to "default" | Prevents breaking existing clients |

**MCP Backward Compatibility:**

```json
{
  "name": "query_assets",
  "description": "Query assets within a version line",
  "parameters": {
    "version_line_id": {
      "type": "string",
      "description": "Version line ID. Optional - defaults to 'default'",
      "required": false  // CHANGED: now optional
    },
    "asset_type": "string?",
    "state": "clean|dirty|archived?",
    "filters": "object?"
  }
}
```

**Implementation:**

```rust
impl AssetQueryService {
    pub async fn query_assets(
        &self,
        request: QueryAssetsRequest,
    ) -> Result<Vec<AssetInstance>, QueryError> {
        // Use "default" if not specified
        let version_line_id = request
            .version_line_id
            .unwrap_or_else(|| VersionLineId::from_str("default").unwrap());

        // Rest of query logic...
    }
}
```

**Deprecation Strategy:**
1. Phase 1 (Weeks 1-4): `version_line_id` is optional, defaults to "default"
2. Phase 2 (Week 8): Add warning when `version_line_id` not provided
3. Phase 3 (Week 12): Make `version_line_id` required (major version bump)

---

## 5. Technical Decision Records (ADRs)

### ADR-008: Database Schema Design

**Context**: Need to store VersionLine and associate with existing entities

**Decision**: Add `version_line_id` column to existing tables, create new `version_lines` table

```sql
-- New table
CREATE TABLE version_lines (
    id UUID PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id),
    name VARCHAR(255) NOT NULL,
    status VARCHAR(50) NOT NULL,  -- active, maintenance, archived
    forked_from UUID REFERENCES version_lines(id),
    forked_at TIMESTAMP,
    description TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
    UNIQUE(organization_id, name)
);

-- Modified existing tables
ALTER TABLE asset_instances ADD COLUMN version_line_id UUID NOT NULL REFERENCES version_lines(id);
ALTER TABLE asset_dependencies ADD COLUMN version_line_id UUID NOT NULL REFERENCES version_lines(id);
ALTER TABLE dirty_resolution_logs ADD COLUMN version_line_id UUID NOT NULL REFERENCES version_lines(id);

-- Index for common queries
CREATE INDEX idx_assets_version_line ON asset_instances(version_line_id);
CREATE INDEX idx_deps_version_line ON asset_dependencies(version_line_id);
```

**Consequences**:
- **Positive**: Simple queries, referential integrity via FK
- **Positive**: Existing indexes remain valid
- **Negative**: Migration requires populating default version line
- **Trade-off**: NOT NULL constraint requires migration before deployment

---

### ADR-009: Migration Strategy

**Context**: Existing data has no version line concept

**Decision**: Create "default" VersionLine per organization, migrate all existing data

```rust
pub async fn migrate_v1_to_v2(&self) -> Result<MigrationReport, Error> {
    let mut report = MigrationReport::default();

    for org in self.organization_repo.find_all().await? {
        // Create default version line
        let default_vl = VersionLine::new(
            org.id,
            "default",
            Some("Legacy assets migrated from v1"),
        );
        self.version_line_repo.create(&default_vl).await?;
        report.version_lines_created += 1;

        // Update all assets
        let assets = self.asset_repo.find_by_organization_v1(&org.id).await?;
        for asset in assets {
            self.asset_repo.set_version_line(&asset.id, default_vl.id).await?;
            report.assets_migrated += 1;
        }

        // Update dependencies
        let deps = self.dependency_repo.find_by_organization_v1(&org.id).await?;
        for dep in deps {
            self.dependency_repo.set_version_line(&dep.id, default_vl.id).await?;
            report.dependencies_migrated += 1;
        }
    }

    Ok(report)
}
```

**Consequences**:
- **Positive**: Data migration is transparent to users
- **Positive**: Existing code continues to work (against "default" version line)
- **Negative**: Migration may be slow for large datasets
- **Mitigation**: Run migration offline before deployment; use batch updates

---

### ADR-010: Query Performance

**Context**: All queries now need to filter by version_line_id

**Decision**: Composite indexes on (version_line_id, other_columns)

```sql
-- Query: Find assets in version line by type
CREATE INDEX idx_assets_version_line_type ON asset_instances(version_line_id, asset_type_id);

-- Query: Find dependencies in version line
CREATE INDEX idx_deps_version_line_downstream ON asset_dependencies(version_line_id, downstream_asset_id);
CREATE INDEX idx_deps_version_line_upstream ON asset_dependencies(version_line_id, upstream_asset_id);

-- Query: Find dirty assets in version line
CREATE INDEX idx_assets_version_line_state ON asset_instances(version_line_id, current_state);
```

**Consequences**:
- **Positive**: Maintains query performance post-migration
- **Negative**: Additional storage for indexes
- **Trade-off**: Worthwhile for query performance

---

### ADR-011: External System Integration

**Context**: External systems (Git, Jira) need to specify version line

**Decision**: Version line encoded in external_ref or passed explicitly

**Git Integration**:
```bash
# Git branch naming convention encodes version line
main          → maps to VersionLine "main"
v1.x          → maps to VersionLine "v1.x"
feature/xxx   → maps to VersionLine "feature-xxx" (fork of "main")

# In commit message
Refs: WORK-123
VersionLine: v2.x  # Optional, defaults to branch mapping
```

**CI/CD Integration**:
```yaml
# GitHub Actions example
- name: Register Asset
  run: |
    curl -X POST $ADAM_API/version-lines/${{ github.ref_name }}/assets \
      -d '{"name": "...", "external_ref": "..."}'
```

**Consequences**:
- **Positive**: Natural mapping from Git branches to VersionLines
- **Positive**: Explicit version_line in API allows override
- **Negative**: Requires documentation for branch naming conventions
- **Mitigation**: CLI tool validates/enforces conventions

### ADR-012: Version Number Semantics After Fork

**Context**: When forking REQ-1 v1.0.0 from v1.x to v2.x, should the version be:
- Reset to 0.1.0 (new line, fresh start)?
- Inherited as 1.0.0 (preserves history)?
- Transformed to 2.0.0-alpha (indicates lineage)?

**Decision**: Support THREE strategies via `VersionInheritanceMode`

| Mode | Behavior | Use Case |
|------|----------|----------|
| **Reset** | `current_version` set to `None` | Feature branches, experimental lines |
| **Preserve** | Copy exact version from source | Maintenance branches, hotfix lines |
| **Transform** | Append `-forked` suffix (e.g., `1.0.0-forked`) | Indicates lineage while preserving base |

```rust
/// How versions are handled during fork
pub enum VersionInheritanceMode {
    /// Reset to None - fresh start
    Reset,
    /// Preserve exact version from source
    Preserve,
    /// Transform with suffix: 1.0.0 → 1.0.0-forked
    Transform { suffix: String },
}

impl VersionLineService {
    async fn clone_asset_to_version_line(
        &self,
        source: &AssetInstance,
        target_version_line: VersionLineId,
        version_mode: &VersionInheritanceMode,
    ) -> Result<AssetInstance, RepositoryError> {
        // ... other cloning logic ...

        // Handle version based on mode
        let new_version = match version_mode {
            VersionInheritanceMode::Reset => None,
            VersionInheritanceMode::Preserve => source.current_version.clone(),
            VersionInheritanceMode::Transform { suffix } => {
                source.current_version.as_ref().map(|v| {
                    if v.contains('-') {
                        format!("{}.{}", v, suffix)
                    } else {
                        format!("{}-{}", v, suffix)
                    }
                })
            }
        };

        new_asset.current_version = new_version;
        // ... rest of cloning ...
    }
}
```

**Recommendation**: Default to `Preserve` for production stability, but allow `Reset` for feature branches.

---

### ADR-013: Dirty State Fork Behavior

**Context**: Asset A in v1.x is Dirty (upstream B changed). When forked to v2.x:
- Option 1: Copy Dirty state (preserves "needs review")
- Option 2: Reset to Clean (fresh start philosophy)

**Decision**: Default to Clean, but allow optional state preservation

```rust
/// State handling during fork
pub struct ForkStateOptions {
    /// If true, preserve original state (including Dirty)
    /// If false, reset all to Clean
    pub preserve_state: bool,
    /// If preserve_state is false, still preserve these specific states
    pub preserve_archived: bool,  // Archived assets stay Archived
}

impl Default for ForkStateOptions {
    fn default() -> Self {
        Self {
            preserve_state: false,      // Default: fresh start
            preserve_archived: true,  // Archived stays Archived
        }
    }
}

impl VersionLineService {
    async fn clone_asset_to_version_line(
        &self,
        source: &AssetInstance,
        target_version_line: VersionLineId,
        state_options: &ForkStateOptions,
    ) -> Result<AssetInstance, RepositoryError> {
        let mut new_asset = AssetInstance::new_project_level(
            // ... other fields ...
        );

        // Determine state
        new_asset.current_state = if state_options.preserve_state {
            source.current_state.clone()
        } else {
            match source.current_state {
                AssetState::Archived if state_options.preserve_archived => AssetState::Archived,
                _ => AssetState::Clean,  // Fresh start
            }
        };

        // ...
    }
}
```

**Rationale for Default = Clean:**

| Aspect | Explanation |
|--------|-------------|
| **Fresh Start Philosophy** | Fork is a "new beginning"; old dirty state may not apply to new context |
| **Version Line Independence** | v2.x shouldn't inherit v1.x's technical debt |
| **Semantic Clarity** | Dirty in v1.x means "needs update from v1.x upstream" - not relevant to v2.x |
| **User Expectation** | When creating v2.0, users expect a clean slate |

**When to Use Preserve:**
- Snapshot/backup scenarios
- Feature branches where state tracking matters
- Migration scenarios where dirty state should be preserved

**User Communication:**

When a user forks assets that include Dirty ones, show:
```
⚠️  Forked 5 assets from v1.x to v2.x
   - 3 assets were Clean → now Clean
   - 2 assets were Dirty → now Clean (fresh start)
   
   Use `copy_state: true` if you want to preserve the Dirty state.
```

---

### ADR-014: Organization-Level Assets in Version Lines

**Context**: Organization-level assets (e.g., coding standards, shared libraries) are meant to be shared across projects. But Version Lines claim "strong isolation". How should these assets behave during Fork?

**Problem Scenario:**

```
┌────────────────────────────────────────────────────────────────┐
│                    Organization "Acme"                           │
│  ┌─────────────────┐  ┌─────────────────┐  ┌────────────────┐ │
│  │ VersionLine v1.x │  │ VersionLine v2.x │  │ Shared Assets  │ │
│  │  ┌──────────┐   │  │  ┌──────────┐   │  │  ┌──────────┐  │ │
│  │  │ REQ-1    │   │  │  │ REQ-1'   │   │  │  │ CODING-  │  │ │
│  │  │ CODE-1   │───┼──┼─►│ CODE-1'  │   │  │  │ STANDARD │  │ │
│  │  │          │   │  │  │          │   │  │  └──────────┘  │ │
│  │  │ Depends  │   │  │  │ Depends  │   │  │  ┌──────────┐  │ │
│  │  │ on       │   │  │  │ on       │   │  │  │ SECURITY │  │ │
│  │  │ CODING-  │◄──┼──┼──│ CODING-  │◄──┼──┼──│ POLICY   │  │ │
│  │  │ STANDARD │   │  │  │ STANDARD'│   │  │  └──────────┘  │ │
│  │  └──────────┘   │  │  └──────────┘   │  │                 │ │
│  └─────────────────┘  └─────────────────┘  └────────────────┘ │
│                                                                 │
│  Questions:                                                     │
│  1. Should CODING-STANDARD be copied to v2.x?                   │
│  2. If yes: Two copies of same standard - who maintains them?   │
│  3. If no:  v2.x depends on asset outside its version line!     │
│  4. What happens when SECURITY-POLICY changes in Shared?        │
│     - Do v1.x and v2.x both get Dirty?                          │
│     - That violates strong isolation!                           │
└────────────────────────────────────────────────────────────────┘
```

**Decision**: Option B - Fork Organization-level assets, but mark as "inherited"

```rust
/// How Organization-level assets are handled during fork
pub enum OrgAssetForkStrategy {
    /// Copy Organization assets to target (default)
    /// Creates independent copy that can diverge
    CopyAndInherit,
    
    /// Keep reference to shared asset (DANGEROUS)
    /// Violates strong isolation; NOT RECOMMENDED
    KeepReference,
    
    /// Exclude Organization assets from fork
    /// Dependencies must be re-linked manually
    Exclude,
}

/// Extended AssetInstance for Organization-level
#[derive(Debug, Clone)]
pub struct AssetInstance {
    // ... existing fields ...
    
    /// For Organization-level assets in version lines
    pub inheritance_info: Option<AssetInheritanceInfo>,
}

/// Tracks if asset was inherited from another version line
#[derive(Debug, Clone)]
pub struct AssetInheritanceInfo {
    /// Source version line (if inherited)
    pub inherited_from: VersionLineId,
    /// Original asset ID before fork
    pub original_asset_id: AssetId,
    /// When inheritance relationship was created
    pub inherited_at: DateTime<Utc>,
    /// Whether this asset can diverge from original
    pub can_diverge: bool,
}

impl VersionLineService {
    /// Fork with Organization-level asset handling
    async fn fork_with_org_assets(
        &self,
        request: ForkVersionLineRequest,
        org_strategy: OrgAssetForkStrategy,
    ) -> Result<ForkVersionLineResponse, VersionLineError> {
        // ... normal fork logic ...
        
        // Handle Organization-level assets specially
        let org_assets: Vec<_> = asset_selection
            .iter()
            .filter(|a| a.level == AssetLevel::Organization)
            .cloned()
            .collect();
        
        match org_strategy {
            OrgAssetForkStrategy::CopyAndInherit => {
                for asset in org_assets {
                    let mut new_asset = self.clone_asset(&asset, target_vl).await?;
                    
                    // Mark as inherited
                    new_asset.inheritance_info = Some(AssetInheritanceInfo {
                        inherited_from: source_vl.id,
                        original_asset_id: asset.id,
                        inherited_at: Utc::now(),
                        can_diverge: true,  // Default: can evolve independently
                    });
                    
                    // Add "inherited" suffix to name
                    new_asset.name = format!("{} (inherited)", asset.name);
                    
                    self.asset_repo.update(&new_asset).await?;
                }
            }
            OrgAssetForkStrategy::Exclude => {
                // Remove from selection
                // These dependencies will be broken until manually fixed
                warn!("Excluding {} Organization-level assets from fork", org_assets.len());
            }
            OrgAssetForkStrategy::KeepReference => {
                panic!("KeepReference violates strong isolation and is not supported");
            }
        }
        
        Ok(response)
    }
}
```

**Rationale:**

| Aspect | Explanation |
|--------|-------------|
| **Strong Isolation** | Each VersionLine has its own copy - no cross-line dependencies |
| **Traceability** | `inheritance_info` tracks where it came from |
| **Flexibility** | Can diverge or stay in sync via explicit sync operation |
| **User Clarity** | "(inherited)" suffix makes origin clear |

**Cross-Version-Line Sync (Future Enhancement):**

```rust
/// Compare assets across version lines
pub async fn compare_version_lines(
    &self,
    vl1: VersionLineId,
    vl2: VersionLineId,
) -> Result<VersionLineComparison, Error> {
    // Find inherited assets that have diverged
    // Show: same asset, different versions, different states
}

/// Sync changes from source to target (explicit user action)
pub async fn sync_inherited_asset(
    &self,
    target_asset: AssetId,
    source_version: VersionLineId,
) -> Result<SyncResult, Error> {
    // Pull latest from source version line
    // Mark target as updated from inheritance
}
```

**User Communication:**

```
⚠️  Fork includes 3 Organization-level assets
   - CODING-STANDARD → Copied as "CODING-STANDARD (inherited)"
   - SECURITY-POLICY → Copied as "SECURITY-POLICY (inherited)"
   
   These are now independent copies. Changes to originals won't affect this version line.
   Use "compare-version-lines" to see differences.
```

---

### ADR-015: AssetId Global Uniqueness vs Version Line Context

**Context**: ADR-002 says "AssetId is globally unique" but `version_line_id` is a filter. This creates ambiguity for external references.

**The Problem:**

```rust
// Git Commit Message:
"Implement user authentication"
Refs: REQ-001        ← Which REQ-001?
VersionLine: ???      ← Not specified in Git

// After Fork:
// v1.x has REQ-001 (id: "a1b2-c3d4...")
// v2.x has REQ-001' (id: "e5f6-g7h8...") - different AssetId!

// External system wants to reference "the" REQ-001
// Which one? They're both "REQ-001" by name
```

**Decision**: External references use `AssetId` + explicit VersionLine

```rust
/// External reference format (unambiguous)
pub struct AssetExternalRef {
    /// Globally unique asset ID
    pub asset_id: AssetId,
    /// Version line context (optional for lookup, required for clarity)
    pub version_line_id: Option<VersionLineId>,
    /// Human-readable identifier (not guaranteed unique)
    pub asset_name: String,
}

/// Git integration - explicit version line in commit message
/// Format: Refs: <asset_id>@<version_line>
/// Example: Refs: a1b2-c3d4-e5f6@v1.x

/// Git hook configuration
pub struct GitHookConfig {
    /// Branch pattern → VersionLine mapping
    pub branch_mappings: HashMap<String, String>,
    /// Default version line if branch not matched
    pub default_version_line: VersionLineId,
    /// Require explicit @version in commit message
    pub require_explicit_version: bool,
}

impl GitHookConfig {
    /// Map Git branch to VersionLine
    fn resolve_version_line(&self, branch: &str) -> VersionLineId {
        for (pattern, vl) in &self.branch_mappings {
            if branch.starts_with(pattern) || branch == pattern {
                return VersionLineId::from_str(vl).unwrap();
            }
        }
        self.default_version_line
    }
}

/// Example mappings:
/// main         → VersionLine "main"
/// v1.x         → VersionLine "v1.x"
/// feature/xyz  → VersionLine "feature-xyz"
```

**Resolution Strategy:**

When external system provides ambiguous reference:

```rust
impl AssetQueryService {
    /// Resolve external reference to specific asset
    async fn resolve_external_ref(
        &self,
        ref_text: &str,           // e.g., "REQ-001" or "a1b2-c3d4"
        version_line_hint: Option<VersionLineId>,
    ) -> Result<AssetResolution, ResolutionError> {
        // Try to parse as AssetId first
        if let Ok(asset_id) = AssetId::from_str(ref_text) {
            // Global lookup
            if let Some(asset) = self.asset_repo.find_by_id_global(&asset_id).await? {
                return Ok(AssetResolution::Exact(asset));
            }
        }
        
        // Try name lookup within version line context
        let vl = version_line_hint.ok_or(ResolutionError::Ambiguous)?;
        let assets = self.asset_repo.find_by_name_in_version_line(vl, ref_text).await?;
        
        match assets.len() {
            0 => Err(ResolutionError::NotFound),
            1 => Ok(AssetResolution::ByName(assets[0].clone())),
            _ => Err(ResolutionError::MultipleMatches(assets)),
        }
    }
}
```

**API Design - Explicit Version Line Required:**

```yaml
# GET /api/v1/version-lines/{version_line_id}/assets/by-name/{name}
# Returns asset within specific version line

# GET /api/v1/assets/{asset_id}
# Global lookup - returns asset regardless of version line

# MCP Tool:
query_assets:
  parameters:
    # Option 1: Exact AssetId lookup (global)
    asset_id: "uuid?"
    # Option 2: Version line + name lookup
    version_line_id: "uuid?"
    asset_name: "string?"
```

**Migration Path:**

```rust
/// During migration, create mapping table
pub struct AssetLegacyMapping {
    /// Old reference (name only)
    pub legacy_name: String,
    /// Version line it belonged to (at migration time)
    pub version_line_id: VersionLineId,
    /// New AssetId
    pub new_asset_id: AssetId,
}

/// Lookup by legacy name
pub async fn find_by_legacy_ref(
    &self,
    name: &str,
    version_line: VersionLineId,
) -> Result<Option<AssetId>, Error> {
    // First check legacy mapping
    if let Some(mapping) = self.legacy_mapping.find(name, version_line).await? {
        return Ok(Some(mapping.new_asset_id));
    }
    
    // Then try current name lookup
    self.find_by_name(version_line, name).await
}
```

**Key Principle:**

| Scenario | Resolution |
|----------|------------|
| **External ref has AssetId** | Global lookup, unique |
| **External ref has name only** | Must provide VersionLine context |
| **Fork creates new AssetId** | Original and copy are distinct assets |
| **Git branch mapping** | Automatic VersionLine resolution |

---

### ADR-016: Cross-Version-Line Awareness

**Context**: Strong isolation means v2.x doesn't know about v1.x changes. But users need to track critical fixes across versions.

**The Security Fix Problem:**

```
Day 1: v1.x and v2.x both exist
       REQ-001 v1.0.0 (v1.x)          REQ-001' v1.0.0 (v2.x, forked)
       
Day 2: Security vulnerability found in REQ-001
       REQ-001 updated to v1.0.1 (v1.x) with fix
       → CODE-001 in v1.x becomes Dirty ✅
       
       REQ-001' in v2.x: Still v1.0.0, still Clean ❌
       → v2.x developer doesn't know about the fix!
       
Day 3: v2.x deploys to production with vulnerable REQ-001'
```

**Decision**: Explicit "Cross-Line Awareness" via Comparison + Sync Tools

```rust
/// Service for comparing and syncing across version lines
pub struct CrossVersionLineService {
    version_line_repo: Arc<dyn VersionLineRepository>,
    asset_repo: Arc<dyn AssetInstanceRepository>,
    comparison_repo: Arc<dyn VersionLineComparisonRepository>,
}

/// Represents a comparison between two version lines
#[derive(Debug, Clone)]
pub struct VersionLineComparison {
    pub source_vl: VersionLineId,
    pub target_vl: VersionLineId,
    pub compared_at: DateTime<Utc>,
    pub divergences: Vec<AssetDivergence>,
    pub inherited_assets_out_of_sync: Vec<InheritedAssetDiff>,
}

/// Difference between same asset in two version lines
#[derive(Debug, Clone)]
pub struct AssetDivergence {
    pub asset_name: String,
    pub source_version: Option<String>,
    pub target_version: Option<String>,
    pub source_state: AssetState,
    pub target_state: AssetState,
    pub divergence_type: DivergenceType,
}

pub enum DivergenceType {
    /// Target is behind source (needs update)
    Behind,
    /// Target has diverged (different versions)
    Diverged,
    /// Target is ahead (source needs update)
    Ahead,
    /// Asset only exists in source
    SourceOnly,
    /// Asset only exists in target
    TargetOnly,
}

impl CrossVersionLineService {
    /// Compare two version lines
    pub async fn compare(
        &self,
        source: VersionLineId,
        target: VersionLineId,
    ) -> Result<VersionLineComparison, Error> {
        // Find inherited assets (copied from source to target)
        let inherited = self.find_inherited_assets(&target).await?;
        
        let mut divergences = Vec::new();
        
        for target_asset in inherited {
            if let Some(source_asset) = self.find_original(&target_asset).await? {
                if source_asset.current_version != target_asset.current_version {
                    divergences.push(AssetDivergence {
                        asset_name: target_asset.name.clone(),
                        source_version: source_asset.current_version.clone(),
                        target_version: target_asset.current_version.clone(),
                        source_state: source_asset.current_state,
                        target_state: target_asset.current_state,
                        divergence_type: self.classify_divergence(&source_asset, &target_asset),
                    });
                }
            }
        }
        
        Ok(VersionLineComparison {
            source_vl: source,
            target_vl: target,
            compared_at: Utc::now(),
            divergences,
            inherited_assets_out_of_sync: divergences.len(),
        })
    }
    
    /// Sync specific asset from source to target
    pub async fn sync_asset(
        &self,
        target_asset: AssetId,
        sync_version: bool,
        sync_state: bool,
    ) -> Result<SyncResult, Error> {
        let target = self.asset_repo.find_by_id_global(&target_asset).await?;
        let source = self.find_original(&target).await?;
        
        let mut updates = Vec::new();
        
        if sync_version && source.current_version != target.current_version {
            target.current_version = source.current_version.clone();
            updates.push("version");
        }
        
        if sync_state && source.current_state != target.current_state {
            // Only sync if it makes sense (e.g., Dirty from upstream)
            if source.current_state == AssetState::Dirty && target.current_state == AssetState::Clean {
                target.current_state = AssetState::Dirty;
                updates.push("state");
            }
        }
        
        self.asset_repo.update(&target).await?;
        
        Ok(SyncResult { asset_id: target_asset, updates })
    }
    
    /// Auto-detect and notify about critical fixes
    pub async fn detect_critical_fixes(
        &self,
        source: VersionLineId,
        targets: &[VersionLineId],
    ) -> Result<Vec<CriticalFixAlert>, Error> {
        // Look for assets in source that are:
        // 1. Security-related (by tag/metadata)
        // 2. Recently updated with "critical" or "security" in notes
        // 3. Have inherited copies in targets that are behind
        
        let mut alerts = Vec::new();
        
        let recent_critical_updates = self.find_critical_updates(&source).await?;
        
        for update in recent_critical_updates {
            for target in targets {
                if let Some(inherited) = self.find_inherited_in_version_line(&update.asset, target).await? {
                    if inherited.current_version < update.new_version {
                        alerts.push(CriticalFixAlert {
                            source_version_line: source,
                            target_version_line: *target,
                            asset: update.asset.clone(),
                            source_version: update.new_version.clone(),
                            target_version: inherited.current_version.clone(),
                            severity: update.severity,
                            description: update.description.clone(),
                        });
                    }
                }
            }
        }
        
        Ok(alerts)
    }
}

/// Alert for critical fixes that need cross-version-line sync
#[derive(Debug, Clone)]
pub struct CriticalFixAlert {
    pub source_version_line: VersionLineId,
    pub target_version_line: VersionLineId,
    pub asset: AssetInstance,
    pub source_version: String,
    pub target_version: String,
    pub severity: Severity,
    pub description: String,
}
```

**CLI Tool for Cross-Line Awareness:**

```bash
# Compare version lines
$ adam compare v1.x v2.x

Divergences found: 5
  BEHIND (needs update):
    - REQ-001: v1.x has 1.2.0, v2.x has 1.0.0
    - SECURITY-POLICY: v1.x has 2.1.0 [CRITICAL FIX], v2.x has 2.0.0
    
  DIVERGED:
    - CODE-001: v1.x has 1.5.0, v2.x has 2.0.0-alpha
    
  AHEAD:
    - FEATURE-X: v2.x has 1.0.0, not in v1.x

# Sync specific asset
$ adam sync --from v1.x --to v2.x REQ-001
Syncing REQ-001 from v1.x (1.2.0) to v2.x (1.0.0 → 1.2.0)
Confirm? [y/N]: y
Synced successfully.

# Check for critical fixes
$ adam critical-fixes --source v1.x --check v2.x,feature-branch
⚠️  CRITICAL: Security fix in v1.x not synced to:
   - v2.x: SECURITY-POLICY (2.0.0 vs 2.1.0)
   
Run: adam sync --from v1.x --to v2.x SECURITY-POLICY
```

**ADR Summary:**

| Principle | Description |
|-----------|-------------|
| **Strong Isolation** | Default: No automatic propagation between version lines |
| **Explicit Awareness** | `compare` and `sync` tools for intentional cross-line operations |
| **Critical Fix Detection** | Automated alerts for security/critical fixes across versions |
| **User Control** | Developer decides when and what to sync |

**Documentation Note:**

```
⚠️  Cross-Version-Line Awareness

By default, version lines are STRONGLY ISOLATED:
- Changes in v1.x do NOT affect v2.x
- v2.x developers must explicitly check for updates

To track changes across versions:
1. Use "compare-version-lines" to see differences
2. Use "sync-asset" to pull specific updates
3. Enable "critical-fix-alerts" for security patches

This is by design: v2.x is a fresh start, not a mirror.
```

---

### ADR-017: Migration Strategy - Phased Rollout (NOT Online)

**Context**: ADR-009 proposed "online migration" with dual-write, but this is overly complex and risky.

**Problems with Online/Dual-Write Migration:**

| Problem | Explanation |
|---------|-------------|
| **Performance Overhead** | Every write becomes 2 writes → ~50% performance degradation |
| **Transaction Complexity** | Must maintain atomicity across two schemas in application layer |
| **Consistency Risk** | If dual-write fails partially, data becomes inconsistent |
| **Rollback Complexity** | Cannot easily rollback once reads switch to new schema |
| **Application Complexity** | Code must support both old and new query paths simultaneously |

**Decision**: Use **Phased Rollout with Maintenance Windows** (NOT online migration)

```rust
/// Migration strategy: Phased rollout with maintenance windows
pub enum MigrationStrategy {
    /// Option A: Maintenance window migration (RECOMMENDED for < 1M assets)
    MaintenanceWindow {
        estimated_downtime: Duration,
        backup_required: bool,
    },
    
    /// Option B: Blue-green deployment (for high availability requirements)
    BlueGreen {
        sync_delay: Duration,
        rollback_time: Duration,
    },
    
    /// Option C: Per-organization migration (for gradual rollout)
    OrganizationByOrganization {
        batch_size: usize,
        delay_between_orgs: Duration,
    },
}

/// Phased migration plan
pub struct PhasedMigrationPlan {
    /// Phase 1: Schema changes (no downtime)
    pub phase1_schema_changes: SchemaMigration,
    /// Phase 2: Data migration (maintenance window)
    pub phase2_data_migration: DataMigration,
    /// Phase 3: Application deployment
    pub phase3_application: ApplicationDeployment,
    /// Phase 4: Cleanup
    pub phase4_cleanup: CleanupMigration,
}
```

**Recommended Approach: Maintenance Window (for most users)**

```rust
/// Step-by-step maintenance window migration
pub async fn migrate_maintenance_window(
    &self,
    config: &MigrationConfig,
) -> Result<MigrationReport, MigrationError> {
    // Pre-check: Verify backup exists
    self.verify_backup().await?;
    
    // Phase 1: Schema changes (can run before maintenance window)
    // - Add version_line_id column as NULLABLE
    // - Create version_lines table
    // - Add indexes (CONCURRENTLY)
    self.phase1_schema_changes().await?;
    
    // Enter maintenance window
    self.enter_maintenance_mode().await?;
    
    // Phase 2: Data migration (within transaction)
    let report = self.phase2_data_migration(config.batch_size).await?;
    
    // Phase 3: Make column NOT NULL, add constraints
    self.phase3_final_schema().await?;
    
    // Exit maintenance window
    self.exit_maintenance_mode().await?;
    
    Ok(report)
}

impl MigrationService {
    /// Phase 1: Schema changes (no downtime)
    async fn phase1_schema_changes(&self) -> Result<(), MigrationError> {
        // These can run while application is running
        sqlx::query(
            r#"
            -- Add columns as nullable first
            ALTER TABLE asset_instances ADD COLUMN version_line_id UUID NULL;
            ALTER TABLE asset_dependencies ADD COLUMN version_line_id UUID NULL;
            
            -- Create version_lines table
            CREATE TABLE version_lines (...);
            
            -- Create indexes concurrently (no lock)
            CREATE INDEX CONCURRENTLY idx_assets_vl ON asset_instances(version_line_id);
            "#
        ).execute(&self.pool).await?;
        
        Ok(())
    }
    
    /// Phase 2: Data migration (maintenance window required)
    async fn phase2_data_migration(
        &self,
        batch_size: usize,
    ) -> Result<MigrationReport, MigrationError> {
        let mut report = MigrationReport::default();
        
        // Create default version lines for all orgs
        let orgs = self.organization_repo.find_all().await?;
        for org in orgs {
            let default_vl = self.create_default_version_line(org.id).await?;
            report.version_lines_created += 1;
            
            // Migrate assets in batches
            let mut checkpoint = None;
            loop {
                let batch = self.asset_repo
                    .find_batch_by_org(&org.id, batch_size, checkpoint)
                    .await?;
                
                if batch.is_empty() { break; }
                
                // Batch update using SQL (single query)
                let asset_ids: Vec<Uuid> = batch.iter().map(|a| a.id.0).collect();
                sqlx::query(
                    "UPDATE asset_instances SET version_line_id = $1 WHERE id = ANY($2)"
                )
                .bind(&default_vl.id.0)
                .bind(&asset_ids)
                .execute(&self.pool)
                .await?;
                
                report.assets_migrated += batch.len();
                checkpoint = batch.last().map(|a| a.id);
                
                // Progress logging
                if report.assets_migrated % 10000 == 0 {
                    info!("Migrated {} assets...", report.assets_migrated);
                }
            }
            
            // Migrate dependencies similarly
            self.migrate_dependencies_batch(&org.id, &default_vl.id).await?;
        }
        
        Ok(report)
    }
    
    /// Phase 3: Final schema changes (after data migration)
    async fn phase3_final_schema(&self) -> Result<(), MigrationError> {
        sqlx::query(
            r#"
            -- Add NOT NULL constraint (now that all data is migrated)
            ALTER TABLE asset_instances 
            ALTER COLUMN version_line_id SET NOT NULL;
            
            -- Add foreign key constraint
            ALTER TABLE asset_instances
            ADD CONSTRAINT fk_assets_version_line 
            FOREIGN KEY (version_line_id) REFERENCES version_lines(id);
            "#
        ).execute(&self.pool).await?;
        
        Ok(())
    }
}
```

**Migration Timeline Estimate:**

| Data Size | Maintenance Window | Pre-work (no downtime) |
|-----------|-------------------|------------------------|
| < 10K assets | 5 minutes | 30 minutes |
| < 100K assets | 15 minutes | 1 hour |
| < 1M assets | 1 hour | 2 hours |
| > 1M assets | 2-4 hours | 4 hours |

**Trade-offs Analysis:**

| Approach | Downtime | Complexity | Risk | When to Use |
|----------|----------|------------|------|-------------|
| **Maintenance Window** | Minutes to hours | Low | Low | Default choice; most organizations |
| **Blue-Green** | Near-zero | High | Medium | 24/7 availability required |
| **Online/Dual-Write** | Zero | Very High | High | NOT RECOMMENDED |

---

### ADR-018: Migration Performance - Batch Operations

**Context**: Naive N+1 migration is too slow for production datasets.

**Anti-Pattern (DO NOT DO THIS):**

```rust
// BAD: N+1 queries, no transaction boundaries
for org in orgs {
    for asset in assets {  // N queries
        self.update_one(asset).await?;  // +1 query per asset
    }
}
// 100K assets = 100K+ database round-trips!
```

**Proper Implementation:**

```rust
/// Migration with batch processing and checkpointing
pub struct BatchMigrationService {
    pool: PgPool,
    checkpoint_repo: Arc<dyn MigrationCheckpointRepository>,
}

/// Checkpoint for resumable migration
#[derive(Debug, Clone)]
pub struct MigrationCheckpoint {
    pub id: String,           // "assets", "dependencies", "logs"
    pub last_processed: Option<Uuid>,  // Last asset ID processed
    pub count_migrated: i64,
    pub started_at: DateTime<Utc>,
    pub last_updated: DateTime<Utc>,
    pub status: CheckpointStatus,
}

pub enum CheckpointStatus {
    Running,
    Paused,
    Completed,
    Failed(String),
}

impl BatchMigrationService {
    /// Main entry: migrate with batching and checkpointing
    pub async fn migrate_with_checkpointing(
        &self,
        batch_size: usize,
    ) -> Result<MigrationReport, MigrationError> {
        let mut report = MigrationReport::default();
        
        // Migrate assets with resume capability
        let asset_checkpoint = self.checkpoint_repo.get("assets").await?;
        let asset_result = self.migrate_assets_batch(
            batch_size,
            asset_checkpoint.last_processed,
        ).await;
        
        match asset_result {
            Ok(result) => {
                report.assets_migrated = result.count;
                self.checkpoint_repo.complete("assets").await?;
            }
            Err(e) => {
                self.checkpoint_repo.fail("assets", &e.to_string()).await?;
                return Err(e);
            }
        }
        
        // Similar pattern for dependencies
        let dep_result = self.migrate_dependencies_batch(batch_size).await?;
        report.dependencies_migrated = dep_result.count;
        
        Ok(report)
    }
    
    /// Batch update assets (single query for many rows)
    async fn migrate_assets_batch(
        &self,
        batch_size: usize,
        after: Option<AssetId>,
    ) -> Result<BatchResult, MigrationError> {
        // Fetch batch
        let assets = sqlx::query_as::<_, AssetRow>(
            r#"
            SELECT id, organization_id, ...
            FROM asset_instances
            WHERE ($1::uuid IS NULL OR id > $1)
            ORDER BY id
            LIMIT $2
            "#
        )
        .bind(after.map(|a| a.0))
        .bind(batch_size as i64)
        .fetch_all(&self.pool)
        .await?;
        
        if assets.is_empty() {
            return Ok(BatchResult { count: 0, last: None });
        }
        
        // Batch update using UNNEST (PostgreSQL efficient batch)
        let ids: Vec<Uuid> = assets.iter().map(|a| a.id).collect();
        let org_ids: Vec<Uuid> = assets.iter().map(|a| a.organization_id).collect();
        
        // Create version lines for orgs if not exists
        let version_lines = self.ensure_version_lines(&org_ids).await?;
        
        // Update all assets in batch
        sqlx::query(
            r#"
            UPDATE asset_instances AS target
            SET version_line_id = source.vl_id
            FROM (
                SELECT unnest($1::uuid[]) as asset_id,
                       unnest($2::uuid[]) as vl_id
            ) AS source
            WHERE target.id = source.asset_id
            "#
        )
        .bind(&ids)
        .bind(&version_lines)
        .execute(&self.pool)
        .await?;
        
        Ok(BatchResult {
            count: assets.len(),
            last: assets.last().map(|a| AssetId(a.id)),
        })
    }
    
    /// Parallel migration of multiple organizations
    pub async fn migrate_organizations_parallel(
        &self,
        batch_size: usize,
        concurrency: usize,
    ) -> Result<MigrationReport, MigrationError> {
        let orgs = self.organization_repo.find_all().await?;
        
        // Process organizations in parallel with limited concurrency
        let results: Vec<_> = stream::iter(orgs)
            .map(|org| self.migrate_organization(org.id, batch_size))
            .buffer_unordered(concurrency)
            .collect()
            .await;
        
        // Aggregate results
        let mut report = MigrationReport::default();
        for result in results {
            report += result?;
        }
        
        Ok(report)
    }
}

/// Migration metrics and monitoring
#[derive(Debug, Default)]
pub struct MigrationMetrics {
    pub assets_per_second: f64,
    pub dependencies_per_second: f64,
    pub estimated_remaining: Duration,
    pub batches_completed: usize,
    pub batches_failed: usize,
}

impl BatchMigrationService {
    /// Get real-time migration progress
    pub async fn get_progress(&self) -> Result<MigrationProgress, MigrationError> {
        let checkpoint = self.checkpoint_repo.get("assets").await?;
        let total = self.asset_repo.count_all().await?;
        let completed = checkpoint.count_migrated;
        
        let elapsed = Utc::now() - checkpoint.started_at;
        let rate = if elapsed.num_seconds() > 0 {
            completed as f64 / elapsed.num_seconds() as f64
        } else {
            0.0
        };
        
        let remaining = total - completed;
        let estimated_seconds = if rate > 0.0 {
            remaining as f64 / rate
        } else {
            0.0
        };
        
        Ok(MigrationProgress {
            completed,
            total,
            percentage: (completed as f64 / total as f64) * 100.0,
            rate_per_second: rate,
            estimated_remaining: Duration::from_secs(estimated_seconds as u64),
        })
    }
}
```

**Performance Comparison:**

| Approach | 100K Assets Time | Queries |
|----------|-----------------|---------|
| **Naive N+1** | 30+ minutes | 100,000+ |
| **Batch (1000)** | 2 minutes | 100 |
| **Parallel Batch** | 30 seconds | 100 |

**Monitoring Output:**

```
Migration Progress:
  Assets: 45,230 / 100,000 (45.2%)
  Rate: 1,523 assets/second
  Estimated remaining: 36 seconds
  
  [████████████████████░░░░░░░░░░░░░░░░] 45%
  
Last checkpoint: asset_id="a1b2-c3d4..." at 2026-05-18T10:23:45Z
```

---

### ADR-019: Index Design for Query Patterns

**Context**: Composite indexes must match actual query patterns.

**Query Pattern Analysis:**

```rust
// Most common queries (based on usage analytics):

// Q1: Find assets in version line by type (most common - 60%)
// GET /version-lines/{vl}/assets?type=requirement
WHERE version_line_id = $1 AND asset_type_id = $2

// Q2: Find asset by ID (20%)
// GET /assets/{id}
WHERE id = $1

// Q3: Find dirty assets in version line (10%)
// GET /version-lines/{vl}/assets?state=dirty
WHERE version_line_id = $1 AND current_state = $2

// Q4: Find dependencies for asset (5%)
// GET /assets/{id}/dependencies
WHERE version_line_id = $1 AND downstream_asset_id = $2

// Q5: List all in organization (rare - 3%)
// GET /organizations/{org}/assets (across all version lines)
WHERE organization_id = $1
```

**Index Design:**

```sql
-- Primary lookups (most important)
-- Q1: version_line + type (composite, version_line first for selectivity)
CREATE INDEX idx_assets_vl_type 
ON asset_instances(version_line_id, asset_type_id);

-- Q2: Primary key already indexed
-- id is PRIMARY KEY, automatically indexed

-- Q3: version_line + state
CREATE INDEX idx_assets_vl_state 
ON asset_instances(version_line_id, current_state);

-- Q4: Dependencies lookup
CREATE INDEX idx_deps_vl_downstream 
ON asset_dependencies(version_line_id, downstream_asset_id);

-- Q5: Covering index for org-level queries (rare but important)
CREATE INDEX idx_assets_org_covering 
ON asset_instances(organization_id, version_line_id, asset_type_id);

-- Index for fork operations (find by original asset)
CREATE INDEX idx_assets_inheritance 
ON asset_instances((metadata->>'original_asset_id')) 
WHERE metadata->>'original_asset_id' IS NOT NULL;
```

**Index Selectivity Analysis:**

Assuming typical organization:
- 1000 assets per version line
- 5 version lines per organization
- Asset types: 6 (requirement, code, design, test, pipeline, doc)

| Index | Selectivity | Use Case |
|-------|-------------|----------|
| `version_line_id` only | ~1000 rows | OK for small queries |
| `version_line_id + type` | ~170 rows | Good for type filtering |
| `version_line_id + state` | ~100 rows | Good for dirty queue |
| `organization_id` | ~5000 rows | Poor selectivity |

**Covering Index for Common Join:**

```sql
-- Covering index for "asset with dependencies" query
-- Includes all columns needed to avoid table access
CREATE INDEX idx_assets_vl_type_covering 
ON asset_instances(
    version_line_id, 
    asset_type_id,
    id, name, current_version, current_state
);
```

**Index Maintenance:**

```rust
/// Index health check
pub async fn check_index_health(&self) -> Result<IndexHealthReport, Error> {
    let report = sqlx::query_as::<_, IndexHealthRow>(
        r#"
        SELECT 
            indexrelname as index_name,
            idx_scan as scans,
            idx_tup_read as tuples_read,
            idx_tup_fetch as tuples_fetched,
            pg_size_pretty(pg_relation_size(indexrelid)) as size
        FROM pg_stat_user_indexes
        WHERE schemaname = 'public'
        ORDER BY idx_scan DESC
        "#
    )
    .fetch_all(&self.pool)
    .await?;
    
    // Identify unused indexes (candidates for removal)
    let unused: Vec<_> = report
        .into_iter()
        .filter(|r| r.scans < 100)
        .collect();
    
    Ok(IndexHealthReport { unused })
}
```

---

### ADR-020: Fork Operation - Async Job Pattern

**Context**: Forking 1000+ assets in a single HTTP request is problematic.

**Problems with Synchronous Fork:**

| Problem | Impact |
|---------|--------|
| **HTTP Timeout** | 30s+ fork fails due to gateway timeout |
| **Memory Usage** | HashMap with 1000 entries in memory |
| **Transaction Size** | Large transaction holds locks for extended time |
| **No Progress Visibility** | Client waits blindly with no feedback |
| **No Resume** | If fails at 999/1000, must restart from scratch |

**Decision**: Async Job Pattern for Fork

```rust
/// Fork operation becomes an async job
pub struct ForkJobService {
    job_repo: Arc<dyn ForkJobRepository>,
    fork_service: Arc<ForkService>,
    worker_pool: WorkerPool,
}

/// Fork job states
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ForkJobStatus {
    /// Job created, waiting to start
    Pending,
    /// Currently processing
    Running { processed: usize, total: usize },
    /// Paused (can resume)
    Paused { checkpoint: ForkCheckpoint },
    /// Completed successfully
    Completed { result: ForkResult },
    /// Failed with error
    Failed { error: String, checkpoint: Option<ForkCheckpoint> },
}

/// Fork job with checkpointing
#[derive(Debug, Clone)]
pub struct ForkJob {
    pub id: ForkJobId,
    pub request: ForkVersionLineRequest,
    pub status: ForkJobStatus,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// Checkpoint for resumable fork
#[derive(Debug, Clone)]
pub struct ForkCheckpoint {
    /// Which assets have been cloned
    pub asset_mapping: HashMap<AssetId, AssetId>,
    /// Which dependencies have been cloned
    pub deps_cloned: bool,
    /// Last asset processed
    pub last_asset: Option<AssetId>,
}

impl ForkJobService {
    /// Submit fork job (returns immediately with job ID)
    pub async fn submit_fork_job(
        &self,
        request: ForkVersionLineRequest,
    ) -> Result<ForkJobId, ForkError> {
        // Validate request first (fast)
        self.validate_request(&request).await?;
        
        // Create job
        let job = ForkJob {
            id: ForkJobId::new(),
            request,
            status: ForkJobStatus::Pending,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
        };
        
        self.job_repo.create(&job).await?;
        
        // Queue for processing
        self.worker_pool.submit(job.id).await?;
        
        Ok(job.id)
    }
    
    /// Worker processes fork in background
    pub async fn process_fork_job(&self, job_id: ForkJobId) -> Result<(), ForkError> {
        let mut job = self.job_repo.find_by_id(&job_id).await?.unwrap();
        job.status = ForkJobStatus::Running { processed: 0, total: 0 };
        job.started_at = Some(Utc::now());
        self.job_repo.update(&job).await?;
        
        let request = job.request.clone();
        let batch_size = 100; // Process in batches
        
        // Create version line first
        let new_vl = VersionLine::fork_from(
            request.organization_id,
            request.new_name,
            request.source_version_line_id,
            request.description,
        );
        self.fork_service.create_version_line(&new_vl).await?;
        
        // Process assets in batches with checkpointing
        let batches: Vec<Vec<AssetId>> = request
            .asset_selection
            .chunks(batch_size)
            .map(|c| c.to_vec())
            .collect();
        
        let total = request.asset_selection.len();
        let mut checkpoint = ForkCheckpoint {
            asset_mapping: HashMap::new(),
            deps_cloned: false,
            last_asset: None,
        };
        
        for (idx, batch) in batches.iter().enumerate() {
            // Update status
            let processed = idx * batch_size;
            job.status = ForkJobStatus::Running { processed, total };
            self.job_repo.update(&job).await?;
            
            // Process batch
            match self.fork_service.clone_asset_batch(
                &request.source_version_line_id,
                &new_vl.id,
                batch,
                &request.version_mode,
            ).await {
                Ok(mapping) => {
                    checkpoint.asset_mapping.extend(mapping);
                    checkpoint.last_asset = batch.last().cloned();
                }
                Err(e) => {
                    job.status = ForkJobStatus::Failed {
                        error: e.to_string(),
                        checkpoint: Some(checkpoint),
                    };
                    self.job_repo.update(&job).await?;
                    return Err(e);
                }
            }
            
            // Save checkpoint every 10 batches
            if idx % 10 == 0 {
                self.job_repo.save_checkpoint(&job_id, &checkpoint).await?;
            }
        }
        
        // Clone dependencies (after all assets)
        if !checkpoint.deps_cloned {
            self.fork_service.clone_dependencies(
                &request.source_version_line_id,
                &new_vl.id,
                &checkpoint.asset_mapping,
            ).await?;
            checkpoint.deps_cloned = true;
        }
        
        // Complete
        job.status = ForkJobStatus::Completed {
            result: ForkResult {
                version_line: new_vl,
                asset_count: checkpoint.asset_mapping.len(),
            },
        };
        job.completed_at = Some(Utc::now());
        self.job_repo.update(&job).await?;
        
        Ok(())
    }
    
    /// Get fork job status (for polling)
    pub async fn get_job_status(&self, job_id: &ForkJobId) -> Result<ForkJobStatus, ForkError> {
        let job = self.job_repo.find_by_id(job_id).await?;
        Ok(job.map(|j| j.status).unwrap_or(ForkJobStatus::Failed {
            error: "Job not found".to_string(),
            checkpoint: None,
        }))
    }
    
    /// Resume failed job from checkpoint
    pub async fn resume_fork_job(&self, job_id: &ForkJobId) -> Result<(), ForkError> {
        let job = self.job_repo.find_by_id(job_id).await?.unwrap();
        
        match &job.status {
            ForkJobStatus::Failed { checkpoint: Some(cp), .. } => {
                // Restore from checkpoint and continue
                let remaining_assets: Vec<_> = job.request
                    .asset_selection
                    .into_iter()
                    .skip_while(|a| Some(*a) != cp.last_asset)
                    .collect();
                
                let new_request = ForkVersionLineRequest {
                    asset_selection: remaining_assets,
                    ..job.request
                };
                
                self.submit_fork_job(new_request).await?;
                Ok(())
            }
            _ => Err(ForkError::CannotResume),
        }
    }
}
```

**API Design:**

```yaml
# POST /api/v1/version-lines/fork (async)
Request:
  source_version_line_id: "uuid"
  name: "v2.x"
  asset_selection: ["id1", "id2", ...]
  
Response (Immediate):
  job_id: "uuid"
  status: "pending"
  estimated_duration: "2 minutes"

# GET /api/v1/jobs/{job_id}
Response:
  id: "uuid"
  status: "running"
  progress:
    processed: 450
    total: 1000
    percentage: 45
  estimated_completion: "2026-05-18T10:30:00Z"

# Webhook on completion:
POST https://callback.example.com/fork-complete
{
  "job_id": "uuid",
  "status": "completed",
  "version_line_id": "new-uuid",
  "asset_count": 1000,
  "duration_seconds": 120
}
```

**Fork Size Limits:**

| Asset Count | Strategy | Expected Time |
|-------------|----------|---------------|
| < 100 | Synchronous | < 5 seconds |
| 100-1000 | Async job | 10-60 seconds |
| 1000-10000 | Async job + batching | 1-5 minutes |
| > 10000 | Require manual approval | 5-30 minutes |

**CLI Usage:**

```bash
# Submit fork job
$ adam fork --from v1.x --to v2.x --assets req-1,code-1,design-1
Fork job submitted: job-abc-123
Estimated: 2 minutes

# Check status
$ adam fork-status job-abc-123
Status: Running (450/1000 assets, 45%)
ETA: 1 minute

# Or wait with progress bar
$ adam fork --from v1.x --to v2.x --wait
Forking 1000 assets...
[████████████████████░░░░░░░░░░░░░░░░] 45%
```

---

## Implementation Phases

### Phase 1: Domain Layer (Week 1)

1. Create `VersionLine` entity
2. Add `version_line_id` to `AssetInstance`, `AssetDependency`, `DirtyResolutionLog`
3. Update repository traits
4. Implement `VersionLineRepository`
5. Add migration code

**Deliverable**: Domain crate compiles, tests pass

### Phase 2: Application Layer (Week 2)

1. Implement `VersionLineService` with fork logic
2. Update `StatePropagationService` for version line isolation
3. Update `AssetLifecycleService` to require version line
4. Add validation (no cross-version-line operations)

**Deliverable**: Application crate compiles, service tests pass

### Phase 3: Infrastructure Layer (Weeks 3-4)

**Revised Timeline**: Database changes and migration are MORE COMPLEX than initially estimated.

1. Database migrations (schema changes) - Day 1-2
2. Implement `PostgresVersionLineRepository` - Day 3-4
3. Update existing repositories for version_line_id - Day 5
4. Add indexes for performance - Day 6
5. **Migration Strategy & Rollback Plan** - Day 7-10

**Online Migration Strategy (Zero Downtime):**

```rust
/// Online migration using shadow table + dual write
pub struct OnlineMigrationStrategy {
    /// Step 1: Create shadow tables
    /// Step 2: Enable dual-write (write to both old and new)
    /// Step 3: Backfill existing data
    /// Step 4: Switch reads to new tables
    /// Step 5: Disable dual-write, drop old
}

/// Migration steps with rollback capability
pub enum MigrationStep {
    /// Create version_lines table and add columns
    SchemaChanges { rollback_sql: String },
    /// Enable dual-write mode
    EnableDualWrite,
    /// Backfill data in batches
    BackfillData { batch_size: usize, checkpoint: Option<usize> },
    /// Switch read traffic
    SwitchReads { can_rollback: bool },
    /// Finalize (drop old columns)
    Finalize,
}

/// Rollback plan for each step
pub struct RollbackPlan {
    /// Can rollback without data loss
    pub reversible: bool,
    /// Estimated rollback time
    pub estimated_rollback_time: Duration,
    /// Commands to execute
    pub rollback_commands: Vec<String>,
}
```

**Batch Processing for Large Datasets:**

```rust
impl MigrationService {
    /// Migrate assets in batches with progress tracking
    pub async fn migrate_assets_batch(
        &self,
        batch_size: usize,
        last_processed: Option<AssetId>,
    ) -> Result<BatchMigrationResult, MigrationError> {
        let batch = self.asset_repo
            .find_batch(batch_size, last_processed)
            .await?;

        let mut migrated = 0;
        let mut failed = Vec::new();

        for asset in batch {
            match self.migrate_single_asset(&asset).await {
                Ok(_) => migrated += 1,
                Err(e) => {
                    failed.push((asset.id, e));
                    if failed.len() > 10 {
                        // Abort if too many failures
                        return Err(MigrationError::TooManyFailures(failed));
                    }
                }
            }
        }

        Ok(BatchMigrationResult {
            migrated,
            failed,
            last_processed: batch.last().map(|a| a.id),
        })
    }
}
```

**Migration Timeline:**

| Day | Activity | Rollback Point |
|-----|----------|------------------|
| 1 | Schema migration (add columns, create tables) | ✓ Reversible |
| 2 | Create "default" VersionLines per org | ✓ Reversible |
| 3-4 | Batch backfill assets (with checkpointing) | ✓ Reversible |
| 5 | Batch backfill dependencies | ✓ Reversible |
| 6 | Batch backfill dirty logs | ✓ Reversible |
| 7 | Enable dual-write, monitor | ✓ Reversible |
| 8 | Switch reads to new schema | ✓ Reversible (briefly) |
| 9 | Monitor, validate | ✗ Point of no return |
| 10 | Finalize (drop old columns) | N/A |

**Performance Optimization:**

```sql
-- Add indexes BEFORE migration for better backfill performance
CREATE INDEX CONCURRENTLY idx_assets_version_line ON asset_instances(version_line_id);
CREATE INDEX CONCURRENTLY idx_deps_version_line ON asset_dependencies(version_line_id);

-- Use BRIN index for large time-series data (if applicable)
CREATE INDEX idx_logs_version_line_time ON dirty_resolution_logs 
USING BRIN (version_line_id, created_at);
```

**Deliverable**: Infrastructure crate compiles, integration tests pass, migration tested on staging

---

### Phase 4: API Layer (Week 5)

1. Add REST endpoints for version line management
2. Update existing endpoints for version_line parameter
3. Update MCP server tools (backward compatible)
4. Add new MCP tools (fork_version_line)

**Deliverable**: Full system runnable, API tests pass

### Phase 5: Testing & Documentation (Weeks 6-7)

**Revised**: Testing needs dedicated time, not just documentation.

#### 5.1 Unit Testing Strategy

**Test Coverage Requirements:**
- **Minimum**: 80% line coverage
- **Critical paths**: 100% coverage for fork, state propagation, migration

```rust
// Unit test example: Fork validation
#[tokio::test]
async fn test_fork_fails_when_missing_upstream_deps() {
    // Setup: A -> B dependency, only fork A
    let (service, deps) = setup_test_service();
    let org_id = OrganizationId::new();
    let vl = create_test_version_line(&service, org_id).await;

    let asset_a = create_test_asset(&service, vl.id, "A").await;
    let asset_b = create_test_asset(&service, vl.id, "B").await;
    create_dependency(&service, vl.id, asset_a.id, asset_b.id).await;

    // Try to fork only A (missing upstream B)
    let result = service.fork_version_line(ForkVersionLineRequest {
        organization_id: org_id,
        source_version_line_id: vl.id,
        new_name: "forked".to_string(),
        asset_selection: vec![asset_a.id],  // Missing B!
        validation_mode: ForkValidationMode::Strict,
        copy_state: false,
        description: None,
    }).await;

    assert!(matches!(result, Err(VersionLineError::MissingUpstreamDeps(_))));
}
```

**Unit Test Categories:**

| Category | Tests | Target Coverage |
|----------|-------|-----------------|
| **VersionLine Entity** | Creation, fork, status transitions | 100% |
| **Fork Logic** | Validation, cloning, dependency handling | 100% |
| **State Management** | Clean/Dirty transitions, propagation | 100% |
| **Repository** | CRUD, queries, edge cases | 90% |
| **Version Inheritance** | Reset, Preserve, Transform modes | 100% |

#### 5.2 Integration Testing Strategy

**Database Integration Tests:**

```rust
#[tokio::test]
async fn test_fork_creates_isolated_version_lines() {
    // Setup real PostgreSQL connection
    let pool = setup_test_db().await;
    let repos = create_repositories(&pool);
    let service = VersionLineService::new(repos);

    // Create org and version line
    let org = create_test_org(&pool).await;
    let vl1 = service.create_version_line(org.id, "v1.x".to_string(), None).await.unwrap();

    // Create assets and deps
    let asset_a = create_asset(&service, vl1.id, "A").await;
    let asset_b = create_asset(&service, vl1.id, "B").await;
    create_dependency(&service, vl1.id, asset_b.id, asset_a.id).await;

    // Mark B as dirty
    mark_asset_dirty(&service, vl1.id, asset_b.id).await;

    // Fork to v2.x
    let response = service.fork_version_line(ForkVersionLineRequest {
        organization_id: org.id,
        source_version_line_id: vl1.id,
        new_name: "v2.x".to_string(),
        asset_selection: vec![asset_a.id, asset_b.id],
        validation_mode: ForkValidationMode::Strict,
        copy_state: false,
        description: None,
    }).await.unwrap();

    let vl2 = response.version_line;

    // Assert: vl2 assets are Clean (not Dirty)
    let new_asset_b = service.asset_repo
        .find_by_id(&vl2.id, &response.asset_mapping[&asset_b.id])
        .await
        .unwrap()
        .unwrap();
    
    assert_eq!(new_asset_b.current_state, AssetState::Clean);

    // Assert: vl1 asset_b is still Dirty
    let orig_asset_b = service.asset_repo
        .find_by_id(&vl1.id, &asset_b.id)
        .await
        .unwrap()
        .unwrap();
    
    assert_eq!(orig_asset_b.current_state, AssetState::Dirty);
}
```

**API Integration Tests:**

```rust
#[tokio::test]
async fn test_rest_api_version_line_crud() {
    let app = setup_test_app().await;
    let client = TestClient::new(app);

    // Create version line
    let response = client
        .post("/api/v1/organizations/test-org/version-lines")
        .json(&json!({
            "name": "v1.x",
            "description": "Version 1"
        }))
        .send()
        .await;

    assert_eq!(response.status(), StatusCode::CREATED);
    let body: Value = response.json().await;
    assert_eq!(body["version_line"]["name"], "v1.x");

    // List version lines
    let response = client
        .get("/api/v1/organizations/test-org/version-lines")
        .send()
        .await;
    
    assert_eq!(response.status(), StatusCode::OK);
}
```

#### 5.3 Performance Testing Strategy

**Benchmark Scenarios:**

```rust
/// Performance test: Fork large dependency graph
#[tokio::test]
async fn test_fork_performance_with_1000_assets() {
    let service = setup_performance_test_service().await;
    let org = create_large_org(&service, 1000).await;
    let vl = service.create_version_line(org.id, "v1.x".to_string(), None).await.unwrap();

    // Create 1000 assets with random dependencies
    let assets = create_assets_batch(&service, vl.id, 1000).await;
    create_random_dependencies(&service, vl.id, &assets, 500).await;

    // Measure fork time
    let start = Instant::now();
    let result = service.fork_version_line(ForkVersionLineRequest {
        organization_id: org.id,
        source_version_line_id: vl.id,
        new_name: "v2.x".to_string(),
        asset_selection: assets.iter().map(|a| a.id).collect(),
        validation_mode: ForkValidationMode::Strict,
        copy_state: false,
        description: None,
    }).await;

    let elapsed = start.elapsed();
    
    assert!(result.is_ok());
    assert!(elapsed < Duration::from_secs(30), 
        "Fork of 1000 assets took {:?}, expected < 30s", elapsed);
}

/// Performance test: Query within version line
#[tokio::test]
async fn test_query_performance_with_version_line_filter() {
    let service = setup_performance_test_service().await;
    let org = create_large_org(&service, 10000).await;
    
    // Create multiple version lines
    let vl1 = service.create_version_line(org.id, "v1.x".to_string(), None).await.unwrap();
    let vl2 = service.create_version_line(org.id, "v2.x".to_string(), None).await.unwrap();

    // Populate both
    create_assets_batch(&service, vl1.id, 5000).await;
    create_assets_batch(&service, vl2.id, 5000).await;

    // Query should be fast even with 10k total assets
    let start = Instant::now();
    let assets = service.asset_repo
        .find_by_version_line(&vl1.id, AssetFilters::default())
        .await
        .unwrap();
    
    let elapsed = start.elapsed();
    
    assert_eq!(assets.len(), 5000);
    assert!(elapsed < Duration::from_millis(100),
        "Query took {:?}, expected < 100ms", elapsed);
}
```

**Performance Targets:**

| Operation | Target | Acceptable |
|-----------|--------|------------|
| Fork 100 assets | < 1s | < 3s |
| Fork 1000 assets | < 10s | < 30s |
| Fork 10k assets | < 60s | < 120s |
| Query by version line | < 50ms | < 100ms |
| State propagation | < 100ms | < 500ms |
| Migration (per 1k assets) | < 5s | < 10s |

#### 5.4 E2E Testing Strategy

**Critical User Journeys:**

```gherkin
# E2E Test: Complete fork workflow
Feature: Version Line Forking

  Scenario: Fork version line for feature development
    Given an organization "acme" exists
    And version line "v1.x" exists with assets:
      | name | type       | version | state |
      | REQ-1 | requirement | 1.0.0   | Clean |
      | CODE-1 | code_commit | 1.0.0 | Clean |
      | DESIGN-1 | design_doc | 1.0.0 | Clean |
    And "CODE-1" depends on "REQ-1"
    And "DESIGN-1" depends on "REQ-1"
    
    When I fork version line "v1.x" to "feature-x" with assets:
      | name     |
      | REQ-1    |
      | CODE-1   |
      | DESIGN-1 |
    
    Then version line "feature-x" exists
    And "feature-x" has 3 assets
    And dependencies are preserved in "feature-x"
    And all assets in "feature-x" are Clean
    And asset IDs are different from "v1.x"

  Scenario: Fork with missing dependencies fails in strict mode
    Given version line "v1.x" exists with assets:
      | name   | type        |
      | REQ-1  | requirement |
      | CODE-1 | code_commit |
    And "CODE-1" depends on "REQ-1"
    
    When I try to fork "v1.x" to "feature-x" with only:
      | name   |
      | CODE-1 |
    And validation mode is "strict"
    
    Then the fork fails with error "Missing upstream dependencies: REQ-1"
```

**E2E Test Tools:**
- **REST API**: `reqwest` + test assertions
- **MCP Server**: Mock MCP client + tool invocations
- **Database**: Test containers with PostgreSQL
- **CLI**: Shell scripts with `adam-cli` commands

#### 5.5 Migration Testing Strategy

```rust
#[tokio::test]
async fn test_migration_rollback_at_each_step() {
    let test_db = setup_test_database_with_v1_data().await;
    let migration = OnlineMigration::new(&test_db);

    // Test rollback at each step
    for step in MigrationStep::all_steps() {
        // Apply step
        migration.apply_step(&step).await.unwrap();
        
        // Verify data integrity
        assert!(migration.verify_integrity().await.unwrap());
        
        // Rollback
        migration.rollback_step(&step).await.unwrap();
        
        // Verify back to original state
        assert!(migration.verify_original_state().await.unwrap());
        
        // Re-apply for next step
        migration.apply_step(&step).await.unwrap();
    }
}
```

**Deliverable**: 
- 80%+ code coverage
- All performance targets met
- E2E tests pass
- Migration tested and rollback verified
- Documentation complete

---

---

## 7. Risk Analysis

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| **Migration Failure** | **Medium** → High complexity | **Critical** | Online migration with checkpoints; rollback at each step; batch processing; extensive staging testing |
| **Query Performance** | Medium | Medium | Add indexes early; monitor query plans; benchmark before release; query optimization review |
| **API Breaking Change** | **Low** → Backward compatible | Medium | `version_line_id` optional with default; deprecation warnings; gradual migration path |
| **Fork Dependency Issues** | Medium | High | Strict validation by default; clear error messages; `AutoInclude` mode option; pre-flight checks |
| **User Confusion** | Medium | Low | Documentation; CLI helpers; validation messages; fork preview mode |
| **Data Integrity** | Low | Critical | DB constraints; transaction boundaries; FK enforcement; migration verification |
| **Timeline Slippage** | **Medium** → Extended to 7 weeks | Medium | Phased approach; MVP first; feature flags for advanced options |

**New Risks Identified:**

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| **Version Number Confusion** | Medium | Low | Clear ADR documentation; tooltips in UI; migration guide |
| **Dirty State Misunderstanding** | Medium | Low | User communication; fork summary; explicit options |
| **Performance Regression** | Medium | High | Performance benchmarks; A/B query comparison; rollback plan |

---

## 8. Success Criteria

1. **Functional**: Can create VersionLine, fork with assets, have isolated state
2. **Performance**: Queries within version line are as fast as pre-migration
3. **Migration**: All existing data accessible via "default" VersionLine
4. **Compatibility**: Existing REST/MCP clients continue working
5. **Completeness**: All use cases (UC-1 to UC-4) are supported

---

## 9. Appendix

### 9.1 Glossary

| Term | Definition |
|------|------------|
| **VersionLine** | An isolated development stream with its own assets and dependencies |
| **Fork** | Creating a new VersionLine by copying selected assets from an existing one |
| **Default VersionLine** | Migration target for existing data; named "default" |
| **Version Line Status** | Lifecycle state: Active, Maintenance, Archived |

### 9.2 References

- Original requirement discussion: [link to conversation]
- ADAM Specification: `docs/spec.md`
- ADAM Architecture: `docs/architecture.md`

---

*End of Document*
