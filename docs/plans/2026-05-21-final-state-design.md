# Final State Design for Immutable Assets

**Date**: 2026-05-21
**Status**: Implemented

## Problem Statement

Code commits (and similar assets like pipeline runs) are fundamentally immutable - once created, they never change. The original state model (Clean/Dirty/Archived) didn't account for this:

- **Code commit**: Git hash is fixed, content never changes
- **Traditional assets**: Requirements, design docs can be updated and published with new versions

## Solution: Final State

### Core Design

Added a new `Final` state to `AssetState` enum:

```rust
pub enum AssetState {
    Clean,      // Mutable assets: can become Dirty, can be published
    Dirty,      // Mutable assets: need update
    Archived,   // All assets: manually deprecated
    Final,      // Immutable assets: code_commit, pipeline_run
}
```

### Final State Rules

| Rule | Behavior |
|------|----------|
| **Creation** | Created directly in `Final` state (no transition) |
| **State Changes** | No transitions allowed (terminal state) |
| **Dirty Propagation** | Does NOT receive dirty (immutable, never outdated) |
| **Publishing** | Cannot be published (already "published" at creation) |
| **Dependencies** | Can be depended upon (test cases can reference commits) |
| **Archival** | Can be archived if needed (separate from Final) |

### Asset Classification

| Asset Type | Mutability | Initial State | Receives Dirty | Version Strategy |
|-----------|-----------|---------------|----------------|------------------|
| requirement | Mutable | Clean | Yes | SemVer |
| design_doc | Mutable | Clean | Yes | SemVer |
| work_item | Mutable | Clean | Yes | SemVer |
| test_case | Mutable | Clean | Yes | SemVer |
| **code_commit** | **Immutable** | **Final** | **No** | **ExternalRef** |
| **pipeline_run** | **Immutable** | **Final** | **No** | **ExternalRef** |

### New Constructor Methods

For immutable assets, use dedicated constructors:

```rust
// Project-level immutable asset (e.g., code commit)
let commit = AssetInstance::new_immutable_project_level(
    "feat: add login",
    code_commit_type_id,
    project_id,
    org_id,
    "abc123def456",  // Git hash as external_ref
    "git",
    json!({"author": "user", "message": "..."}),
);

// Organization-level immutable asset (e.g., pipeline run)
let run = AssetInstance::new_immutable_organization_level(
    "build #42",
    pipeline_run_type_id,
    org_id,
    "pipeline-42",   // Run ID as external_ref
    "ci",
    json!({"status": "success"}),
);
```

## Key Insights

1. **Final ≠ Archived**: 
   - `Archived` = "intentionally deprecated, no longer used"
   - `Final` = "immutable by nature, always valid but never changes"

2. **Dirty Propagation**: Only affects mutable assets. When a requirement publishes v2:
   - Design doc → becomes Dirty (needs update)
   - Code commit → stays Final (already captured at creation time)

3. **External References**: Immutable assets use `external_ref` (git hash, run ID) rather than SemVer for versioning.

## Implementation Details

### Files Modified

1. `crates/adam-domain/src/asset/state.rs`
   - Added `Final` variant to `AssetState`
   - Updated `can_transition_to()` to block all transitions from Final
   - Added `is_final()` and `can_receive_dirty()` methods
   - Updated tests

2. `crates/adam-domain/src/asset/instance.rs`
   - Added `new_immutable_project_level()` constructor
   - Added `new_immutable_organization_level()` constructor

3. `crates/adam-infrastructure/src/repositories/postgres.rs`
   - Added "final" string mapping for database persistence

4. `crates/adam-adapters/src/rest/mod.rs`
   - Added `Final` to `AssetStateDto`
   - Updated serialization/deserialization

## Future Considerations

- **Query Enhancement**: When querying a code commit, could show "based on requirement-v1 (newer version v2 exists)" as informational
- **Auto-Creation**: Git hooks could auto-create code_commit assets on push
- **Batch Import**: Tool to import historical commits as Final assets
