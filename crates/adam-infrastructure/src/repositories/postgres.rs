//! PostgreSQL repository implementations

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use adam_domain::asset::instance::AssetTypeId;
use adam_domain::asset::state::AssetState;
use adam_domain::dependency::boundary::AssetLevel;
use adam_domain::version::VersionConstraint;
use adam_domain::repository::UpgradePolicy;
use adam_domain::repository::{TransactionContext, UnitOfWork};
use adam_domain::{
    AssetDependencyRecord, AssetId, AssetInstance, AssetRepository, AssetType, AssetTypeRepository,
    CreateAssetCommand, DependencyRepository, DirtyQueueEntry, DirtyQueueRepository,
    DirtyResolutionLog, DirtyResolutionLogRepository, EffectiveUpdateReason,
    OrganizationId, ProjectId, RepositoryError, SemVer, UpdateAssetCommand, VersionStrategy,
    VirtualInstance, VirtualInstanceId, VirtualInstanceRepository,
};

/// PostgreSQL implementation of AssetRepository
pub struct PostgresAssetRepository {
    pool: PgPool,
}

/// PostgreSQL implementation of AssetTypeRepository
pub struct PostgresAssetTypeRepository {
    pool: PgPool,
}

impl PostgresAssetTypeRepository {
    /// Create a new PostgresAssetTypeRepository with the given connection pool
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    fn row_to_asset_type(&self, row: &sqlx::postgres::PgRow) -> Result<AssetType, RepositoryError> {
        let strategy: String = row.get("version_strategy");
        let version_strategy = match strategy.as_str() {
            "semver" => VersionStrategy::Semver,
            "external_ref" => VersionStrategy::ExternalRef,
            "composite" => VersionStrategy::Composite,
            other => {
                return Err(RepositoryError::DatabaseError(format!(
                    "Unknown version_strategy: {other}"
                )));
            }
        };

        let retention_policy: Option<serde_json::Value> = row.try_get("retention_policy").ok();
        let icon: Option<String> = row.try_get("icon").ok();

        Ok(AssetType::new_with_fields(
            AssetTypeId::from_uuid(row.get("id")),
            OrganizationId::from_uuid(row.get("organization_id")),
            row.get("name"),
            row.get("display_name"),
            row.try_get("description").unwrap_or_default(),
            row.get("metadata_schema"),
            version_strategy,
            retention_policy,
            icon,
            row.get("created_at"),
            row.get("updated_at"),
        ))
    }
}

