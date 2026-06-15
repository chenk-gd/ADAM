# Work Item Kind Dependency Model Implementation Plan

**Goal:** Upgrade `work_item` from a requirement-breakdown-only asset into a typed work container through `metadata.work_item_kind`, while keeping dependency legality, relationship semantics, and Dirty propagation explicit.

**Architecture:** Keep `work_item` as one asset type. Use metadata-filtered dependency rules to infer relationship and propagation behavior at publish time. Rust domain code uses strong enums; REST, MCP, and database boundaries use stable snake_case strings.

---

## 1. Scope

Included:

- `work_item` subtypes via `metadata.work_item_kind`
- strong `RelationshipType` on dependency rules and dependency records
- explicit `PropagationPolicy`
- metadata-aware dependency rule matching during publish
- Dirty propagation filtered by propagation policy
- REST/MCP compatibility with snake_case strings
- database migration and seed backfill

Deferred:

- `EventOnly` policy
- automatic event-to-work-item creation
- new `defect_report` or `test_report` asset types
- broad workflow rules for refactor, release, maintenance, and pipeline automation
- JSONPath or arbitrary metadata predicates

## 2. Design Decisions

### 2.1 Work Item Subtypes

Do not create separate asset types for bugs, test execution tasks, refactors, or release tasks in this slice.

Represent subtype in metadata:

```json
{
  "work_item_kind": "bugfix",
  "external_status": "in_progress",
  "priority": "high",
  "severity": "major"
}
```

Supported first-slice values:

```text
feature
bugfix
test_execution
```

The built-in asset types remain:

```text
requirement
work_item
design_doc
code_commit
test_case
pipeline
```

This slice must not introduce `defect_report` or `test_report`. Bug scope is represented by `requirement` plus `work_item(kind=bugfix)`. Reproduction or verification context is represented by `test_case`.

### 2.2 RelationshipType

Use `RelationshipType` in both `DependencyRule` and `AssetDependencyRecord`.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipType {
    DependsOn,
    References,
    Implements,
    Fixes,
    Verifies,
    Executes,
    Produces,
    Blocks,
    RelatesTo,
}
```

Database, REST, and MCP use:

```text
depends_on
references
implements
fixes
verifies
executes
produces
blocks
relates_to
```

Implement `as_str()`, `FromStr`, and `Display`. Code should not compare raw relationship strings.

### 2.3 PropagationPolicy

First-slice policies:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropagationPolicy {
    Dirty,
    ContextOnly,
    AuditOnly,
}
```

Meanings:

- `Dirty`: upstream publish can mark downstream Dirty.
- `ContextOnly`: usable for context and graph traversal; never creates Dirty queue entries.
- `AuditOnly`: preserved for traceability; never creates Dirty queue entries.

API/database strings:

```text
dirty
context_only
audit_only
```

### 2.4 Relationship Default Policy

`RelationshipType::default_propagation_policy()` must implement:

| Relationship | Default Policy |
| --- | --- |
| `DependsOn` | `Dirty` |
| `Implements` | `Dirty` |
| `Fixes` | `Dirty` |
| `Verifies` | `Dirty` |
| `References` | `ContextOnly` |
| `Executes` | `ContextOnly` |
| `Produces` | `AuditOnly` |
| `Blocks` | `AuditOnly` |
| `RelatesTo` | `AuditOnly` |

### 2.5 Propagation Policy Resolution

When publishing an asset with dependencies, final policy is resolved in this order:

```text
1. Publish request explicit propagation_policy
2. Most specific metadata-matching DependencyRule.propagation_policy
3. Type-level DependencyRule.propagation_policy
4. relationship.default_propagation_policy()
```

### 2.6 Legality vs Inference

Dependency legality remains type-level:

```text
source_type_id + target_type_id must be allowed by at least one DependencyRule.
```

Metadata filters refine inference only:

```text
If metadata-specific rules match, use the most specific one for relationship/policy inference.
If no metadata-specific rule matches, do not reject solely for that reason.
```

Metadata matching supports exact top-level key equality only.

## 3. First-Slice Default Rule Matrix

| Source Asset | Source Filter | Relationship | Target Asset | Target Filter | Propagation | Purpose |
| --- | --- | --- | --- | --- | --- | --- |
| `work_item` | `work_item_kind=feature` | `Implements` | `requirement` | none | `Dirty` | Feature work tracks requirement changes. |
| `work_item` | `work_item_kind=bugfix` | `Fixes` | `requirement` | none | `Dirty` | Bugfix tracks the requirement or defect scope it corrects. |
| `work_item` | `work_item_kind=bugfix` | `References` | `test_case` | none | `ContextOnly` | Test case gives reproduction or verification context. |
| `work_item` | `work_item_kind=test_execution` | `Executes` | `test_case` | none | `ContextOnly` | Test execution uses test cases without becoming Dirty by default. |
| `test_case` | none | `Verifies` | `requirement` | none | `Dirty` | Test case is reviewed when requirement changes. |

