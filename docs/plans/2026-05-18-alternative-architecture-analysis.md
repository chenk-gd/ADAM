# Alternative Architecture Analysis: Fork vs Asset Version History

**Date**: 2026-05-18  
**Context**: ADAM Version Line Design Review  
**Objective**: Evaluate alternatives to the Fork-based version line approach

---

## Executive Summary

After deep analysis, **Option 1: Asset Version History** is architecturally superior to Fork for the following reasons:

1. **Preserves Single Source of Truth**: Assets remain unique, versions are just history
2. **SemVer is Designed for This**: Semantic versioning naturally supports parallel version lines
3. **Simpler Mental Model**: Users think "REQ-1 has versions 1.x and 2.x", not "REQ-1 and REQ-1' are different assets"
4. **Better Query Performance**: No cross-table joins for version line membership
5. **Natural Dependency Resolution**: Constraint-based dependencies ("^1.0.0") automatically resolve to correct version

**However**: This requires significant redesign of:
- Dirty state propagation (per-version, not per-asset)
- Dependency model (constraint-based, not point-in-time)
- Version line definition (filter, not container)

---

## Detailed Comparison

### Current: Fork-Based (Physical Separation)

```rust
// v1.x has REQ-1 (id: A1)
// v2.x has REQ-1' (id: A2) - DIFFERENT ASSET!

pub struct AssetInstance {
    pub id: AssetId,              // A1 vs A2 - different!
    pub version_line_id: VersionLineId,  // v1.x vs v2.x
    pub name: "REQ-1",            // Same name, different asset
    pub current_version: "1.0.0", // v1.x: 1.0.0, v2.x: 2.0.0
}

// Dependencies are between SPECIFIC asset instances
pub struct AssetDependency {
    pub downstream_id: AssetId,   // CODE-1 (A3)
    pub upstream_id: AssetId,     // REQ-1 (A1) - points to specific asset
    pub declared_version: "1.0.0", // Snapshot at publish time
}
```

**Problems:**
1. **Identity Crisis**: REQ-1 (v1.x) and REQ-1' (v2.x) are logically the same but technically different
2. **External References**: Git commit says "Refs: REQ-1" - which one?
3. **Cross-Version Sync**: Security fix in v1.x REQ-1 doesn't automatically affect v2.x REQ-1'
4. **Data Duplication**: N version lines = N copies of potentially same asset

---

### Option 1: Asset Version History (Recommended)

```rust
// REQ-1 is ONE asset with MULTIPLE versions

pub struct AssetInstance {
    pub id: AssetId,              // A1 - single identity
    pub name: "REQ-1",
    pub current_version: SemVer,  // Latest: 2.0.0
    pub versions: Vec<AssetVersion>, // [1.0.0, 1.1.0, 1.2.0, 2.0.0]
}

pub struct AssetVersion {
    pub version: SemVer,          // 1.0.0, 1.1.0, 2.0.0
    pub content_ref: String,      // Git commit, Jira ticket, etc.
    pub state: AssetState,        // Per-version state!
    pub created_at: DateTime<Utc>,
    pub is_lts: bool,            // Long-term support flag
}

// Dependencies use CONSTRAINTS, not point-in-time references
pub struct AssetDependency {
    pub downstream_id: AssetId,   // CODE-1
    pub upstream_id: AssetId,     // REQ-1 (single asset!)
    pub version_constraint: String, // ">=1.0.0, <2.0.0" or "^1.0.0"
}

// VersionLine is a FILTER, not a container
pub struct VersionLine {
    pub id: VersionLineId,
    pub name: "v1.x",
    pub version_constraint: ">=1.0.0, <2.0.0", // SemVer range
    pub auto_include: bool,      // Include new matching versions?
}
```

**Advantages:**

| Aspect | Benefit |
|--------|---------|
| **Identity** | REQ-1 is always REQ-1, regardless of version |
| **External Refs** | Git "Refs: REQ-1" unambiguous - it's the asset |
| **Dirty State** | Per-version dirty state: "CODE-1 is dirty against REQ-1@1.5.0" |
| **Dependencies** | Constraints auto-resolve: "^1.0.0" matches 1.1.0, 1.5.0, not 2.0.0 |
| **No Sync Needed** | Security fix in 1.1.0 automatically applies to all "^1.0.0" dependents |
| **Query** | Simple: `WHERE version >= 1.0.0 AND version < 2.0.0` |

**Implementation - Dirty State Propagation:**