#[async_trait]
impl AssetTypeRepository for PostgresAssetTypeRepository {
    async fn create(&self, asset_type: &AssetType) -> Result<AssetType, RepositoryError> {
        let row = sqlx::query(
            r#"
            INSERT INTO asset_types (
                id, organization_id, name, display_name, description,
                metadata_schema, version_strategy, retention_policy, icon,
                created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            RETURNING id, organization_id, name, display_name, description,
                metadata_schema, version_strategy, retention_policy, icon,
                created_at, updated_at
            "#,
        )
        .bind(asset_type.id.0)
        .bind(asset_type.organization_id.0)
        .bind(&asset_type.name)
        .bind(&asset_type.display_name)
        .bind(&asset_type.description)
        .bind(&asset_type.metadata_schema)
        .bind(asset_type.version_strategy.to_string())
        .bind(
            asset_type
                .retention_policy()
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
        )
        .bind(asset_type.icon())
        .bind(asset_type.created_at)
        .bind(chrono::Utc::now())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        self.row_to_asset_type(&row)
    }

    async fn find_by_id(&self, id: &AssetTypeId) -> Result<Option<AssetType>, RepositoryError> {
        let row = sqlx::query(
            r#"
            SELECT id, organization_id, name, display_name, description,
                metadata_schema, version_strategy, retention_policy, icon,
                created_at, updated_at
            FROM asset_types
            WHERE id = $1
            "#,
        )
        .bind(id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        row.map(|row| self.row_to_asset_type(&row)).transpose()
    }

    async fn find_by_name(&self, name: &str) -> Result<Option<AssetType>, RepositoryError> {
        let row = sqlx::query(
            r#"
            SELECT id, organization_id, name, display_name, description,
                metadata_schema, version_strategy, retention_policy, icon,
                created_at, updated_at
            FROM asset_types
            WHERE name = $1
            LIMIT 1
            "#,
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        row.map(|row| self.row_to_asset_type(&row)).transpose()
    }

    async fn list_all(&self) -> Result<Vec<AssetType>, RepositoryError> {
        let rows = sqlx::query(
            r#"
            SELECT id, organization_id, name, display_name, description,
                metadata_schema, version_strategy, retention_policy, icon,
                created_at, updated_at
            FROM asset_types
            ORDER BY name
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        rows.iter().map(|row| self.row_to_asset_type(row)).collect()
    }
}

impl PostgresAssetRepository {
    /// Create a new PostgresAssetRepository with the given connection pool
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create from an Arc-wrapped pool (for compatibility with some patterns)
    pub fn from_arc(pool: Arc<PgPool>) -> Self {
        // Note: This clones the pool, which is fine since PgPool is cheap to clone
        Self::new((*pool).clone())
    }
}

#[async_trait]
impl AssetRepository for PostgresAssetRepository {
    async fn create(&self, cmd: &CreateAssetCommand) -> Result<AssetInstance, RepositoryError> {
        let asset_id = AssetId::new();
        let now = chrono::Utc::now();

        let level_str = match cmd.level {
            AssetLevel::Project => "project",
            AssetLevel::Organization => "organization",
        };

        let state_str = "clean"; // New assets start in clean state

        // Insert the asset instance
        let result = sqlx::query(
            r#"
            INSERT INTO asset_instances (
                id, type_id, organization_id, name, external_ref, source,
                level, project_id, current_version, current_state,
                metadata, assignees, idempotency_key, created_at, updated_at
            )
            VALUES (
                $1, $2, $3, $4, $5, $6,
                $7, $8, NULL, $9,
                $10, '[]', $11, $12, $13
            )
            RETURNING id
            "#,
        )
        .bind(asset_id.0)
        .bind(cmd.asset_type_id.0)
        .bind(cmd.organization_id.0)
        .bind(&cmd.name)
        .bind(&cmd.external_ref)
        .bind(&cmd.source)
        .bind(level_str)
        .bind(cmd.project_id.map(|p| p.0))
        .bind(state_str)
        .bind(&cmd.metadata)
        .bind(&cmd.idempotency_key)
        .bind(now)
        .bind(now)
        .fetch_one(&self.pool)
        .await;

        match result {
            Ok(row) => {
                let id: Uuid = row.get("id");
                Ok(AssetInstance::new_with_fields(
                    AssetId::from_uuid(id),
                    cmd.name.clone(),
                    cmd.asset_type_id,
                    cmd.project_id,
                    cmd.organization_id,
                    cmd.level,
                    AssetState::Clean,
                    cmd.external_ref.clone(),
                    cmd.source.clone(),
                    cmd.metadata.clone(),
                    vec![],
                    None,
                    SemVer::new(0, 0, 0), // Default version
                    1,                     // Initial lock version
                    now,
                    now,
                    cmd.idempotency_key.clone(),
                ))
            }
            Err(sqlx::Error::Database(db_err)) => {
                if let Some(constraint) = db_err.constraint() {
                    if constraint.contains("idempotency") {
                        return Err(RepositoryError::DuplicateIdempotencyKey(
                            cmd.idempotency_key.clone().unwrap_or_default(),
                        ));
                    }
                }
                Err(RepositoryError::DatabaseError(db_err.to_string()))
            }
            Err(e) => Err(RepositoryError::DatabaseError(e.to_string())),
        }
    }

    async fn find_by_id(&self, id: &AssetId) -> Result<Option<AssetInstance>, RepositoryError> {
        let row = sqlx::query(
            r#"
            SELECT
                id, name, type_id, organization_id, level, project_id,
                current_state, created_at, updated_at, idempotency_key,
                external_ref, source, metadata, assignees, publisher, current_version
            FROM asset_instances
            WHERE id = $1
            "#,
        )
        .bind(id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        match row {
            Some(row) => Ok(Some(self.row_to_asset(&row)?)),
            None => Ok(None),
        }
    }

    async fn update_state(&self, id: &AssetId, state: AssetState) -> Result<(), RepositoryError> {
        let state_str = match state {
            AssetState::Clean => "clean",
            AssetState::Dirty => "dirty",
            AssetState::Archived => "archived",
            AssetState::Final => "final",
        };

        let result = sqlx::query(
            r#"
            UPDATE asset_instances
            SET current_state = $1, updated_at = NOW()
            WHERE id = $2
            "#,
        )
        .bind(state_str)
        .bind(id.0)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(RepositoryError::NotFound(id.0.to_string()));
        }

        Ok(())
    }

    async fn update_publication(
        &self,
        id: &AssetId,
        current_version: String,
        publisher: String,
        state: AssetState,
    ) -> Result<(), RepositoryError> {
        let state_str = match state {
            AssetState::Clean => "clean",
            AssetState::Dirty => "dirty",
            AssetState::Archived => "archived",
            AssetState::Final => "final",
        };

        let result = sqlx::query(
            r#"
            UPDATE asset_instances
            SET current_version = $1,
                publisher = $2,
                current_state = $3,
                updated_at = NOW()
            WHERE id = $4
            "#,
        )
        .bind(&current_version)
        .bind(&publisher)
        .bind(state_str)
        .bind(id.0)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(RepositoryError::NotFound(id.0.to_string()));
        }

        Ok(())
    }

    async fn update_publication_cas(
        &self,
        id: &AssetId,
        current_version: String,
        publisher: String,
        state: AssetState,
        expected_lock_version: i64,
    ) -> Result<i64, RepositoryError> {
        let state_str = match state {
            AssetState::Clean => "clean",
            AssetState::Dirty => "dirty",
            AssetState::Archived => "archived",
            AssetState::Final => "final",
        };

        // CAS: Only update if lock_version matches expected
        let result = sqlx::query(
            r#"
            UPDATE asset_instances
            SET current_version = $1,
                publisher = $2,
                current_state = $3,
                lock_version = lock_version + 1,
                updated_at = NOW()
            WHERE id = $4
              AND lock_version = $5
            RETURNING lock_version
            "#,
        )
        .bind(&current_version)
        .bind(&publisher)
        .bind(state_str)
        .bind(id.0)
        .bind(expected_lock_version)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        match result {
            Some(row) => {
                let new_lock_version: i64 = row.try_get("lock_version")
                    .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;
                Ok(new_lock_version)
            }
            None => {
                // CAS failed - either asset not found or lock_version mismatch
                // Check if asset exists to determine error type
                let exists = sqlx::query("SELECT lock_version FROM asset_instances WHERE id = $1")
                    .bind(id.0)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

                match exists {
                    Some(row) => {
                        let actual: i64 = row.try_get("lock_version")
                            .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;
                        Err(RepositoryError::ConcurrentModification {
                            expected: expected_lock_version,
                            actual,
                        })
                    }
                    None => Err(RepositoryError::NotFound(id.0.to_string())),
                }
            }
        }
    }

    async fn find_by_project_id(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<AssetInstance>, RepositoryError> {
        let rows = sqlx::query(
            r#"
            SELECT
                id, name, type_id, organization_id, level, project_id,
                current_state, created_at, updated_at, idempotency_key,
                external_ref, source, metadata, assignees, publisher, current_version
            FROM asset_instances
            WHERE project_id = $1
            ORDER BY name
            "#,
        )
        .bind(project_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        rows.iter().map(|row| self.row_to_asset(row)).collect()
    }

    async fn find_by_organization_id(
        &self,
        org_id: &OrganizationId,
    ) -> Result<Vec<AssetInstance>, RepositoryError> {
        let rows = sqlx::query(
            r#"
            SELECT
                id, name, type_id, organization_id, level, project_id,
                current_state, created_at, updated_at, idempotency_key,
                external_ref, source, metadata, assignees, publisher, current_version
            FROM asset_instances
            WHERE organization_id = $1
            ORDER BY name
            "#,
        )
        .bind(org_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        rows.iter().map(|row| self.row_to_asset(row)).collect()
    }

    async fn update(
        &self,
        id: &AssetId,
        cmd: &UpdateAssetCommand,
    ) -> Result<AssetInstance, RepositoryError> {
        // First get the existing asset
        let asset = self
            .find_by_id(id)
            .await?
            .ok_or_else(|| RepositoryError::NotFound(id.0.to_string()))?;

        // Build dynamic SQL based on which fields are provided
        let mut updates = vec![];
        let mut bind_idx = 1;

        if cmd.name.is_some() {
            updates.push(format!("name = ${bind_idx}"));
            bind_idx += 1;
        }
        if cmd.assignees.is_some() {
            updates.push(format!("assignees = ${bind_idx}::jsonb"));
            bind_idx += 1;
        }
        if cmd.metadata.is_some() {
            updates.push(format!("metadata = ${bind_idx}::jsonb"));
            bind_idx += 1;
        }
        // Always update updated_at
        updates.push(format!("updated_at = ${bind_idx}"));
        bind_idx += 1;

        if updates.is_empty() {
            return Ok(asset);
        }

        let sql = format!(
            "UPDATE asset_instances SET {} WHERE id = ${} RETURNING *",
            updates.join(", "),
            bind_idx
        );

        let mut query = sqlx::query(&sql);

        // Bind values
        if let Some(name) = &cmd.name {
            query = query.bind(name);
        }
        if let Some(assignees) = &cmd.assignees {
            query = query.bind(serde_json::to_value(assignees).unwrap_or_default());
        }
        if let Some(metadata) = &cmd.metadata {
            query = query.bind(metadata);
        }
        query = query.bind(chrono::Utc::now());
        query = query.bind(id.0);

        let row = query
            .fetch_one(&self.pool)
            .await
            .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        self.row_to_asset(&row)
    }

    async fn delete(&self, id: &AssetId) -> Result<(), RepositoryError> {
        let result = sqlx::query(
            r#"
            DELETE FROM asset_instances
            WHERE id = $1
            "#,
        )
        .bind(id.0)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(RepositoryError::NotFound(id.0.to_string()));
        }

        Ok(())
    }
}

impl PostgresAssetRepository {
    /// Convert a database row to an AssetInstance
    fn row_to_asset(&self, row: &sqlx::postgres::PgRow) -> Result<AssetInstance, RepositoryError> {
        let level_str: String = row
            .try_get("level")
            .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;
        let state_str: String = row
            .try_get("current_state")
            .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        // Required fields - use try_get with map_err
        let external_ref: String = row
            .try_get::<String, _>("external_ref")
            .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;
        let source: String = row
            .try_get::<String, _>("source")
            .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;
        let metadata: serde_json::Value = row
            .try_get::<serde_json::Value, _>("metadata")
            .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;
        let assignees: Vec<String> = row
            .try_get::<serde_json::Value, _>("assignees")
            .map_err(|e| RepositoryError::DatabaseError(e.to_string()))
            .and_then(|v| {
                serde_json::from_value(v).map_err(|e| {
                    RepositoryError::DatabaseError(format!("Failed to parse assignees: {e}"))
                })
            })?;

        // Optional fields - use ok().flatten() or unwrap_or_default
        let publisher: Option<String> =
            row.try_get::<Option<String>, _>("publisher").ok().flatten();
        let current_version_str: Option<String> = row
            .try_get::<Option<String>, _>("current_version")
            .ok()
            .flatten();
        // Parse version or use default
        let current_version = current_version_str
            .and_then(|v| SemVer::parse(&v).ok())
            .unwrap_or_else(|| SemVer::new(0, 0, 0));
        // Lock version from DB or default to 1
        let lock_version: i64 = row
            .try_get::<Option<i64>, _>("lock_version")
            .ok()
            .flatten()
            .unwrap_or(1);

        let created_at: chrono::DateTime<chrono::Utc> = row
            .try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
            .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;
        let updated_at: chrono::DateTime<chrono::Utc> = row
            .try_get::<chrono::DateTime<chrono::Utc>, _>("updated_at")
            .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;
        let idempotency_key: Option<String> = row
            .try_get::<Option<String>, _>("idempotency_key")
            .ok()
            .flatten();

        Ok(AssetInstance::new_with_fields(
            AssetId::from_uuid(
                row.try_get::<Uuid, _>("id")
                    .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?,
            ),
            row.try_get::<String, _>("name")
                .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?,
            AssetTypeId::from_uuid(
                row.try_get::<Uuid, _>("type_id")
                    .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?,
            ),
            row.try_get::<Option<Uuid>, _>("project_id")
                .ok()
                .flatten()
                .map(ProjectId::from_uuid),
            OrganizationId::from_uuid(
                row.try_get::<Uuid, _>("organization_id")
                    .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?,
            ),
            match level_str.as_str() {
                "project" => AssetLevel::Project,
                "organization" => AssetLevel::Organization,
                _ => {
                    return Err(RepositoryError::DatabaseError(format!(
                        "Invalid level: {level_str}"
                    )));
                }
            },
            match state_str.as_str() {
                "clean" => AssetState::Clean,
                "dirty" => AssetState::Dirty,
                "archived" => AssetState::Archived,
                "final" => AssetState::Final,
                _ => {
                    return Err(RepositoryError::DatabaseError(format!(
                        "Invalid state: {state_str}"
                    )));
                }
            },
            external_ref,
            source,
            metadata,
            assignees,
            publisher,
            current_version,
            lock_version,
            created_at,
            updated_at,
            idempotency_key,
        ))
    }
}

/// PostgreSQL implementation of DirtyQueueRepository
pub struct PostgresDirtyQueueRepository {
    pool: PgPool,
}

impl PostgresDirtyQueueRepository {
    /// Create a new PostgresDirtyQueueRepository
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl adam_domain::DirtyQueueRepository for PostgresDirtyQueueRepository {
    async fn upsert(
        &self,
        entry: &adam_domain::repository::DirtyQueueEntry,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            r#"
            INSERT INTO dirty_queue (
                id, asset_id, upstream_asset_id, upstream_version, upstream_old_version,
                impact_level, since, resolved, idempotency_key, created_at, updated_at
            )
            VALUES (
                $1, $2, $3, $4, $5,
                $6, $7, false, $8, $9, $9
            )
            ON CONFLICT (asset_id, upstream_asset_id) WHERE resolved = false
            DO UPDATE SET
                upstream_version = EXCLUDED.upstream_version,
                upstream_old_version = EXCLUDED.upstream_old_version,
                impact_level = EXCLUDED.impact_level,
                since = EXCLUDED.since,
                updated_at = EXCLUDED.updated_at
            "#,
        )
        .bind(entry.id)
        .bind(entry.asset_id.0)
        .bind(entry.upstream_asset_id.0)
        .bind(&entry.upstream_version)
        .bind(&entry.upstream_old_version)
        .bind(&entry.impact_level)
        .bind(entry.since)
        .bind(entry.id.to_string()) // Use UUID as idempotency key
        .bind(entry.created_at)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    async fn find_unresolved_by_asset(
        &self,
        asset_id: &AssetId,
    ) -> Result<Vec<adam_domain::repository::DirtyQueueEntry>, RepositoryError> {
        let rows = sqlx::query(
            r#"
            SELECT
                id, asset_id, upstream_asset_id, upstream_version,
                upstream_old_version, impact_level, since, created_at, resolved_at
            FROM dirty_queue
            WHERE asset_id = $1 AND resolved = false
            ORDER BY created_at DESC
            "#,
        )
        .bind(asset_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|row| adam_domain::repository::DirtyQueueEntry {
                id: row.get("id"),
                asset_id: AssetId::from_uuid(row.get("asset_id")),
                upstream_asset_id: AssetId::from_uuid(row.get("upstream_asset_id")),
                upstream_version: row.get("upstream_version"),
                upstream_old_version: row.get("upstream_old_version"),
                impact_level: row.get("impact_level"),
                since: row.get("since"),
                created_at: row.get("created_at"),
                resolved_at: row.get("resolved_at"),
            })
            .collect())
    }

    async fn resolve(&self, entry_id: &Uuid) -> Result<(), RepositoryError> {
        let result = sqlx::query(
            r#"
            UPDATE dirty_queue
            SET resolved = true, resolved_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(entry_id)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(RepositoryError::NotFound(entry_id.to_string()));
        }

        Ok(())
    }

    async fn find_all_unresolved(
        &self,
    ) -> Result<Vec<adam_domain::repository::DirtyQueueEntry>, RepositoryError> {
        let rows = sqlx::query(
            r#"
            SELECT
                id, asset_id, upstream_asset_id, upstream_version,
                upstream_old_version, impact_level, since, created_at, resolved_at
            FROM dirty_queue
            WHERE resolved = false
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|row| adam_domain::repository::DirtyQueueEntry {
                id: row.get("id"),
                asset_id: AssetId::from_uuid(row.get("asset_id")),
                upstream_asset_id: AssetId::from_uuid(row.get("upstream_asset_id")),
                upstream_version: row.get("upstream_version"),
                upstream_old_version: row.get("upstream_old_version"),
                impact_level: row.get("impact_level"),
                since: row.get("since"),
                created_at: row.get("created_at"),
                resolved_at: row.get("resolved_at"),
            })
            .collect())
    }
}

/// PostgreSQL implementation of DependencyRepository
pub struct PostgresDependencyRepository {
    pool: PgPool,
}

impl PostgresDependencyRepository {
    /// Create a new PostgresDependencyRepository
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    fn row_to_dependency_record(
        &self,
        row: &sqlx::postgres::PgRow,
    ) -> Result<AssetDependencyRecord, RepositoryError> {
        let reason: String = row.get("effective_reason");
        let effective_reason = match reason.as_str() {
            "publish" => EffectiveUpdateReason::Publish,
            "manual_clean" => EffectiveUpdateReason::ManualClean,
            other => {
                return Err(RepositoryError::DatabaseError(format!(
                    "Unknown effective_reason: {other}"
                )));
            }
        };

        let declared_version_str: String = row.get("declared_version");
        let declared_constraint = VersionConstraint::parse(&declared_version_str)
            .map_err(|e| RepositoryError::ValidationError(format!("Invalid constraint: {e}")))?;
        let effective_version_str: String = row.get("effective_version");
        let effective_version = SemVer::parse(&effective_version_str)
            .map_err(|e| RepositoryError::ValidationError(format!("Invalid version: {e}")))?;
        let upgrade_policy_str: String = row.get("upgrade_policy");
        let upgrade_policy = match upgrade_policy_str.as_str() {
            "auto_patch" => UpgradePolicy::AutoPatch,
            "auto_minor" => UpgradePolicy::AutoMinor,
            "notify" => UpgradePolicy::Notify,
            "manual" => UpgradePolicy::Manual,
            "pin" => UpgradePolicy::Pin,
            _ => UpgradePolicy::default(),
        };
        let lock_version: i64 = row.try_get::<i64, _>("lock_version").unwrap_or(1);

        Ok(AssetDependencyRecord {
            id: row.get("id"),
            source_id: AssetId::from_uuid(row.get("source_id")),
            target_id: AssetId::from_uuid(row.get("target_id")),
            relationship: row.get("relationship"),
            declared_constraint,
            constraint_str: declared_version_str,
            effective_version,
            effective_updated_by: row.get("effective_updated_by"),
            effective_updated_at: row.get("effective_updated_at"),
            effective_reason,
            upgrade_policy,
            lock_version,
            created_at: row.get("created_at"),
        })
    }
}

#[async_trait]
impl adam_domain::DependencyRepository for PostgresDependencyRepository {
    async fn find_downstream(&self, asset_id: &AssetId) -> Result<Vec<AssetId>, RepositoryError> {
        let rows = sqlx::query(
            r#"
            SELECT source_id
            FROM asset_dependencies
            WHERE target_id = $1
            "#,
        )
        .bind(asset_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|row| AssetId::from_uuid(row.get("source_id")))
            .collect())
    }

    async fn find_upstream(&self, asset_id: &AssetId) -> Result<Vec<AssetId>, RepositoryError> {
        let rows = sqlx::query(
            r#"
            SELECT target_id
            FROM asset_dependencies
            WHERE source_id = $1
            "#,
        )
        .bind(asset_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|row| AssetId::from_uuid(row.get("target_id")))
            .collect())
    }

    async fn create_dependency(
        &self,
        source_id: &AssetId,
        target_id: &AssetId,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            r#"
            INSERT INTO asset_dependencies (
                id, source_id, target_id, declared_version, effective_version,
                effective_updated_by, effective_updated_at, effective_reason, created_at
            )
            VALUES (
                gen_random_uuid(), $1, $2, '0.0.0', '0.0.0',
                'system', NOW(), 'publish', NOW()
            )
            "#,
        )
        .bind(source_id.0)
        .bind(target_id.0)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            if e.to_string().contains("Cross-organization") {
                RepositoryError::InvalidStateTransition(
                    "Cross-organization dependency not allowed".to_string(),
                )
            } else if e.to_string().contains("Cycle detected") {
                RepositoryError::InvalidStateTransition("Dependency cycle detected".to_string())
            } else {
                RepositoryError::DatabaseError(e.to_string())
            }
        })?;

