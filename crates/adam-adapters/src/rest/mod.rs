//! ADAM REST API handlers and routing

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, Method, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_http::{
    cors::CorsLayer,
    trace::{DefaultOnFailure, DefaultOnRequest, DefaultOnResponse, TraceLayer},
};
use tracing::Level;
use uuid::Uuid;

use adam_application::services::state_propagator::{StatePropagationError, StatePropagator};
use adam_domain::{
    AssetId, AssetInstance, AssetRepository, AssetState, AssetTypeId, AssetTypeRepository,
    AuthPrincipal, AuthorizationError, CreateAssetCommand, DependencyRepository,
    DirtyQueueRepository, OrganizationId, ProjectId, RepositoryError, Role,
};

// ============================================================================
// Authentication Types
// ============================================================================

/// Authentication context passed to handlers
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub principal: AuthPrincipal,
}

/// Auth extraction error
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("Missing authorization header")]
    MissingHeader,
    #[error("Invalid authorization format")]
    InvalidFormat,
    #[error("Invalid token")]
    InvalidToken,
}

impl IntoResponse for AuthError {
    fn into_response(self) -> axum::response::Response {
        let status = StatusCode::UNAUTHORIZED;
        let body = Json(ErrorResponse {
            error: self.to_string(),
        });
        (status, body).into_response()
    }
}

/// Extract AuthPrincipal from Authorization header
/// For MVP: parses "Bearer {org_id}:{user_id}:{role1,role2}:{project1,project2}" format
/// Roles: SystemAdmin, OrgAdmin, ProjectAdmin, Developer, Reader, AiAgent
pub fn extract_auth_principal(headers: &HeaderMap) -> Result<AuthPrincipal, AuthError> {
    let auth_header = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .ok_or(AuthError::MissingHeader)?;

    if !auth_header.starts_with("Bearer ") {
        return Err(AuthError::InvalidFormat);
    }

    let token = auth_header[7..].trim();

    // Token format for MVP: "org_id:user_id:role1,role2:project1,project2"
    // Example: "org-123:user-456:Developer,Reader:proj-1,proj-2"
    // In production, this would validate a JWT
    let parts: Vec<&str> = token.split(':').collect();
    if parts.len() < 2 {
        return Err(AuthError::InvalidToken);
    }

    let org_id = Uuid::parse_str(parts[0]).map_err(|_| AuthError::InvalidToken)?;
    let user_id = parts[1].to_string();

    // Parse roles (optional, defaults to empty)
    let roles = if parts.len() >= 3 && !parts[2].is_empty() {
        parts[2]
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|s| match s {
                "SystemAdmin" => Role::SystemAdmin,
                "OrgAdmin" => Role::OrgAdmin,
                "ProjectAdmin" => Role::ProjectAdmin,
                "Developer" => Role::Developer,
                "Reader" => Role::Reader,
                "AiAgent" => Role::AiAgent,
                _ => Role::Developer, // Default fallback
            })
            .collect()
    } else {
        vec![Role::Developer] // Default role
    };

    let project_memberships = if parts.len() >= 4 {
        parts[3]
            .split(',')
            .filter(|s| !s.is_empty())
            .filter_map(|s| Uuid::parse_str(s).ok())
            .map(ProjectId::from_uuid)
            .collect()
    } else {
        vec![]
    };

    Ok(AuthPrincipal {
        id: user_id,
        organization_id: OrganizationId::from_uuid(org_id),
        project_memberships,
        roles,
    })
}

/// axum extractor for AuthContext
#[derive(Debug, Clone)]
pub struct ExtractAuth(pub AuthContext);

impl axum::extract::FromRequestParts<AppState> for ExtractAuth {
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let principal = extract_auth_principal(&parts.headers)?;
        Ok(ExtractAuth(AuthContext { principal }))
    }
}

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
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Forbidden: {0}")]
    Forbidden(String),
    #[error("Conflict: {0}")]
    Conflict(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match &self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, self.to_string()),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
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
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, self.to_string()),
            ApiError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg.clone()),
            ApiError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
        };

        let body = Json(ErrorResponse { error: message });
        (status, body).into_response()
    }
}