```rust
/// State is per VERSION, not per asset
pub struct AssetVersion {
    pub version: SemVer,
    pub state: AssetVersionState,  // Clean | Dirty | Archived
    pub dirty_reason: Option<DirtyReason>,
}

pub enum DirtyReason {
    /// Upstream published new version within constraint
    UpstreamUpdated {
        upstream_asset: AssetId,
        new_version: SemVer,
    },
    /// Direct change to this version
    DirectChange,
}

impl StatePropagationService {
    /// When upstream publishes new version
    pub async fn propagate_publish(
        &self,
        upstream_asset: AssetId,
        new_version: SemVer,
    ) -> Result<(), StateError> {
        // Find all downstream assets that depend on this upstream
        let deps = self.dependency_repo
            .find_downstream(&upstream_asset)
            .await?;
        
        for dep in deps {
            // Parse the constraint
            let constraint = VersionConstraint::parse(&dep.version_constraint)?;
            
            // Does new version satisfy constraint?
            if constraint.satisfies(&new_version) {
                // Mark the SPECIFIC version range as dirty
                // e.g., "CODE-1 ^1.0.0 is dirty because REQ-1@1.5.0 exists"
                self.mark_version_dirty(
                    &dep.downstream_id,
                    &dep.version_constraint,
                    &upstream_asset,
                    &new_version,
                ).await?;
            }
        }
        
        Ok(())
    }
}
```

**Implementation - Version Line as Filter:**

```rust
impl AssetRepository {
    /// Get assets in version line (by constraint, not by ID)
    pub async fn find_by_version_line(
        &self,
        version_line: &VersionLine,
    ) -> Result<Vec<AssetInstance>, RepositoryError> {
        let constraint = VersionConstraint::parse(&version_line.version_constraint)?;
        
        // Find all assets with versions matching the constraint
        let assets = sqlx::query_as::<_, AssetRow>(
            r#"
            SELECT a.*, v.version, v.state
            FROM asset_instances a
            JOIN asset_versions v ON a.id = v.asset_id
            WHERE v.version = (
                SELECT MAX(version) 
                FROM asset_versions 
                WHERE asset_id = a.id 
                AND version BETWEEN $1 AND $2
            )
            "#,
        )
        .bind(&constraint.min_version())
        .bind(&constraint.max_version())
        .fetch_all(&self.pool)
        .await?;
        
        Ok(assets.into_iter().map(|r| r.into()).collect())
    }
}
```

**Challenges:**

1. **Complex Dirty State**: Need to track "which version of CODE-1 is dirty against which version of REQ-1"
2. **Constraint Resolution**: Must implement SemVer constraint matching
3. **Query Complexity**: "Assets in v1.x" requires subquery to find latest matching version
4. **Migration**: Existing point-in-time dependencies need conversion

---

### Option 2: VersionLine as View (Lightweight)

```rust
// No version_line_id on assets!
pub struct AssetInstance {
    pub id: AssetId,
    pub name: "REQ-1",
    pub current_version: SemVer,  // 2.0.0
    // NO version_line_id!
}

// VersionLine is just a query parameter
pub struct VersionLine {
    pub id: VersionLineId,
    pub name: "v1.x",
    pub version_constraint: ">=1.0.0, <2.0.0",
}

// Query by constraint
pub async fn find_assets_in_version_line(
    &self,
    vl: &VersionLine,
) -> Result<Vec<AssetInstance>, Error> {
    self.asset_repo
        .find_by_version_constraint(&vl.version_constraint)
        .await
}
```

**Advantages:**
- Minimal schema changes
- No data duplication
- Automatic inclusion of new versions

**Disadvantages:**
- Query performance (parse constraint on every query)
- No explicit version line membership (implicit via version number)
- Harder to reason about "what's in v1.x"

---

### Option 3: Project Isolation

```rust
// v1.x and v2.x are different Projects
pub struct Project {
    pub id: ProjectId,
    pub name: "MyApp v1.x",
    pub version_line: VersionLineId,  // Optional: v1.x, v2.x
}

pub struct AssetInstance {
    pub id: AssetId,
    pub project_id: ProjectId,  // v1 project or v2 project
    pub name: "REQ-1",
    pub current_version: SemVer,
}

// Cross-project dependencies
pub struct AssetDependency {
    pub downstream_project: ProjectId,  // v2 project
    pub downstream_asset: AssetId,      // CODE-1 (v2)
    pub upstream_project: ProjectId,    // v1 project
    pub upstream_asset: AssetId,        // REQ-1 (v1)
}
```

**Advantages:**
- Reuses existing Project concept
- Natural permission boundaries
- Cross-project dependencies are explicit

**Disadvantages:**
- Still duplicates assets (v1 REQ-1 vs v2 REQ-1)
- Complex cross-project dependency management
- Organization-level assets still problematic

---

### Option 4: Git-Style Branches (Pointers)

```rust
// Asset has versions
pub struct AssetInstance {
    pub id: AssetId,
    pub versions: Vec<AssetVersion>,
}

// VersionLine points to specific versions
pub struct VersionLine {
    pub id: VersionLineId,
    pub name: "v1.x",
    pub pointers: HashMap<AssetId, SemVer>, // REQ-1 -> 1.5.0
}

// Dependencies use pointers
pub struct AssetDependency {
    pub downstream_id: AssetId,
    pub upstream_id: AssetId,
    pub upstream_versionline: VersionLineId, // Which pointer to use?
}
```