Compatibility defaults:

| Existing Relationship | Policy |
| --- | --- |
| `depends_on` | `Dirty` |
| `references` | `ContextOnly` |

## 4. Implementation Requirements

### Domain

- Extend `crates/adam-domain/src/dependency/rule.rs`.
- Re-export `RelationshipType` and `PropagationPolicy`.
- Add `source_metadata_filter`, `target_metadata_filter`, and `propagation_policy` to `DependencyRule`.
- Add serde defaults:

```rust
#[serde(default)]
pub source_metadata_filter: Option<serde_json::Value>,
#[serde(default)]
pub target_metadata_filter: Option<serde_json::Value>,
#[serde(default = "default_propagation_policy")]
pub propagation_policy: PropagationPolicy,
```

- Change `AssetDependencyRecord.relationship` from `String` to `RelationshipType`.
- Add `AssetDependencyRecord.propagation_policy: PropagationPolicy`.
- Add `AssetDependencyRecord::new(...)`, `with_propagation_policy(...)`, and `with_upgrade_policy(...)`.

### Rule Repository

Extend `DependencyRuleRepository` with:

```rust
async fn find_allowed_type_rules(
    &self,
    source_type: &AssetTypeId,
    target_type: &AssetTypeId,
) -> Result<Vec<DependencyRule>, crate::RepositoryError>;

async fn find_matching_rules(
    &self,
    source_type: &AssetTypeId,
    source_metadata: &serde_json::Value,
    target_type: &AssetTypeId,
    target_metadata: &serde_json::Value,
) -> Result<Vec<DependencyRule>, crate::RepositoryError>;
```

`is_dependency_allowed` remains a type-level check.

### VersionService

`PublishDependency` must include:

```rust
pub relationship: Option<RelationshipType>,
pub propagation_policy: Option<PropagationPolicy>,
pub upgrade_policy: Option<UpgradePolicy>,
```

`VersionService::publish` should receive the rule repository only for the publish path:

```rust
pub async fn publish(
    &self,
    cmd: PublishAssetCommand,
    rule_repo: &dyn DependencyRuleRepository,
) -> Result<AssetVersion, VersionServiceError>
```

Publishing must:

1. load source and target assets
2. validate type-level rules
3. find metadata-matching rules
4. resolve relationship/policy by the priority chain
5. persist typed `AssetDependencyRecord`

### StatePropagator

Dirty propagation must skip non-dirty policies before version constraint checks:

```rust
if !dependency.propagation_policy.triggers_dirty() {
    skipped += 1;
    continue;
}
```

Fallback records from simple downstream lookup must preserve legacy behavior:

```rust
relationship: RelationshipType::DependsOn
propagation_policy: PropagationPolicy::Dirty
```

### REST / MCP

REST and MCP accept and return snake_case strings only:

```text
relationship: "references"
propagation_policy: "context_only"
```

Do not use `format!("{:?}", enum_value)` for API output.

### Database

Create a new migration, for example:

```text
migrations/013_work_item_kind_dependency_policy.sql
```

Do not modify previously executed migrations such as `012_default_dependency_rules.sql`.

The new migration should:

- add `source_metadata_filter`, `target_metadata_filter`, and `propagation_policy` to `dependency_rules`
- add `propagation_policy` to `asset_dependencies`
- backfill existing rows
- insert the first-slice metadata-filtered rules
- add check constraints for relationship and propagation policy strings

## 5. Acceptance Criteria

1. `work_item(kind=feature)` implementing a requirement becomes Dirty when that requirement publishes a matching new version.
2. `work_item(kind=bugfix)` fixing a requirement becomes Dirty when that requirement publishes a matching new version.
3. `work_item(kind=bugfix)` referencing a test case does not become Dirty when that test case publishes a new version.
4. `work_item(kind=test_execution)` executing a test case does not become Dirty by default when that test case publishes a new version.
5. `test_case` verifying a requirement becomes Dirty when the requirement publishes a matching new version.
6. A dependency with type-level rule but no metadata-rule match is allowed and uses defaults.
7. A dependency with no type-level rule is rejected.
8. REST/MCP return `dirty`, `context_only`, and `audit_only`, not Rust Debug enum names.
9. Existing publish requests without relationship or propagation fields still create `depends_on` / `dirty` dependencies.
10. Manual Clean updates effective baselines and does not propagate Dirty.

## 6. Verification Commands

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

Before committing, run GitNexus change detection:

```text
gitnexus_detect_changes(scope="all")
```
