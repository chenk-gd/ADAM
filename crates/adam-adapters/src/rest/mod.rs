//! ADAM REST API handlers and routing

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use adam_domain::{
    AssetId, AssetInstance, AssetRepository, AssetState, AssetTypeId, CreateAssetCommand,
    DependencyRepository, DirtyQueueRepository, OrganizationId, ProjectId, RepositoryError,
};

// ============================================================================
// Error Types
// ============================================================================

/// REST API error responses
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Asset not found")]
    NotFound,
    #[error("Invalid request: {0}")]
    BadRequest(String),
    #[error("Repository error: {0}")]
    Repository(#[from] RepositoryError),
    #[error("Internal server error")]
    Internal,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, self.to_string()),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::Repository(RepositoryError::NotFound(_)) => {
                (StatusCode::NOT_FOUND, "Asset not found".to_string())
            }
            ApiError::Repository(RepositoryError::DuplicateIdempotencyKey(_)) => (
                StatusCode::CONFLICT,
                "Asset with this idempotency key already exists".to_string(),
            ),
            ApiError::Repository(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            ApiError::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            ),
        };

        let body = Json(ErrorResponse { error: message });
        (status, body).into_response()
    }
}

/// Error response body
#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

// ============================================================================
// Request/Response DTOs
// ============================================================================

/// Create asset request
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateAssetRequest {
    pub name: String,
    pub asset_type_id: Uuid,
    pub organization_id: Uuid,
    pub project_id: Option<Uuid>,
    pub level: AssetLevelDto,
    pub idempotency_key: Option<String>,
}

/// Asset level DTO for serialization
#[derive(Debug, Deserialize, Serialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum AssetLevelDto {
    Project,
    Organization,
}

impl From<AssetLevelDto> for adam_domain::dependency::boundary::AssetLevel {
    fn from(dto: AssetLevelDto) -> Self {
        match dto {
            AssetLevelDto::Project => adam_domain::dependency::boundary::AssetLevel::Project,
            AssetLevelDto::Organization => {
                adam_domain::dependency::boundary::AssetLevel::Organization
            }
        }
    }
}