**Advantages:**
- Flexible: different version lines can have different versions
- Git-like mental model

**Disadvantages:**
- Complex pointer management
- Dependencies need version line context
- Query requires JOIN with pointers table
- Merge is complex (which pointer wins?)

---

## Comparison Matrix

| Criteria | Fork (Current) | Asset History (Opt 1) | View (Opt 2) | Project (Opt 3) | Git Pointers (Opt 4) |
|----------|---------------|----------------------|--------------|-----------------|---------------------|
| **Asset Uniqueness** | ❌ Multiple copies | ✅ Single asset | ✅ Single asset | ❌ Multiple copies | ✅ Single asset |
| **External Refs** | ❌ Ambiguous | ✅ Clear | ✅ Clear | ⚠️ Project-scoped | ⚠️ Pointer-scoped |
| **Dirty Propagation** | ❌ Per-line manual | ✅ Constraint-based | ✅ Constraint-based | ⚠️ Cross-project | ⚠️ Complex |
| **Query Performance** | ⚠️ Join required | ⚠️ Subquery required | ⚠️ Parse constraint | ✅ Simple | ❌ Pointer JOIN |
| **Migration Cost** | - | High | Low | Medium | High |
| **Conceptual Simplicity** | ⚠️ Fork mental model | ✅ SemVer natural | ✅ Simple filter | ⚠️ Project proliferation | ⚠️ Pointer indirection |
| **Feature Branch Support** | ✅ Easy fork | ⚠️ Version naming | ⚠️ Constraint naming | ✅ Project per branch | ✅ Easy to create |

---

## Recommendation: Hybrid Approach

After careful consideration, I recommend a **hybrid of Option 1 (Asset Version History) with elements of Option 4 (Git pointers) for feature branches**.

### Core Model: Asset Version History

```rust
pub struct AssetInstance {
    pub id: AssetId,              // Single identity
    pub name: String,
    pub versions: Vec<AssetVersion>, // Version history
    pub current_version: SemVer,     // Latest version
    pub default_version_line: VersionLineId, // Default constraint
}

pub struct AssetVersion {
    pub version: SemVer,
    pub content_ref: String,
    pub state: AssetVersionState, // Per-version!
    pub created_at: DateTime<Utc>,
    pub tags: Vec<String>,        // "lts", "stable", "beta"
}

// Dependencies use constraints
pub struct AssetDependency {
    pub downstream_id: AssetId,
    pub upstream_id: AssetId,
    pub version_constraint: String, // ">=1.0.0, <2.0.0"
}
```

### Version Line: Named Constraints

```rust
pub struct VersionLine {
    pub id: VersionLineId,
    pub name: "v1.x",
    pub constraint: ">=1.0.0, <2.0.0",
    pub auto_update: bool,         // Auto-include new matching versions?
    pub created_from: Option<SemVer>, // Forked from version X
}
```

### Feature Branches: Explicit Pointers

For feature branches (where you need specific versions), use explicit overrides:

```rust
pub struct FeatureBranch {
    pub id: FeatureBranchId,
    pub name: "feature-x",
    pub base_version_line: VersionLineId, // v1.x
    pub overrides: HashMap<AssetId, SemVer>, // REQ-1 -> 1.5.0-beta
}
```

### Migration from Current Design

```rust
/// Migration strategy
pub async fn migrate_to_version_history(&self) -> Result<(), Error> {
    // Step 1: Add versions table
    // Step 2: Convert existing assets to versions
    // Step 3: Convert dependencies to constraints
    // Step 4: Create version lines from existing data
    // Step 5: Remove forked duplicates (merge by name + version)
}
```

---

## Why Not Fork?

**Fundamental Issue**: Fork treats "v1.x REQ-1" and "v2.x REQ-1" as different assets, when they should be the same asset at different versions.

**Real-world analogy:**
- **Fork**: "v1.x REQ-1" and "v2.x REQ-1" are like "iPhone 13" and "iPhone 14" - different products
- **Version History**: "v1.x REQ-1" and "v2.x REQ-1" are like "iOS 15" and "iOS 16" - same product, different versions

ADAM should model the latter, not the former.

---

## Next Steps

If you agree with this analysis:

1. **Archive** the Fork-based design documents
2. **Create** new design based on Asset Version History
3. **Plan** migration strategy from current data
4. **Prototype** the constraint-based dependency resolution

This is a significant change, but it aligns ADAM with industry-standard versioning models (SemVer, npm, Cargo) rather than inventing a new paradigm.

---

*Document Status*: Recommendation awaiting review  
*Decision Required*: Proceed with Asset Version History or continue with Fork?