impl From<StatePropagationError> for ApiError {
    fn from(err: StatePropagationError) -> Self {
        match err {
            StatePropagationError::ArchivedAssetCannotTrigger => {
                ApiError::Conflict("Cannot publish archived asset".to_string())
            }
            StatePropagationError::DownstreamAssetNotFound(id) => {
                ApiError::BadRequest(format!("Downstream asset not found: {id:?}"))
            }
            StatePropagationError::Repository(e) => ApiError::Repository(e),
        }
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
/// Note: organization_id is NOT in the request body - it comes from AuthPrincipal
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateAssetRequest {
    pub name: String,
    pub asset_type_id: Uuid,
    pub project_id: Option<Uuid>,
    pub level: AssetLevelDto,
    pub idempotency_key: Option<String>,
    /// External system reference URL
    pub external_ref: String,
    /// Source system (git/wiki/jira/manual)
    pub source: String,
    /// Metadata JSON according to asset type schema
    pub metadata: Option<serde_json::Value>,
    /// Optional: declare dependencies at creation time
    pub dependencies: Option<Vec<Uuid>>,
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
    pub external_ref: String,
    pub source: String,
    pub metadata: serde_json::Value,
    pub assignees: Vec<String>,
    pub publisher: Option<String>,
    pub current_version: Option<String>,
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
        // Clone fields that need to be accessed after partial moves
        let external_ref = asset.external_ref.clone();
        let source = asset.source.clone();
        let metadata = asset.metadata.clone();
        let assignees = asset.assignees.clone();
        let created_at = asset.created_at;

        // Call borrowing methods
        let state = asset.state();
        let updated_at = asset.updated_at();
        let publisher = asset.publisher().map(|s| s.to_string());
        let current_version = asset.current_version().map(|s| s.to_string());

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
            current_state: state.into(),
            external_ref,
            source,
            metadata,
            assignees,
            publisher,
            current_version,
            created_at,
            updated_at,
        }
    }
}

/// Query parameters for listing assets (FR-021)
#[derive(Debug, Deserialize)]
pub struct ListAssetsQuery {
    /// Required: project to scope the query
    pub project_id: Uuid,
    /// Optional: filter by asset type
    pub asset_type: Option<Uuid>,
    /// Optional: filter by state (clean/dirty/archived)
    pub state: Option<String>,
    /// Optional: filter by name substring
    pub name_contains: Option<String>,
    /// Optional: filter by assignee
    pub assignee: Option<String>,
    /// Pagination: page number (1-based)
    pub page: Option<u32>,
    /// Pagination: items per page (default 20, max 100)
    pub per_page: Option<u32>,
    /// Sort field (name, created_at, updated_at)
    pub sort_by: Option<String>,
    /// Sort order (asc/desc)
    pub sort_order: Option<String>,
}