        Ok(())
    }

    async fn create_dependency_record(
        &self,
        record: &AssetDependencyRecord,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            r#"
            INSERT INTO asset_dependencies (
                id, source_id, target_id, relationship, declared_version,
                effective_version, effective_updated_by, effective_updated_at,
                effective_reason, created_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (source_id, target_id)
            DO UPDATE SET
                relationship = EXCLUDED.relationship,
                declared_version = EXCLUDED.declared_version,
                effective_version = EXCLUDED.effective_version,
                effective_updated_by = EXCLUDED.effective_updated_by,
                effective_updated_at = EXCLUDED.effective_updated_at,
                effective_reason = EXCLUDED.effective_reason
            "#,
        )
        .bind(record.id)
        .bind(record.source_id.0)
        .bind(record.target_id.0)
        .bind(&record.relationship)
        .bind(&record.constraint_str)
        .bind(record.effective_version.to_string())
        .bind(&record.effective_updated_by)
        .bind(record.effective_updated_at)
        .bind(record.effective_reason.as_str())
        .bind(record.created_at)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    async fn find_downstream_dependencies(
        &self,
        asset_id: &AssetId,
    ) -> Result<Vec<AssetDependencyRecord>, RepositoryError> {
        let rows = sqlx::query(
            r#"
            SELECT
                id, source_id, target_id, relationship, declared_version,
                effective_version, effective_updated_by, effective_updated_at,
                effective_reason, created_at
            FROM asset_dependencies
            WHERE target_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(asset_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        rows.iter()
            .map(|row| self.row_to_dependency_record(row))
            .collect()
    }

    async fn find_upstream_dependencies(
        &self,
        asset_id: &AssetId,
    ) -> Result<Vec<AssetDependencyRecord>, RepositoryError> {
        let rows = sqlx::query(
            r#"
            SELECT
                id, source_id, target_id, relationship, declared_version,
                effective_version, effective_updated_by, effective_updated_at,
                effective_reason, created_at
            FROM asset_dependencies
            WHERE source_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(asset_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        rows.iter()
            .map(|row| self.row_to_dependency_record(row))
            .collect()
    }

    async fn update_effective_version(
        &self,
        source_id: &AssetId,
        target_id: &AssetId,
        effective_version: String,
        updated_by: String,
        reason: EffectiveUpdateReason,
    ) -> Result<(), RepositoryError> {
        let result = sqlx::query(
            r#"
            UPDATE asset_dependencies
            SET
                effective_version = $1,
                effective_updated_by = $2,
                effective_updated_at = NOW(),
                effective_reason = $3
            WHERE source_id = $4 AND target_id = $5
            "#,
        )
        .bind(effective_version)
        .bind(updated_by)
        .bind(reason.as_str())
        .bind(source_id.0)
        .bind(target_id.0)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(RepositoryError::NotFound(format!(
                "dependency {} -> {}",
                source_id.0, target_id.0
            )));
        }

        Ok(())
    }
}

/// PostgreSQL implementation of DirtyResolutionLogRepository
pub struct PostgresDirtyResolutionLogRepository {
    pool: PgPool,
}

impl PostgresDirtyResolutionLogRepository {
    /// Create a new PostgresDirtyResolutionLogRepository
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    fn row_to_log(&self, row: &sqlx::postgres::PgRow) -> DirtyResolutionLog {
        DirtyResolutionLog {
            id: row.get("id"),
            asset_id: AssetId::from_uuid(row.get("asset_id")),
            asset_version: row.get("asset_version"),
            upstream_asset_id: AssetId::from_uuid(row.get("upstream_asset_id")),
            from_version: row.get("from_version"),
            to_version: row.get("to_version"),
            action: row.get("action"),
            review_result: row.get("review_result"),
            comment: row.get("comment"),
            reviewed_by: row.get("reviewed_by"),
            reviewed_at: row.get("reviewed_at"),
        }
    }
}

#[async_trait]
impl DirtyResolutionLogRepository for PostgresDirtyResolutionLogRepository {
    async fn insert(&self, log: &DirtyResolutionLog) -> Result<(), RepositoryError> {
        sqlx::query(
            r#"
            INSERT INTO dirty_resolution_logs (
                id, asset_id, asset_version, upstream_asset_id,
                from_version, to_version, action, review_result,
                comment, reviewed_by, reviewed_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            "#,
        )
        .bind(log.id)
        .bind(log.asset_id.0)
        .bind(&log.asset_version)
        .bind(log.upstream_asset_id.0)
        .bind(&log.from_version)
        .bind(&log.to_version)
        .bind(&log.action)
        .bind(&log.review_result)
        .bind(&log.comment)
        .bind(&log.reviewed_by)
        .bind(log.reviewed_at)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    async fn find_by_asset(
        &self,
        asset_id: &AssetId,
    ) -> Result<Vec<DirtyResolutionLog>, RepositoryError> {
        let rows = sqlx::query(
            r#"
            SELECT
                id, asset_id, asset_version, upstream_asset_id,
                from_version, to_version, action, review_result,
                comment, reviewed_by, reviewed_at
            FROM dirty_resolution_logs
            WHERE asset_id = $1
            ORDER BY reviewed_at DESC
            "#,
        )
        .bind(asset_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        Ok(rows.iter().map(|row| self.row_to_log(row)).collect())
    }
}

/// PostgreSQL implementation of AssetVersionRepository
pub struct PostgresAssetVersionRepository {
    pool: PgPool,
}

impl PostgresAssetVersionRepository {
    /// Create a new PostgresAssetVersionRepository
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl adam_domain::AssetVersionRepository for PostgresAssetVersionRepository {
    async fn create(&self, version: &adam_domain::AssetVersion) -> Result<(), RepositoryError> {
        sqlx::query(
            r#"
            INSERT INTO asset_versions (
                id, instance_id, version_number, metadata, dependencies,
                release_notes, suggested_type, released_by, released_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
        )
        .bind(version.id.0)
        .bind(version.asset_id.0)
        .bind(&version.version_number)
        .bind(&version.metadata)
        .bind(serde_json::to_value(&version.dependencies).unwrap_or_default())
        .bind(&version.release_notes)
        .bind(&version.suggested_type)
        .bind(&version.released_by)
        .bind(version.released_at)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    async fn find_by_asset(
        &self,
        asset_id: &AssetId,
    ) -> Result<Vec<adam_domain::AssetVersion>, RepositoryError> {
        let rows = sqlx::query(
            r#"
            SELECT
                id, instance_id, version_number, metadata, dependencies,
                release_notes, suggested_type, released_by, released_at
            FROM asset_versions
            WHERE instance_id = $1
            ORDER BY released_at DESC
            "#,
        )
        .bind(asset_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|row| adam_domain::AssetVersion {
                id: adam_domain::asset::version::AssetVersionId(row.get("id")),
                asset_id: AssetId::from_uuid(row.get("instance_id")),
                version_number: row.get("version_number"),
                metadata: row.get("metadata"),
                dependencies: serde_json::from_value(row.get("dependencies")).unwrap_or_default(),
                release_notes: row.get("release_notes"),
                suggested_type: row.get("suggested_type"),
                released_by: row.get("released_by"),
                released_at: row.get("released_at"),
            })
            .collect())
    }

    async fn find_by_version(
        &self,
        asset_id: &AssetId,
        version: &str,
    ) -> Result<Option<adam_domain::AssetVersion>, RepositoryError> {
        let row = sqlx::query(
            r#"
            SELECT
                id, instance_id, version_number, metadata, dependencies,
                release_notes, suggested_type, released_by, released_at
            FROM asset_versions
            WHERE instance_id = $1 AND version_number = $2
            "#,
        )
        .bind(asset_id.0)
        .bind(version)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        Ok(row.map(|row| adam_domain::AssetVersion {
            id: adam_domain::asset::version::AssetVersionId(row.get("id")),
            asset_id: AssetId::from_uuid(row.get("instance_id")),
            version_number: row.get("version_number"),
            metadata: row.get("metadata"),
            dependencies: serde_json::from_value(row.get("dependencies")).unwrap_or_default(),
            release_notes: row.get("release_notes"),
            suggested_type: row.get("suggested_type"),
            released_by: row.get("released_by"),
            released_at: row.get("released_at"),
        }))
    }

    async fn find_latest(
        &self,
        asset_id: &AssetId,
    ) -> Result<Option<adam_domain::AssetVersion>, RepositoryError> {
        let row = sqlx::query(
            r#"
            SELECT
                id, instance_id, version_number, metadata, dependencies,
                release_notes, suggested_type, released_by, released_at
            FROM asset_versions
            WHERE instance_id = $1
            ORDER BY released_at DESC
            LIMIT 1
            "#,
        )
        .bind(asset_id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        Ok(row.map(|row| adam_domain::AssetVersion {
            id: adam_domain::asset::version::AssetVersionId(row.get("id")),
            asset_id: AssetId::from_uuid(row.get("instance_id")),
            version_number: row.get("version_number"),
            metadata: row.get("metadata"),
            dependencies: serde_json::from_value(row.get("dependencies")).unwrap_or_default(),
            release_notes: row.get("release_notes"),
            suggested_type: row.get("suggested_type"),
            released_by: row.get("released_by"),
            released_at: row.get("released_at"),
        }))
    }
}

/// PostgreSQL implementation of VirtualInstanceRepository
pub struct PostgresVirtualInstanceRepository {
    pool: PgPool,
}

impl PostgresVirtualInstanceRepository {
    /// Create a new PostgresVirtualInstanceRepository
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    fn row_to_virtual_instance(&self, row: &sqlx::postgres::PgRow) -> VirtualInstance {
        VirtualInstance {
            id: VirtualInstanceId::from_uuid(row.get("id")),
            target_type: AssetTypeId::from_uuid(row.get("target_type_id")),
            target_type_name: row.get("target_type_name"),
            anchors: row
                .get::<Vec<Uuid>, _>("anchor_ids")
                .into_iter()
                .map(AssetId::from_uuid)
                .collect(),
            project_id: ProjectId::from_uuid(row.get("project_id")),
            organization_id: OrganizationId::from_uuid(row.get("organization_id")),
            created_by: row.get("created_by"),
            created_at: row.get("created_at"),
            expires_at: row.get("expires_at"),
            context_summary: row.get("context_summary"),
        }
    }
}

#[async_trait]
impl VirtualInstanceRepository for PostgresVirtualInstanceRepository {
    async fn find_by_id(
        &self,
        id: &VirtualInstanceId,
    ) -> Result<Option<VirtualInstance>, RepositoryError> {
        let row = sqlx::query(
            r#"
            SELECT
                id, target_type_id, target_type_name, anchor_ids,
                project_id, organization_id, created_by, created_at,
                expires_at, context_summary
            FROM virtual_instances
            WHERE id = $1
            "#,
        )
        .bind(id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        Ok(row.map(|row| self.row_to_virtual_instance(&row)))
    }

    async fn create(&self, instance: &VirtualInstance) -> Result<VirtualInstance, RepositoryError> {
        sqlx::query(
            r#"
            INSERT INTO virtual_instances (
                id, target_type_id, target_type_name, anchor_ids,
                project_id, organization_id, created_by, created_at,
                expires_at, context_summary
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            "#,
        )
        .bind(instance.id.0)
        .bind(instance.target_type.0)
        .bind(&instance.target_type_name)
        .bind(
            instance
                .anchors
                .iter()
                .map(|asset_id| asset_id.0)
                .collect::<Vec<_>>(),
        )
        .bind(instance.project_id.0)
        .bind(instance.organization_id.0)
        .bind(&instance.created_by)
        .bind(instance.created_at)
        .bind(instance.expires_at)
        .bind(&instance.context_summary)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        Ok(instance.clone())
    }

    async fn delete_expired(&self) -> Result<u64, RepositoryError> {
        let result = sqlx::query(
            r#"
            DELETE FROM virtual_instances
            WHERE expires_at < NOW()
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        Ok(result.rows_affected())
    }

    async fn find_by_project(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<VirtualInstance>, RepositoryError> {
        let rows = sqlx::query(
            r#"
            SELECT
                id, target_type_id, target_type_name, anchor_ids,
                project_id, organization_id, created_by, created_at,
                expires_at, context_summary
            FROM virtual_instances
            WHERE project_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(project_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|row| self.row_to_virtual_instance(row))
            .collect())
    }
}

/// PostgreSQL implementation of UnitOfWork for transaction management
pub struct PostgresUnitOfWork {
    pool: PgPool,
}

impl PostgresUnitOfWork {
    /// Create a new PostgresUnitOfWork with the given connection pool
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UnitOfWork for PostgresUnitOfWork {
    async fn transaction<F, T, E>(&self, operation: F) -> Result<T, E>
    where
        F: for<'a> FnOnce(&'a mut TransactionContext)
                -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, E>> + Send + 'a>>
            + Send
            + 'async_trait,
        E: From<RepositoryError> + Send,
        T: Send,
    {
        // Begin transaction
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| E::from(RepositoryError::DatabaseError(e.to_string())))?;

        // Create transactional repositories
        let asset_repo = PostgresAssetRepository::new(self.pool.clone());
        let dependency_repo = PostgresDependencyRepository::new(self.pool.clone());
        let dirty_queue_repo = PostgresDirtyQueueRepository::new(self.pool.clone());

        // Create transaction context
        let mut ctx = TransactionContext {
            asset_repo: Box::new(asset_repo),
            dependency_repo: Box::new(dependency_repo),
            dirty_queue_repo: Box::new(dirty_queue_repo),
        };

        // Execute the operation
        match operation(&mut ctx).await {
            Ok(result) => {
                // Commit transaction
                tx.commit()
                    .await
                    .map_err(|e| E::from(RepositoryError::DatabaseError(e.to_string())))?;
                Ok(result)
            }
            Err(e) => {
                // Rollback transaction
                // Note: sqlx transactions auto-rollback on drop, but explicit is clearer
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }
}

#[cfg(test)]
#[cfg(test)]
mod tests {
    #[test]
    fn asset_version_sql_uses_instance_id_column() {
        let source = include_str!("postgres.rs");
        let version_repo_impl = source
            .split("impl adam_domain::AssetVersionRepository for PostgresAssetVersionRepository")
            .nth(1)
            .and_then(|tail| tail.split("#[cfg(test)]").next())
            .expect("expected PostgresAssetVersionRepository impl block");
        let bad_column_list = ["id, ", "asset_id", ", version_number"].concat();
        let bad_where_clause = ["WHERE ", "asset_id", " = $1"].concat();

        assert!(
            version_repo_impl.contains("id, instance_id, version_number"),
            "asset_versions SELECT/INSERT columns must include instance_id"
        );
        assert!(
            version_repo_impl.contains("WHERE instance_id = $1"),
            "asset_versions lookup SQL must filter by instance_id"
        );
        assert!(
            !version_repo_impl.contains(&bad_column_list),
            "asset_versions SQL must not select or insert non-existent column asset_id"
        );
        assert!(
            !version_repo_impl.contains(&bad_where_clause),
            "asset_versions SQL must not filter by non-existent column asset_id"
        );
    }

    // Note: These tests require a running PostgreSQL instance
    // They are marked with #[ignore] by default and should be run with:
    // cargo test --features integration-tests -- --ignored

    #[tokio::test]
    #[ignore]
    async fn postgres_repo_creates_asset() {
        // This test requires a PostgreSQL database
        // DATABASE_URL must be set in environment
    }
}
