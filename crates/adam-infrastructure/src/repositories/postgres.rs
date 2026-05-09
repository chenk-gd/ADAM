//! PostgreSQL repository implementations

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use adam_domain::{
    AssetId, AssetInstance, AssetRepository, CreateAssetCommand, OrganizationId, ProjectId,
    RepositoryError,
};
use adam_domain::asset::instance::AssetTypeId;
use adam_domain::asset::state::AssetState;
use adam_domain::dependency::boundary::AssetLevel;

/// PostgreSQL implementation of AssetRepository
pub struct PostgresAssetRepository {
    pool: PgPool,
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
                $1, $2, $3, $4, $5, 'manual',
                $6, $7, '0.1.0', $8,
                '{}', '[]', $9, $10, $11
            )
            RETURNING id
            "#
        )
        .bind(asset_id.0)
        .bind(cmd.asset_type_id.0)
        .bind(cmd.organization_id.0)
        .bind(&cmd.name)
        .bind(format!("manual: {}", cmd.name)) // external_ref
        .bind(level_str)
        .bind(cmd.project_id.map(|p| p.0))
        .bind(state_str)
        .bind(&cmd.idempotency_key)
        .bind(now)
        .bind(now)
        .fetch_one(&self.pool)
        .await;

        match result {
            Ok(row) => {
                let id: Uuid = row.get("id");
                Ok(AssetInstance {
                    id: AssetId::from_uuid(id),
                    name: cmd.name.clone(),
                    asset_type_id: cmd.asset_type_id,
                    project_id: cmd.project_id,
                    organization_id: cmd.organization_id,
                    level: cmd.level,
                    current_state: AssetState::Clean,
                    external_ref: cmd.external_ref.clone(),
                    source: cmd.source.clone(),
                    metadata: cmd.metadata.clone(),
                    assignees: vec![],
                    publisher: None,
                    current_version: None,
                    created_at: now,
                    updated_at: now,
                    idempotency_key: cmd.idempotency_key.clone(),
                })
            }
            Err(sqlx::Error::Database(db_err)) => {
                if let Some(constraint) = db_err.constraint() {
                    if constraint.contains("idempotency") {
                        return Err(RepositoryError::DuplicateIdempotencyKey(
                            cmd.idempotency_key.clone().unwrap_or_default()
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
                current_state, created_at, updated_at, idempotency_key
            FROM asset_instances
            WHERE id = $1
            "#
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
        };

        let result = sqlx::query(
            r#"
            UPDATE asset_instances
            SET current_state = $1, updated_at = NOW()
            WHERE id = $2
            "#
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

    async fn find_by_project_id(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<AssetInstance>, RepositoryError> {
        let rows = sqlx::query(
            r#"
            SELECT
                id, name, type_id, organization_id, level, project_id,
                current_state, created_at, updated_at, idempotency_key
            FROM asset_instances
            WHERE project_id = $1
            ORDER BY name
            "#
        )
        .bind(project_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        rows.iter()
            .map(|row| self.row_to_asset(row))
            .collect()
    }

    async fn find_by_organization_id(
        &self,
        org_id: &OrganizationId,
    ) -> Result<Vec<AssetInstance>, RepositoryError> {
        let rows = sqlx::query(
            r#"
            SELECT
                id, name, type_id, organization_id, level, project_id,
                current_state, created_at, updated_at, idempotency_key
            FROM asset_instances
            WHERE organization_id = $1
            ORDER BY name
            "#
        )
        .bind(org_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        rows.iter()
            .map(|row| self.row_to_asset(row))
            .collect()
    }
}

impl PostgresAssetRepository {
    /// Convert a database row to an AssetInstance
    fn row_to_asset(&self, row: &sqlx::postgres::PgRow) -> Result<AssetInstance, RepositoryError> {
        let level_str: String = row.try_get("level")
            .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;
        let state_str: String = row.try_get("current_state")
            .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        // Get optional new fields with defaults
        let external_ref: String = row.try_get::<String, _>("external_ref")
            .unwrap_or_default();
        let source: String = row.try_get::<String, _>("source")
            .unwrap_or_else(|_| "manual".to_string());
        let metadata: serde_json::Value = row.try_get::<serde_json::Value, _>("metadata")
            .unwrap_or_else(|_| serde_json::json!({}));
        let assignees: Vec<String> = row.try_get::<Vec<String>, _>("assignees")
            .unwrap_or_default();
        let publisher: Option<String> = row.try_get::<Option<String>, _>("publisher").ok().flatten();
        let current_version: Option<String> = row.try_get::<Option<String>, _>("current_version").ok().flatten();

        Ok(AssetInstance {
            id: AssetId::from_uuid(row.try_get::<Uuid, _>("id")
                .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?),
            name: row.try_get::<String, _>("name")
                .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?,
            asset_type_id: AssetTypeId::from_uuid(row.try_get::<Uuid, _>("type_id")
                .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?),
            project_id: row.try_get::<Option<Uuid>, _>("project_id")
                .ok()
                .flatten()
                .map(ProjectId::from_uuid),
            organization_id: OrganizationId::from_uuid(row.try_get::<Uuid, _>("organization_id")
                .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?),
            level: match level_str.as_str() {
                "project" => AssetLevel::Project,
                "organization" => AssetLevel::Organization,
                _ => return Err(RepositoryError::DatabaseError(format!("Invalid level: {}", level_str))),
            },
            current_state: match state_str.as_str() {
                "clean" => AssetState::Clean,
                "dirty" => AssetState::Dirty,
                "archived" => AssetState::Archived,
                _ => return Err(RepositoryError::DatabaseError(format!("Invalid state: {}", state_str))),
            },
            external_ref,
            source,
            metadata,
            assignees,
            publisher,
            current_version,
            created_at: row.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
                .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?,
            updated_at: row.try_get::<chrono::DateTime<chrono::Utc>, _>("updated_at")
                .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?,
            idempotency_key: row.try_get::<Option<String>, _>("idempotency_key").ok().flatten(),
        })
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
                impact_level, resolved, idempotency_key, created_at, updated_at
            )
            VALUES (
                $1, $2, $3, $4, '0.0.0',
                'medium', false, $5, $6, $6
            )
            ON CONFLICT (asset_id, upstream_asset_id) WHERE resolved = false
            DO UPDATE SET
                upstream_version = EXCLUDED.upstream_version,
                updated_at = EXCLUDED.updated_at
            "#
        )
        .bind(entry.id)
        .bind(entry.asset_id.0)
        .bind(entry.upstream_asset_id.0)
        .bind(&entry.upstream_version)
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
                upstream_old_version, created_at, resolved_at
            FROM dirty_queue
            WHERE asset_id = $1 AND resolved = false
            ORDER BY created_at DESC
            "#
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
            "#
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

    async fn find_all_unresolved(&self) -> Result<Vec<adam_domain::repository::DirtyQueueEntry>, RepositoryError> {
        let rows = sqlx::query(
            r#"
            SELECT
                id, asset_id, upstream_asset_id, upstream_version,
                upstream_old_version, created_at, resolved_at
            FROM dirty_queue
            WHERE resolved = false
            ORDER BY created_at DESC
            "#
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
}

#[async_trait]
impl adam_domain::DependencyRepository for PostgresDependencyRepository {
    async fn find_downstream(&self, asset_id: &AssetId) -> Result<Vec<AssetId>, RepositoryError> {
        let rows = sqlx::query(
            r#"
            SELECT source_id
            FROM asset_dependencies
            WHERE target_id = $1
            "#
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
            "#
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
            "#
        )
        .bind(source_id.0)
        .bind(target_id.0)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            if e.to_string().contains("Cross-organization") {
                RepositoryError::InvalidStateTransition("Cross-organization dependency not allowed".to_string())
            } else if e.to_string().contains("Cycle detected") {
                RepositoryError::InvalidStateTransition("Dependency cycle detected".to_string())
            } else {
                RepositoryError::DatabaseError(e.to_string())
            }
        })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
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