/// Asset response DTO
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AssetResponse {
    pub id: Uuid,
    pub name: String,
    pub asset_type_id: Uuid,
    pub organization_id: Uuid,
    pub project_id: Option<Uuid>,
    pub level: AssetLevelDto,
    pub current_state: AssetStateDto,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Asset state DTO
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetStateDto {
    Clean,
    Dirty,
    Archived,
}

impl From<AssetState> for AssetStateDto {
    fn from(state: AssetState) -> Self {
        match state {
            AssetState::Clean => AssetStateDto::Clean,
            AssetState::Dirty => AssetStateDto::Dirty,
            AssetState::Archived => AssetStateDto::Archived,
        }
    }
}

impl From<AssetInstance> for AssetResponse {
    fn from(asset: AssetInstance) -> Self {
        AssetResponse {
            id: asset.id.0,
            name: asset.name,
            asset_type_id: asset.asset_type_id.0,
            organization_id: asset.organization_id.0,
            project_id: asset.project_id.map(|p| p.0),
            level: match asset.level {
                adam_domain::dependency::boundary::AssetLevel::Project => AssetLevelDto::Project,
                adam_domain::dependency::boundary::AssetLevel::Organization => {
                    AssetLevelDto::Organization
                }
            },
            current_state: asset.current_state.into(),
            created_at: asset.created_at,
            updated_at: asset.updated_at,
        }
    }
}

/// Query parameters for listing assets
#[derive(Debug, Deserialize)]
pub struct ListAssetsQuery {
    pub project_id: Option<Uuid>,
    pub organization_id: Option<Uuid>,
}

/// Publish version request
#[derive(Debug, Deserialize)]
pub struct PublishRequest {
    pub version: String,
}

/// Publish response
#[derive(Debug, Serialize)]
pub struct PublishResponse {
    pub affected_assets: Vec<Uuid>,
}

/// Resolve dirty request
#[derive(Debug, Deserialize)]
pub struct ResolveRequest {
    pub resolved_version: String,
}

// ============================================================================
// Application State
// ============================================================================

/// Shared application state for handlers
#[derive(Clone)]
pub struct AppState {
    pub asset_repo: Arc<dyn AssetRepository>,
    pub dependency_repo: Arc<dyn DependencyRepository>,
    pub dirty_repo: Arc<dyn DirtyQueueRepository>,
}

// ============================================================================
// Handlers
// ============================================================================

/// Create a new asset
pub async fn create_asset(
    State(state): State<AppState>,
    Json(req): Json<CreateAssetRequest>,
) -> Result<(StatusCode, Json<AssetResponse>), ApiError> {
    let cmd = CreateAssetCommand {
        name: req.name,
        asset_type_id: AssetTypeId::from_uuid(req.asset_type_id),
        project_id: req.project_id.map(ProjectId::from_uuid),
        organization_id: OrganizationId::from_uuid(req.organization_id),
        level: req.level.into(),
        idempotency_key: req.idempotency_key,
    };

    let asset = state
        .asset_repo
        .create(&cmd)
        .await
        .map_err(ApiError::from)?;

    Ok((StatusCode::CREATED, Json(asset.into())))
}

/// Get asset by ID
pub async fn get_asset(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<AssetResponse>, ApiError> {
    let asset_id = AssetId::from_uuid(id);
    let asset = state
        .asset_repo
        .find_by_id(&asset_id)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;

    Ok(Json(asset.into()))
}

/// List assets with optional filters
pub async fn list_assets(
    State(state): State<AppState>,
    Query(query): Query<ListAssetsQuery>,
) -> Result<Json<Vec<AssetResponse>>, ApiError> {
    let assets = if let Some(project_id) = query.project_id {
        let pid = ProjectId::from_uuid(project_id);
        state
            .asset_repo
            .find_by_project_id(&pid)
            .await
            .map_err(ApiError::from)?
    } else if let Some(org_id) = query.organization_id {
        let oid = OrganizationId::from_uuid(org_id);
        state
            .asset_repo
            .find_by_organization_id(&oid)
            .await
            .map_err(ApiError::from)?
    } else {
        // Without filters, return empty list (or could return all)
        vec![]
    };

    Ok(Json(assets.into_iter().map(AssetResponse::from).collect()))
}

/// Publish a new version (triggers dirty propagation)
pub async fn publish_asset(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<PublishRequest>,
) -> Result<(StatusCode, Json<PublishResponse>), ApiError> {
    use adam_application::services::state_propagator::StatePropagator;

    let asset_id = AssetId::from_uuid(id);
    let propagator = StatePropagator::new();

    let affected = propagator
        .on_asset_published(
            &asset_id,
            &req.version,
            state.asset_repo.as_ref(),
            state.dependency_repo.as_ref(),
            state.dirty_repo.as_ref(),
        )
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    Ok((
        StatusCode::OK,
        Json(PublishResponse {
            affected_assets: affected.into_iter().map(|id| id.0).collect(),
        }),
    ))
}

/// Resolve dirty state
pub async fn resolve_dirty(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(_req): Json<ResolveRequest>,
) -> Result<StatusCode, ApiError> {
    let asset_id = AssetId::from_uuid(id);

    // Update asset state to Clean
    state
        .asset_repo
        .update_state(&asset_id, AssetState::Clean)
        .await
        .map_err(ApiError::from)?;

    Ok(StatusCode::NO_CONTENT)
}

// ============================================================================
// Router
// ============================================================================

/// Create the REST API router
pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/assets", post(create_asset).get(list_assets))
        .route("/api/v1/assets/{id}", get(get_asset))
        .route("/api/v1/assets/{id}/publish", post(publish_asset))
        .route("/api/v1/assets/{id}/resolve", post(resolve_dirty))
        .with_state(state)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Method, Request};
    use tower::ServiceExt;

    fn create_test_state() -> AppState {
        AppState {
            asset_repo: Arc::new(adam_domain::InMemoryAssetRepository::new()),
            dependency_repo: Arc::new(InMemoryDependencyRepository::new()),
            dirty_repo: Arc::new(adam_domain::InMemoryDirtyQueueRepository::new()),
        }
    }

    use async_trait::async_trait;

    /// Simple in-memory dependency repo for testing
    struct InMemoryDependencyRepository {
        data: std::sync::Mutex<
            std::collections::HashMap<AssetId, Vec<AssetId>>, // asset_id -> downstream assets
        >,
    }

    impl InMemoryDependencyRepository {
        fn new() -> Self {
            Self {
                data: std::sync::Mutex::new(std::collections::HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl DependencyRepository for InMemoryDependencyRepository {
        async fn find_downstream(
            &self,
            _asset_id: &AssetId,
        ) -> Result<Vec<AssetId>, RepositoryError> {
            Ok(vec![])
        }

        async fn find_upstream(
            &self,
            _asset_id: &AssetId,
        ) -> Result<Vec<AssetId>, RepositoryError> {
            Ok(vec![])
        }

        async fn create_dependency(
            &self,
            source: &AssetId,
            target: &AssetId,
        ) -> Result<(), RepositoryError> {
            let mut data = self.data.lock().unwrap();
            data.entry(*target).or_default().push(*source);
            Ok(())
        }
    }

    #[tokio::test]
    async fn create_asset_endpoint_returns_201() {
        let state = create_test_state();
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/assets")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "name": "Test Asset",
                            "asset_type_id": Uuid::new_v4(),
                            "organization_id": Uuid::new_v4(),
                            "level": "project",
                            "project_id": Uuid::new_v4(),
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);

        // Verify response body
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let asset: AssetResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(asset.name, "Test Asset");
    }

    #[tokio::test]
    async fn create_asset_with_invalid_level_returns_422() {
        let state = create_test_state();
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/assets")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"name": "Test", "asset_type_id": "00000000-0000-0000-0000-000000000001", "organization_id": "00000000-0000-0000-0000-000000000002", "level": "invalid"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Axum returns 422 for JSON deserialization errors
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn get_asset_returns_200_for_existing() {
        let state = create_test_state();

        // First create an asset
        let org_id = OrganizationId::new();
        let type_id = AssetTypeId::new();
        let cmd = CreateAssetCommand {
            name: "Existing Asset".to_string(),
            asset_type_id: type_id,
            project_id: None,
            organization_id: org_id,
            level: adam_domain::dependency::boundary::AssetLevel::Organization,
            idempotency_key: None,
        };
        let asset = state.asset_repo.create(&cmd).await.unwrap();

        let app = create_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/v1/assets/{}", asset.id.0))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let fetched: AssetResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(fetched.name, "Existing Asset");
    }

    #[tokio::test]
    async fn get_asset_returns_404_for_nonexistent() {
        let state = create_test_state();
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/v1/assets/{}", Uuid::new_v4()))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn list_assets_returns_empty_when_no_filter() {
        let state = create_test_state();
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/assets")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let assets: Vec<AssetResponse> = serde_json::from_slice(&body).unwrap();
        assert!(assets.is_empty());
    }

    #[tokio::test]
    async fn list_assets_by_organization_returns_assets() {
        let state = create_test_state();
        let org_id = OrganizationId::new();

        // Create an asset
        let cmd = CreateAssetCommand {
            name: "Org Asset".to_string(),
            asset_type_id: AssetTypeId::new(),
            project_id: None,
            organization_id: org_id,
            level: adam_domain::dependency::boundary::AssetLevel::Organization,
            idempotency_key: None,
        };
        state.asset_repo.create(&cmd).await.unwrap();

        let app = create_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/v1/assets?organization_id={}", org_id.0))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let assets: Vec<AssetResponse> = serde_json::from_slice(&body).unwrap();
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].name, "Org Asset");
    }
}