/// Paginated response wrapper
#[derive(Debug, Serialize, Deserialize)]
pub struct PaginatedResponse<T> {
    pub data: Vec<T>,
    pub total: usize,
    pub page: u32,
    pub per_page: u32,
    pub total_pages: u32,
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

/// Update asset request (FR-007)
#[derive(Debug, Deserialize)]
pub struct UpdateAssetRequest {
    pub name: Option<String>,
    pub assignees: Option<Vec<String>>,
    pub metadata: Option<serde_json::Value>,
}

// ============================================================================
// AssetType DTOs (FR-001/002)
// ============================================================================

/// Create asset type request
#[derive(Debug, Deserialize)]
pub struct CreateAssetTypeRequest {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub metadata_schema: serde_json::Value,
}

/// Asset type response
#[derive(Debug, Serialize)]
pub struct AssetTypeResponse {
    pub id: Uuid,
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<adam_domain::AssetType> for AssetTypeResponse {
    fn from(asset_type: adam_domain::AssetType) -> Self {
        AssetTypeResponse {
            id: asset_type.id.0,
            name: asset_type.name,
            display_name: asset_type.display_name,
            description: asset_type.description,
            created_at: asset_type.created_at,
        }
    }
}

// ============================================================================
// Application State
// ============================================================================

/// Shared application state for handlers
#[derive(Clone)]
pub struct AppState {
    pub asset_repo: Arc<dyn AssetRepository>,
    pub asset_type_repo: Arc<dyn AssetTypeRepository>,
    pub dependency_repo: Arc<dyn DependencyRepository>,
    pub dirty_repo: Arc<dyn DirtyQueueRepository>,
}

// ============================================================================
// Authorization
// ============================================================================

/// Check if principal can access an asset
fn check_asset_access(
    principal: &AuthPrincipal,
    asset: &AssetInstance,
) -> Result<(), AuthorizationError> {
    // Check organization boundary
    if asset.organization_id != principal.organization_id {
        return Err(AuthorizationError::CrossOrganizationAccessDenied);
    }

    // For project-level assets, check project membership
    // OrgAdmin and SystemAdmin can bypass project membership checks within their org
    if let Some(asset_project_id) = asset.project_id {
        let is_org_admin = principal
            .roles
            .iter()
            .any(|r| matches!(r, Role::OrgAdmin | Role::SystemAdmin));

        if !is_org_admin && !principal.project_memberships.contains(&asset_project_id) {
            return Err(AuthorizationError::ProjectAccessDenied(asset_project_id));
        }
    }

    Ok(())
}

/// Check if principal can access a project
fn check_project_access(
    principal: &AuthPrincipal,
    project_id: ProjectId,
) -> Result<(), AuthorizationError> {
    // OrgAdmin and SystemAdmin can bypass project membership checks within their org
    let is_org_admin = principal
        .roles
        .iter()
        .any(|r| matches!(r, Role::OrgAdmin | Role::SystemAdmin));

    if !is_org_admin && !principal.project_memberships.contains(&project_id) {
        return Err(AuthorizationError::ProjectAccessDenied(project_id));
    }
    Ok(())
}

/// Create a new asset
/// Organization context comes from AuthPrincipal, not request body
pub async fn create_asset(
    State(state): State<AppState>,
    ExtractAuth(auth): ExtractAuth,
    Json(req): Json<CreateAssetRequest>,
) -> Result<(StatusCode, Json<AssetResponse>), ApiError> {
    // For project-level assets, validate project membership
    if let Some(project_id) = req.project_id {
        let pid = ProjectId::from_uuid(project_id);
        check_project_access(&auth.principal, pid).map_err(|_| {
            ApiError::Forbidden(format!(
                "User is not a member of project {} in organization {:?}",
                project_id, auth.principal.organization_id
            ))
        })?;
    }

    // TODO: Validate that project belongs to principal's organization
    // (requires ProjectRepository lookup)

    // Organization comes from authenticated principal, not request body
    let cmd = CreateAssetCommand {
        name: req.name,
        asset_type_id: AssetTypeId::from_uuid(req.asset_type_id),
        project_id: req.project_id.map(ProjectId::from_uuid),
        organization_id: auth.principal.organization_id,
        level: req.level.into(),
        external_ref: req.external_ref,
        source: req.source,
        metadata: req.metadata.unwrap_or_else(|| serde_json::json!({})),
        idempotency_key: req.idempotency_key,
    };

    let asset = state
        .asset_repo
        .create(&cmd)
        .await
        .map_err(ApiError::from)?;

    // Create dependencies if provided
    if let Some(dependency_ids) = req.dependencies {
        for dep_id in dependency_ids {
            let target_id = AssetId::from_uuid(dep_id);

            // Verify the target asset exists
            if state
                .asset_repo
                .find_by_id(&target_id)
                .await
                .map_err(ApiError::from)?
                .is_none()
            {
                return Err(ApiError::BadRequest(format!(
                    "Dependency asset not found: {dep_id}"
                )));
            }

            // Create the dependency relationship
            state
                .dependency_repo
                .create_dependency(&asset.id, &target_id)
                .await
                .map_err(ApiError::from)?;
        }
    }

    Ok((StatusCode::CREATED, Json(asset.into())))
}

/// Get asset by ID
pub async fn get_asset(
    State(state): State<AppState>,
    ExtractAuth(auth): ExtractAuth,
    Path(id): Path<Uuid>,
) -> Result<Json<AssetResponse>, ApiError> {
    let asset_id = AssetId::from_uuid(id);
    let asset = state
        .asset_repo
        .find_by_id(&asset_id)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;

    // Verify asset is accessible by the authenticated principal
    check_asset_access(&auth.principal, &asset).map_err(|e| match e {
        AuthorizationError::CrossOrganizationAccessDenied => {
            ApiError::Forbidden("Cross-organization access denied".to_string())
        }
        AuthorizationError::ProjectAccessDenied(_) => {
            ApiError::Forbidden("Project access denied".to_string())
        }
        _ => ApiError::Forbidden("Access denied".to_string()),
    })?;

    Ok(Json(asset.into()))
}

/// List assets for a project
/// Per FR-026: Returns project-level assets + organization-level assets
pub async fn list_assets(
    State(state): State<AppState>,
    ExtractAuth(auth): ExtractAuth,
    Query(query): Query<ListAssetsQuery>,
) -> Result<Json<PaginatedResponse<AssetResponse>>, ApiError> {
    let project_id = ProjectId::from_uuid(query.project_id);

    // Verify principal has access to this project
    check_project_access(&auth.principal, project_id).map_err(|_| {
        ApiError::Forbidden(format!(
            "User is not a member of project {} in organization {:?}",
            query.project_id, auth.principal.organization_id
        ))
    })?;

    // Get project-level assets
    let project_assets = state
        .asset_repo
        .find_by_project_id(&project_id)
        .await
        .map_err(ApiError::from)?;

    // Get organization-level assets
    let org_assets = state
        .asset_repo
        .find_by_organization_id(&auth.principal.organization_id)
        .await
        .map_err(ApiError::from)?;

    // Merge: project assets + org-level assets
    let mut all_assets: Vec<AssetInstance> = project_assets;
    for asset in org_assets {
        if asset.project_id.is_none() {
            all_assets.push(asset);
        }
    }

    // Apply filters
    let filtered: Vec<AssetInstance> = all_assets
        .into_iter()
        .filter(|a| {
            // Filter by asset_type
            if let Some(type_id) = query.asset_type {
                if a.asset_type_id.0 != type_id {
                    return false;
                }
            }
            // Filter by state
            if let Some(ref state_str) = query.state {
                let matches = match state_str.as_str() {
                    "clean" => a.state() == AssetState::Clean,
                    "dirty" => a.state() == AssetState::Dirty,
                    "archived" => a.state() == AssetState::Archived,
                    _ => true,
                };
                if !matches {
                    return false;
                }
            }
            // Filter by name_contains
            if let Some(ref name_pat) = query.name_contains {
                if !a.name.to_lowercase().contains(&name_pat.to_lowercase()) {
                    return false;
                }
            }
            // Filter by assignee
            if let Some(ref assignee) = query.assignee {
                if !a.assignees.contains(assignee) {
                    return false;
                }
            }
            true
        })
        .collect();

    // Pagination
    let total = filtered.len();
    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(20).clamp(1, 100);
    let total_pages = ((total as f64) / (per_page as f64)).ceil() as u32;

    let start = ((page - 1) * per_page) as usize;
    let end = (start + per_page as usize).min(total);
    let paginated: Vec<AssetResponse> = filtered[start..end]
        .iter()
        .cloned()
        .map(AssetResponse::from)
        .collect();

    Ok(Json(PaginatedResponse {
        data: paginated,
        total,
        page,
        per_page,
        total_pages,
    }))
}

/// Publish a new version (triggers dirty propagation)
pub async fn publish_asset(
    State(state): State<AppState>,
    ExtractAuth(_auth): ExtractAuth,
    Path(id): Path<Uuid>,
    Json(req): Json<PublishRequest>,
) -> Result<(StatusCode, Json<PublishResponse>), ApiError> {
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
        .map_err(ApiError::from)?;

    Ok((
        StatusCode::OK,
        Json(PublishResponse {
            affected_assets: affected.into_iter().map(|id| id.0).collect(),
        }),
    ))
}

/// Resolve dirty state
/// Per architecture: resolve dirty queue entries first, only mark Clean when no unresolved remain
pub async fn resolve_dirty(
    State(state): State<AppState>,
    ExtractAuth(_auth): ExtractAuth,
    Path(id): Path<Uuid>,
    Json(_req): Json<ResolveRequest>,
) -> Result<StatusCode, ApiError> {
    let asset_id = AssetId::from_uuid(id);

    // Check for unresolved dirty queue entries for this asset
    let unresolved = state
        .dirty_repo
        .find_unresolved_by_asset(&asset_id)
        .await
        .map_err(ApiError::from)?;

    if !unresolved.is_empty() {
        // Mark all unresolved entries as resolved
        for entry in unresolved {
            state
                .dirty_repo
                .resolve(&entry.id)
                .await
                .map_err(ApiError::from)?;
        }
    }

    // Check again if there are any remaining unresolved entries
    let remaining = state
        .dirty_repo
        .find_unresolved_by_asset(&asset_id)
        .await
        .map_err(ApiError::from)?;

    // Only mark as Clean if no unresolved entries remain
    if remaining.is_empty() {
        state
            .asset_repo
            .update_state(&asset_id, AssetState::Clean)
            .await
            .map_err(ApiError::from)?;
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Create asset type (FR-001/002)
pub async fn create_asset_type(
    State(state): State<AppState>,
    ExtractAuth(auth): ExtractAuth,
    Json(req): Json<CreateAssetTypeRequest>,
) -> Result<(StatusCode, Json<AssetTypeResponse>), ApiError> {
    // Check permission
    if !auth
        .principal
        .has_permission(adam_domain::auth::Permission::AssetTypeCreate)
    {
        return Err(ApiError::Forbidden(
            "AssetTypeCreate permission required".to_string(),
        ));
    }

    // Create asset type
    let asset_type = adam_domain::AssetType::new(
        auth.principal.organization_id,
        req.name,
        req.display_name,
        req.description,
        req.metadata_schema,
    );

    // Save to repository
    let created = state
        .asset_type_repo
        .create(&asset_type)
        .await
        .map_err(ApiError::from)?;

    Ok((StatusCode::CREATED, Json(created.into())))
}

/// List asset types
pub async fn list_asset_types(
    State(state): State<AppState>,
    ExtractAuth(auth): ExtractAuth,
) -> Result<Json<Vec<AssetTypeResponse>>, ApiError> {
    // Check permission
    if !auth
        .principal
        .has_permission(adam_domain::auth::Permission::AssetTypeRead)
    {
        return Err(ApiError::Forbidden(
            "AssetTypeRead permission required".to_string(),
        ));
    }

    // Query from repository
    let asset_types = state
        .asset_type_repo
        .list_all()
        .await
        .map_err(ApiError::from)?;

    Ok(Json(asset_types.into_iter().map(|at| at.into()).collect()))
}

/// Update asset (FR-007)
pub async fn update_asset(
    State(state): State<AppState>,
    ExtractAuth(auth): ExtractAuth,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateAssetRequest>,
) -> Result<Json<AssetResponse>, ApiError> {
    let asset_id = AssetId::from_uuid(id);

    // Get existing asset
    let asset = state
        .asset_repo
        .find_by_id(&asset_id)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;

    // Check access
    if let Err(e) = check_asset_access(&auth.principal, &asset) {
        return Err(match e {
            AuthorizationError::CrossOrganizationAccessDenied => {
                ApiError::Forbidden("Cross-organization access denied".to_string())
            }
            AuthorizationError::ProjectAccessDenied(_) => {
                ApiError::Forbidden("Project access denied".to_string())
            }
            _ => ApiError::Forbidden("Access denied".to_string()),
        });
    }

    // Check permission
    if !auth
        .principal
        .has_permission(adam_domain::auth::Permission::AssetUpdate)
    {
        return Err(ApiError::Forbidden(
            "AssetUpdate permission required".to_string(),
        ));
    }

    // Update the asset
    let cmd = adam_domain::UpdateAssetCommand {
        name: req.name,
        assignees: req.assignees,
        metadata: req.metadata,
    };

    let updated = state
        .asset_repo
        .update(&asset_id, &cmd)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(updated.into()))
}

/// Delete asset (FR-008)
pub async fn delete_asset(
    State(state): State<AppState>,
    ExtractAuth(auth): ExtractAuth,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let asset_id = AssetId::from_uuid(id);

    // Get asset
    let asset = state
        .asset_repo
        .find_by_id(&asset_id)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;

    // Check access and permission
    if let Err(e) = check_asset_access(&auth.principal, &asset) {
        return Err(match e {
            AuthorizationError::CrossOrganizationAccessDenied => {
                ApiError::Forbidden("Cross-organization access denied".to_string())
            }
            AuthorizationError::ProjectAccessDenied(_) => {
                ApiError::Forbidden("Project access denied".to_string())
            }
            _ => ApiError::Forbidden("Access denied".to_string()),
        });
    }

    if !auth
        .principal
        .has_permission(adam_domain::auth::Permission::AssetDelete)
    {
        return Err(ApiError::Forbidden(
            "AssetDelete permission required".to_string(),
        ));
    }

    // Check downstream dependencies before deleting
    let downstream = state
        .dependency_repo
        .find_downstream(&asset_id)
        .await
        .map_err(ApiError::from)?;

    if !downstream.is_empty() {
        return Err(ApiError::Conflict(format!(
            "Cannot delete asset with {} downstream dependencies",
            downstream.len()
        )));
    }

    // Delete the asset
    state
        .asset_repo
        .delete(&asset_id)
        .await
        .map_err(ApiError::from)?;

    Ok(StatusCode::NO_CONTENT)
}

// ============================================================================
// Router
// ============================================================================

/// Create the REST API router with middleware layers
pub fn create_router(state: AppState) -> Router {
    // Protected routes - require authentication
    let protected_routes = Router::new()
        .route("/api/v1/assets", post(create_asset).get(list_assets))
        .route(
            "/api/v1/assets/{id}",
            get(get_asset).put(update_asset).delete(delete_asset),
        )
        .route("/api/v1/assets/{id}/publish", post(publish_asset))
        .route("/api/v1/assets/{id}/resolve", post(resolve_dirty))
        // AssetType routes (FR-001/002)
        .route(
            "/api/v1/asset-types",
            post(create_asset_type).get(list_asset_types),
        );

    // Public routes (if any) would go here
    let public_routes = Router::new().route("/health", get(health_check));

    Router::new()
        .merge(protected_routes)
        .merge(public_routes)
        // Add CORS layer with restricted configuration
        .layer(
            CorsLayer::new()
                .allow_origin([
                    "http://localhost:3000".parse().unwrap(),
                    "http://localhost:8080".parse().unwrap(),
                ])
                .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
                .allow_headers([
                    axum::http::header::AUTHORIZATION,
                    axum::http::header::CONTENT_TYPE,
                ]),
        )
        // Add tracing layer
        .layer(
            TraceLayer::new_for_http()
                .on_request(DefaultOnRequest::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO))
                .on_failure(DefaultOnFailure::new().level(Level::ERROR)),
        )
        .with_state(state)
}

/// Health check handler - public endpoint
async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// Health check response
#[derive(Debug, Serialize)]
struct HealthResponse {
    status: String,
    version: String,
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
            asset_type_repo: Arc::new(adam_domain::InMemoryAssetTypeRepository::new()),
            dependency_repo: Arc::new(InMemoryDependencyRepository::new()),
            dirty_repo: Arc::new(adam_domain::InMemoryDirtyQueueRepository::new()),
        }
    }

