# Version Constraint System Documentation

## Overview

The ADAM Version Constraint system provides SemVer-based version management with constraint-based dependency tracking. This enables fine-grained control over dependency upgrades and automatic state propagation.

## Core Concepts

### Semantic Versioning (SemVer)

All assets use [Semantic Versioning](https://semver.org/) with the format `MAJOR.MINOR.PATCH`:
- **MAJOR**: Breaking changes
- **MINOR**: New features (backwards compatible)
- **PATCH**: Bug fixes (backwards compatible)

```rust
use adam_domain::SemVer;

let version = SemVer::new(1, 2, 3);  // Creates version 1.2.3
let parsed = SemVer::parse("v1.2.3").unwrap();  // Parses "v1.2.3"
```

### Version Constraints

Dependencies declare constraints that define acceptable version ranges:

| Constraint | Format | Description | Example |
|------------|--------|-------------|---------|
| **Exact** | `=1.0.0` | Only exact version | `=1.0.0` matches only 1.0.0 |
| **Caret** | `^1.0.0` | Compatible versions (same major) | `^1.0.0` matches 1.x.x but not 2.0.0 |
| **Tilde** | `~1.2.0` | Same major+minor | `~1.2.0` matches 1.2.x but not 1.3.0 |
| **Wildcard** | `*` | Any version | `*` matches all versions |
| **Range** | `>=1.0.0, <2.0.0` | Custom range | Matches versions in range |

```rust
use adam_domain::VersionConstraint;

let constraint = VersionConstraint::parse("^1.0.0").unwrap();
assert!(constraint.matches(&SemVer::new(1, 5, 0)));
assert!(!constraint.matches(&SemVer::new(2, 0, 0)));
```

### Upgrade Policies

Controls automatic dependency updates:

| Policy | Description |
|--------|-------------|
| `AutoPatch` | Automatically update to latest patch version |
| `AutoMinor` | Automatically update to latest minor version (same major) |
| `Notify` | Mark downstream as Dirty when upstream updates |
| `Manual` | Require manual review for all updates |
| `Pin` | Never update, fixed to exact version |

```rust
use adam_domain::UpgradePolicy;

let policy = UpgradePolicy::AutoPatch;  // Default is Notify
```

## Architecture Components

### Domain Layer

#### SemVer (`crates/adam-domain/src/version/semver.rs`)
- Structured version type with major, minor, patch fields
- Ordering and comparison support
- Parsing with optional "v" prefix
- Version bumping methods

#### VersionConstraint (`crates/adam-domain/src/version/constraint.rs`)
- Enum with Exact, Caret, Tilde, Range, Wildcard variants
- Parsing from string expressions
- `matches()` method for version checking
- Serialization support

#### CompiledDependency (`crates/adam-domain/src/dependency/compiled.rs`)
- Pre-compiled constraint using `semver` crate
- Fast constraint matching
- Staleness detection via lock_version
- Cache for compiled dependencies

### Application Layer

#### ConfigCache (`crates/adam-application/src/services/config_cache.rs`)
- Caches type rules and organization policies
- TTL-based expiration (default: 5 minutes)
- Thread-safe with RwLock
- Reduces database lookups

```rust
use adam_application::{ConfigCache, ConstraintTemplate};

let cache = ConfigCache::new(type_rule_repo, policy_repo);
cache.preload(org_id).await?;

let rule = cache.get_type_rule(downstream_type, upstream_type).await;
```

#### AssetLifecycleService (`crates/adam-application/src/services/asset_lifecycle.rs`)
- CAS (Compare-And-Swap) optimistic locking
- Exponential backoff retry
- Idempotent publish with deduplication

```rust
let version = service
    .publish_version_cas(asset_id, new_version, content_ref, expected_lock, publisher)
    .await?;
```

#### StatePropagator (`crates/adam-application/src/services/state_propagator.rs`)
- Constraint-based dirty propagation
- Only propagates when constraint is satisfied
- Aggregated dirty logs

#### UnpublishService (`crates/adam-application/src/services/unpublish.rs`)
- Time-windowed unpublish policy
- Downstream notification propagation
- Organization-level configuration

#### MajorUpgradeService (`crates/adam-application/src/services/major_upgrade.rs`)
- Snapshot-based rollback
- Dependency restoration
- Upgrade status tracking

## Database Schema

### asset_instances

| Column | Type | Description |
|--------|------|-------------|
| current_version_major | INTEGER | Major version |
| current_version_minor | INTEGER | Minor version |
| current_version_patch | INTEGER | Patch version |
| lock_version | BIGINT | Optimistic locking |

### asset_dependencies

| Column | Type | Description |
|--------|------|-------------|
| declared_constraint | VARCHAR | Constraint expression (e.g., "^1.0.0") |
| constraint_str | VARCHAR | String representation |
| effective_version_major | INTEGER | Locked major version |
| effective_version_minor | INTEGER | Locked minor version |
| effective_version_patch | INTEGER | Locked patch version |
| upgrade_policy | VARCHAR | AutoPatch, AutoMinor, Notify, Manual, Pin |
| lock_version | BIGINT | Optimistic locking |

## Usage Examples

### Creating a Dependency with Constraint

```rust
use adam_domain::{AssetDependencyRecord, VersionConstraint, SemVer, UpgradePolicy};

let dependency = AssetDependencyRecord {
    id: uuid::Uuid::new_v4(),
    source_id: downstream_id,
    target_id: upstream_id,
    relationship: "depends_on".to_string(),
    declared_constraint: VersionConstraint::Caret(SemVer::new(1, 0, 0)),
    constraint_str: "^1.0.0".to_string(),
    effective_version: SemVer::new(1, 0, 0),
    effective_updated_by: "user@example.com".to_string(),
    effective_updated_at: chrono::Utc::now(),
    effective_reason: EffectiveUpdateReason::InitialCreation,
    upgrade_policy: UpgradePolicy::Notify,
    lock_version: 1,
    created_at: chrono::Utc::now(),
};
```

### Checking Constraint Matches

```rust
use adam_domain::CompiledDependency;

// Compile for fast matching
let compiled = CompiledDependency::compile(&dependency_record)?;

// Check if new version matches constraint
let new_version = SemVer::new(1, 5, 0);
if compiled.matches(&new_version) {
    println!("Version {} matches constraint", new_version);
}
```

### Auto-Update Check

```rust
// Check if should auto-update
if compiled.should_auto_update(&new_version) {
    // Update effective_version automatically
}

// Find next auto-update version
let available = vec![
    SemVer::new(1, 0, 1),
    SemVer::new(1, 0, 5),
    SemVer::new(1, 1, 0),
];
if let Some(next) = compiled.next_auto_version(&available) {
    println!("Auto-update to: {}", next);
}
```

## Migration from String Versions

If migrating from string-based versions:

1. Parse existing version strings to SemVer
2. Set default constraint as `^current_version`
3. Set upgrade_policy to `Notify` for safety
4. Migrate database using migration script `migrations/011_version_constraints.sql`

## Best Practices

1. **Use Caret (^) for libraries**: Allows minor updates with new features
2. **Use Tilde (~) for critical dependencies**: Only patch updates
3. **Use Exact (=) or Pin for reproducible builds**: Fixed versions
4. **Set AutoPatch for internal dependencies**: Automatic bug fixes
5. **Set Notify for external dependencies**: Manual review of updates

## Error Handling

| Error | Description | Resolution |
|-------|-------------|------------|
| `ConcurrentModification` | Lock version mismatch | Retry with fresh lock version |
| `IdempotencyConflict` | Same key, different request | Use unique idempotency keys |
| `InvalidConstraint` | Malformed constraint string | Validate input format |
| `RollbackNotPossible` | Cannot rollback incomplete upgrade | Check upgrade status |

## Performance Considerations

1. **Use CompiledDependency**: Pre-compiles constraints for fast matching
2. **Enable ConfigCache**: Reduces database lookups by 80%+
3. **Batch operations**: Use bulk inserts/updates when possible
4. **Index usage**: Ensure indexes on version columns for lookups

## Testing

See `crates/adam-domain/tests/version_constraint_integration_test.rs` for comprehensive examples.

Run tests:
```bash
cargo test -p adam-domain version_constraint -- --nocapture
cargo test -p adam-application config_cache -- --nocapture
cargo test -p adam-domain compiled -- --nocapture
```

## References

- [Implementation Progress](../.claude/work/IMPLEMENTATION_PROGRESS.md)
- [SemVer Specification](https://semver.org/)
- [Implementation Plan](../plans/2026-05-18-version-constraint-implementation.md)