    /// Generate a test authorization header
    /// Token format: "org_id:user_id:role1,role2:project1,project2"
    fn test_auth_header(
        org_id: Uuid,
        user_id: &str,
        roles: &[Role],
        project_ids: &[Uuid],
    ) -> (String, String) {
        let roles_str = roles
            .iter()
            .map(|r| match r {
                Role::SystemAdmin => "SystemAdmin",
                Role::OrgAdmin => "OrgAdmin",
                Role::ProjectAdmin => "ProjectAdmin",
                Role::Developer => "Developer",
                Role::Reader => "Reader",
                Role::AiAgent => "AiAgent",
            })
            .collect::<Vec<_>>()
            .join(",");
        let projects = project_ids
            .iter()
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let token = format!("{org_id}:{user_id}:{roles_str}:{projects}");
        ("authorization".to_string(), format!("Bearer {token}"))
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
    async fn create_asset_without_auth_returns_401() {
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
                            "level": "project",
                            "project_id": Uuid::new_v4(),
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn create_asset_endpoint_returns_201() {
        let state = create_test_state();
        let app = create_router(state);

        let org_id = Uuid::new_v4();
        let project_id = Uuid::new_v4();
        let (auth_header, auth_value) =
            test_auth_header(org_id, "user-123", &[Role::Developer], &[project_id]);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/assets")
                    .header("content-type", "application/json")
                    .header(&auth_header, &auth_value)
                    .body(Body::from(
                        serde_json::json!({
                            "name": "Test Asset",
                            "asset_type_id": Uuid::new_v4(),
                            "level": "project",
                            "project_id": project_id,
                            "external_ref": "https://example.com/asset",
                            "source": "manual",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);

        // Verify response body - organization_id should come from auth, not request
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let asset: AssetResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(asset.name, "Test Asset");
        assert_eq!(asset.organization_id, org_id); // From auth context
    }

    #[tokio::test]
    async fn create_asset_with_invalid_level_returns_422() {
        let state = create_test_state();
        let app = create_router(state);

        let org_id = Uuid::new_v4();
        let (auth_header, auth_value) =
            test_auth_header(org_id, "user-123", &[Role::Developer], &[]);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/assets")
                    .header("content-type", "application/json")
                    .header(&auth_header, &auth_value)
                    .body(Body::from(
                        r#"{"name": "Test", "asset_type_id": "00000000-0000-0000-0000-000000000001", "level": "invalid"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Axum returns 422 for JSON deserialization errors
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn get_asset_returns_401_without_auth() {
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

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn get_asset_returns_200_for_existing() {
        let state = create_test_state();
        let org_id = OrganizationId::from_uuid(Uuid::new_v4());

        // First create an asset
        let type_id = AssetTypeId::new();
        let cmd = CreateAssetCommand {
            name: "Existing Asset".to_string(),
            asset_type_id: type_id,
            project_id: None,
            organization_id: org_id,
            level: adam_domain::dependency::boundary::AssetLevel::Organization,
            external_ref: "https://example.com/asset/1".to_string(),
            source: "manual".to_string(),
            metadata: serde_json::json!({}),
            idempotency_key: None,
        };
        let asset = state.asset_repo.create(&cmd).await.unwrap();

        let app = create_router(state);
        let (auth_header, auth_value) =
            test_auth_header(org_id.0, "user-123", &[Role::Developer], &[]);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/v1/assets/{}", asset.id.0))
                    .header(&auth_header, &auth_value)
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

        let org_id = Uuid::new_v4();
        let (auth_header, auth_value) =
            test_auth_header(org_id, "user-123", &[Role::Developer], &[]);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/v1/assets/{}", Uuid::new_v4()))
                    .header(&auth_header, &auth_value)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn list_assets_requires_project_id() {
        let state = create_test_state();
        let app = create_router(state);

        let org_id = Uuid::new_v4();
        let (auth_header, auth_value) =
            test_auth_header(org_id, "user-123", &[Role::Developer], &[]);

        // Without project_id query param, should fail (400 - missing required param)
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/assets")
                    .header(&auth_header, &auth_value)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Query parameter is required per FR-026 - axum returns 400 for missing required params
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn list_assets_returns_project_and_org_level_assets() {
        let state = create_test_state();
        let org_id = OrganizationId::from_uuid(Uuid::new_v4());
        let project_id = ProjectId::new();

        // Create a project-level asset
        let project_cmd = CreateAssetCommand {
            name: "Project Asset".to_string(),
            asset_type_id: AssetTypeId::new(),
            project_id: Some(project_id),
            organization_id: org_id,
            level: adam_domain::dependency::boundary::AssetLevel::Project,
            external_ref: "https://example.com/project/asset".to_string(),
            source: "manual".to_string(),
            metadata: serde_json::json!({}),
            idempotency_key: None,
        };
        state.asset_repo.create(&project_cmd).await.unwrap();

        // Create an organization-level asset
        let org_cmd = CreateAssetCommand {
            name: "Org Asset".to_string(),
            asset_type_id: AssetTypeId::new(),
            project_id: None,
            organization_id: org_id,
            level: adam_domain::dependency::boundary::AssetLevel::Organization,
            external_ref: "https://example.com/org/asset".to_string(),
            source: "manual".to_string(),
            metadata: serde_json::json!({}),
            idempotency_key: None,
        };
        state.asset_repo.create(&org_cmd).await.unwrap();

        let app = create_router(state);
        // User must be a member of the project to list its assets
        let (auth_header, auth_value) =
            test_auth_header(org_id.0, "user-123", &[Role::Developer], &[project_id.0]);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/v1/assets?project_id={}", project_id.0))
                    .header(&auth_header, &auth_value)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let paginated: PaginatedResponse<AssetResponse> = serde_json::from_slice(&body).unwrap();

        // Should return both project asset and org-level asset
        assert_eq!(paginated.total, 2);
        assert_eq!(paginated.data.len(), 2);
        assert!(paginated.data.iter().any(|a| a.name == "Project Asset"));
        assert!(paginated.data.iter().any(|a| a.name == "Org Asset"));
    }

    #[tokio::test]
    async fn list_assets_requires_auth() {
        let state = create_test_state();
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/assets?project_id=00000000-0000-0000-0000-000000000001")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn resolve_dirty_resolves_queue_entries_and_sets_clean() {
        let state = create_test_state();
        let org_id = OrganizationId::new();

        // Create a dirty asset - use the returned asset which has the actual ID
        let created_asset = state
            .asset_repo
            .create(&CreateAssetCommand {
                name: "Dirty Asset".to_string(),
                asset_type_id: AssetTypeId::new(),
                project_id: None,
                organization_id: org_id,
                level: adam_domain::dependency::boundary::AssetLevel::Organization,
                external_ref: "https://example.com/asset".to_string(),
                source: "manual".to_string(),
                metadata: serde_json::json!({}),
                idempotency_key: None,
            })
            .await
            .unwrap();

        // Create a dirty queue entry for this asset
        let entry = adam_domain::DirtyQueueEntry {
            id: uuid::Uuid::new_v4(),
            asset_id: created_asset.id,
            upstream_asset_id: AssetId::new(),
            upstream_version: "v1.0.0".to_string(),
            upstream_old_version: "0.0.0".to_string(),
            impact_level: "medium".to_string(),
            since: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
            resolved_at: None,
        };
        state.dirty_repo.upsert(&entry).await.unwrap();

        // Update asset to Dirty state
        state
            .asset_repo
            .update_state(&created_asset.id, AssetState::Dirty)
            .await
            .unwrap();

        let app = create_router(state.clone());
        let (auth_header, auth_value) =
            test_auth_header(org_id.0, "user-123", &[Role::Developer], &[]);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(format!("/api/v1/assets/{}/resolve", created_asset.id.0))
                    .header("content-type", "application/json")
                    .header(&auth_header, &auth_value)
                    .body(Body::from(r#"{"resolved_version": "v1.0.0"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        // Verify the dirty queue entry is resolved
        let unresolved = state
            .dirty_repo
            .find_unresolved_by_asset(&created_asset.id)
            .await
            .unwrap();
        assert!(unresolved.is_empty());

        // Verify asset is now Clean
        let updated = state
            .asset_repo
            .find_by_id(&created_asset.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.state(), AssetState::Clean);
    }

    #[tokio::test]
    async fn health_check_returns_200() {
        let state = create_test_state();
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let health: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(health["status"], "healthy");
        assert!(health["version"].as_str().is_some());
    }

    #[tokio::test]
    async fn create_asset_non_member_returns_403() {
        let state = create_test_state();
        let app = create_router(state);

        // User is member of project1 but not project2
        let org_id = Uuid::new_v4();
        let project1 = Uuid::new_v4();
        let project2 = Uuid::new_v4();
        let (auth_header, auth_value) =
            test_auth_header(org_id, "user-123", &[Role::Developer], &[project1]);

        // Try to create asset in project2 (not a member)
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/assets")
                    .header("content-type", "application/json")
                    .header(&auth_header, &auth_value)
                    .body(Body::from(
                        serde_json::json!({
                            "name": "Test Asset",
                            "asset_type_id": Uuid::new_v4(),
                            "level": "project",
                            "project_id": project2, // Not a member
                            "external_ref": "https://example.com/asset",
                            "source": "manual",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn get_asset_cross_org_returns_403() {
        let state = create_test_state();

        // Create asset in org1
        let org1_id = OrganizationId::new();
        let asset = state
            .asset_repo
            .create(&CreateAssetCommand {
                name: "Org1 Asset".to_string(),
                asset_type_id: AssetTypeId::new(),
                project_id: None,
                organization_id: org1_id,
                level: adam_domain::dependency::boundary::AssetLevel::Organization,
                external_ref: "https://example.com/asset".to_string(),
                source: "manual".to_string(),
                metadata: serde_json::json!({}),
                idempotency_key: None,
            })
            .await
            .unwrap();

        let app = create_router(state);

        // User from org2 tries to access
        let org2_id = Uuid::new_v4();
        let (auth_header, auth_value) =
            test_auth_header(org2_id, "user-456", &[Role::Developer], &[]);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/v1/assets/{}", asset.id.0))
                    .header(&auth_header, &auth_value)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn get_project_asset_non_member_returns_403() {
        let state = create_test_state();

        // Create project-level asset
        let org_id = OrganizationId::new();
        let project_id = ProjectId::new();
        let asset = state
            .asset_repo
            .create(&CreateAssetCommand {
                name: "Project Asset".to_string(),
                asset_type_id: AssetTypeId::new(),
                project_id: Some(project_id),
                organization_id: org_id,
                level: adam_domain::dependency::boundary::AssetLevel::Project,
                external_ref: "https://example.com/asset".to_string(),
                source: "manual".to_string(),
                metadata: serde_json::json!({}),
                idempotency_key: None,
            })
            .await
            .unwrap();

        let app = create_router(state);

        // User is member of different project
        let other_project = Uuid::new_v4();
        let (auth_header, auth_value) =
            test_auth_header(org_id.0, "user-456", &[Role::Developer], &[other_project]);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/v1/assets/{}", asset.id.0))
                    .header(&auth_header, &auth_value)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn list_assets_non_member_returns_403() {
        let state = create_test_state();
        let app = create_router(state);

        // User is member of project1 but not project2
        let org_id = Uuid::new_v4();
        let project1 = Uuid::new_v4();
        let project2 = Uuid::new_v4();
        let (auth_header, auth_value) =
            test_auth_header(org_id, "user-123", &[Role::Developer], &[project1]);

        // Try to list assets for project2 (not a member)
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/v1/assets?project_id={project2}"))
                    .header(&auth_header, &auth_value)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn org_admin_can_access_any_project_in_org() {
        let state = create_test_state();
        let app = create_router(state);

        // User is NOT member of any project but has OrgAdmin role
        let org_id = Uuid::new_v4();
        let project_id = Uuid::new_v4();
        let (auth_header, auth_value) =
            test_auth_header(org_id, "org-admin", &[Role::OrgAdmin], &[]);

        // Try to list assets for project (not a member, but OrgAdmin)
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/v1/assets?project_id={project_id}"))
                    .header(&auth_header, &auth_value)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // OrgAdmin can access any project in their org - returns 200 even if empty
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn system_admin_can_access_any_project_in_org() {
        let state = create_test_state();
        let app = create_router(state);

        // User is NOT member of any project but has SystemAdmin role
        let org_id = Uuid::new_v4();
        let project_id = Uuid::new_v4();
        let (auth_header, auth_value) =
            test_auth_header(org_id, "sys-admin", &[Role::SystemAdmin], &[]);

        // Try to list assets for project (not a member, but SystemAdmin)
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/v1/assets?project_id={project_id}"))
                    .header(&auth_header, &auth_value)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // SystemAdmin can access any project in their org - returns 200 even if empty
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn org_admin_cannot_access_cross_org() {
        let state = create_test_state();

        // Create asset in org1
        let org1_id = OrganizationId::new();
        let project1_id = ProjectId::new();
        let asset = state
            .asset_repo
            .create(&CreateAssetCommand {
                name: "Org1 Asset".to_string(),
                asset_type_id: AssetTypeId::new(),
                project_id: Some(project1_id),
                organization_id: org1_id,
                level: adam_domain::dependency::boundary::AssetLevel::Project,
                external_ref: "https://example.com/asset".to_string(),
                source: "manual".to_string(),
                metadata: serde_json::json!({}),
                idempotency_key: None,
            })
            .await
            .unwrap();

        let app = create_router(state);

        // OrgAdmin from org2 (different org) tries to access
        let org2_id = Uuid::new_v4();
        let (auth_header, auth_value) =
            test_auth_header(org2_id, "org-admin", &[Role::OrgAdmin], &[]);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!(
                        "/api/v1/assets/{}?project_id={}",
                        asset.id.0, project1_id.0
                    ))
                    .header(&auth_header, &auth_value)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Cross-org access denied even for OrgAdmin
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
