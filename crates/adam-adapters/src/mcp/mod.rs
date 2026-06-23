//! ADAM MCP Server - Model Context Protocol implementation for AI Agent integration

// Allow deprecated rmcp::Error - the library will update to rmcp::ErrorData/RmcpError
#![allow(deprecated)]

use adam_application::VersionService;
use adam_application::services::version_service::ChangeType;
use adam_application::services::workflow::AgentTaskService;
use adam_application::services::{
    ManualCleanCommand, ManualCleanResolution, PublishAssetCommand, PublishDependency,
};
use rmcp::{
    ServerHandler,
    model::{CallToolRequestParam, CallToolResult, Content, Implementation, ServerInfo, Tool},
    schemars::JsonSchema,
    service::{RequestContext, RoleServer},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Default agent-task lease duration (seconds) when the caller omits it.
const DEFAULT_AGENT_TASK_LEASE_SECONDS: i64 = 900;

use adam_domain::workflow::repository::{
    AgentTaskRepository, PromotionRuleRepository, WorkflowActionRepository,
    WorkflowEventRepository, WorkflowInstanceRepository,
};
use adam_domain::workflow::{AgentTaskId, Capability};
use adam_domain::{
    AssetId, AssetInstance, AssetRepository, AssetTypeId, AssetVersionRepository, AuthPrincipal,
    AuthorizationError, AuthorizationService, DependencyRepository, DependencyRuleRepository,
    DirtyQueueRepository, DirtyResolutionLogRepository, OrganizationId, Permission, ProjectId,
    RepositoryError, VirtualInstance, VirtualInstanceRepository,
};

// ============================================================================
// MCP Server State
// ============================================================================

/// Shared state for MCP server
#[derive(Clone)]
pub struct McpServerState {
    pub asset_repo: Arc<dyn AssetRepository>,
    pub dependency_repo: Arc<dyn DependencyRepository>,
    pub dependency_rule_repo: Arc<dyn DependencyRuleRepository>,
    pub dirty_repo: Arc<dyn DirtyQueueRepository>,
    pub version_repo: Arc<dyn AssetVersionRepository>,
    pub dirty_log_repo: Arc<dyn DirtyResolutionLogRepository>,
    pub virtual_repo: Arc<dyn VirtualInstanceRepository>,
    /// Workflow automation repositories (Slice 1)
    pub workflow_event_repo: Arc<dyn WorkflowEventRepository>,
    pub workflow_rule_repo: Arc<dyn PromotionRuleRepository>,
    pub workflow_instance_repo: Arc<dyn WorkflowInstanceRepository>,
    pub workflow_action_repo: Arc<dyn WorkflowActionRepository>,
    pub agent_task_repo: Arc<dyn AgentTaskRepository>,
    /// Session authentication principal
    pub principal: AuthPrincipal,
}

// ============================================================================
// MCP Server Error Types
// ============================================================================

/// MCP tool errors that can be returned to clients
#[derive(Debug, thiserror::Error)]
pub enum McpToolError {
    #[error("Authentication required")]
    Unauthorized,
    #[error("Access denied: {0}")]
    AccessDenied(String),
    #[error("Invalid project ID: {0}")]
    InvalidProjectId(String),
    #[error("Invalid asset ID: {0}")]
    InvalidAssetId(String),
    #[error("Asset not found: {0}")]
    AssetNotFound(String),
    #[error("Project not found: {0}")]
    ProjectNotFound(String),
    #[error("Repository error: {0}")]
    Repository(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("{0}")]
    Other(String),
}

impl From<AuthorizationError> for McpToolError {
    fn from(e: AuthorizationError) -> Self {
        match e {
            AuthorizationError::CrossOrganizationAccessDenied => {
                McpToolError::AccessDenied("Cross-organization access denied".into())
            }
            AuthorizationError::ProjectAccessDenied(_) => {
                McpToolError::AccessDenied("Project access denied".into())
            }
            AuthorizationError::PermissionDenied { required } => {
                McpToolError::AccessDenied(format!("Permission {required:?} required"))
            }
            AuthorizationError::ProjectNotFound(_) => {
                McpToolError::ProjectNotFound("Project not found".into())
            }
        }
    }
}

impl From<RepositoryError> for McpToolError {
    fn from(e: RepositoryError) -> Self {
        match e {
            RepositoryError::NotFound(msg) => McpToolError::AssetNotFound(msg),
            other => McpToolError::Repository(other.to_string()),
        }
    }
}

// ============================================================================
// Tool Request/Response Types
// ============================================================================

/// Query assets tool request
#[derive(Debug, Deserialize, JsonSchema)]
pub struct QueryAssetsRequest {
    /// Project ID to scope the query (required per FR-026)
    pub project_id: String,
    /// Optional: Filter by asset type
    pub asset_type: Option<String>,
    /// Optional: Filter by state
    pub state: Option<String>,
    /// Optional: Search by name (substring match)
    pub name_contains: Option<String>,
}

/// Single asset in query response
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct AssetInfo {
    pub id: String,
    pub name: String,
    pub asset_type: String,
    pub state: String,
    pub level: String,
}

/// Query assets tool response
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct QueryAssetsResponse {
    pub assets: Vec<AssetInfo>,
    pub total: usize,
}

/// Create virtual asset tool request
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateVirtualAssetRequest {
    /// Target asset type for the virtual asset
    pub target_type: String,
    /// Anchor asset IDs for context
    pub anchors: Vec<String>,
    /// Project ID for scoping
    pub project_id: String,
}

/// Create virtual asset tool response
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CreateVirtualAssetResponse {
    pub virtual_asset_id: String,
    pub context_summary: String,
}

/// Get asset tool request
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetAssetRequest {
    /// Asset ID to retrieve
    pub asset_id: String,
}

/// Get asset tool response
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetAssetResponse {
    pub id: String,
    pub name: String,
    pub asset_type: String,
    pub state: String,
    pub level: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Get asset content tool request
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetAssetContentRequest {
    /// Asset ID to get content for
    pub asset_id: String,
}

/// Get asset content tool response
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetAssetContentResponse {
    pub asset_id: String,
    pub content: String,
    pub mime_type: String,
}

/// Get dependency graph tool request
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetDependencyGraphRequest {
    /// Asset ID to get dependencies for
    pub asset_id: String,
    /// Direction: "upstream" or "downstream"
    pub direction: String,
}

/// Dependency node in graph response
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DependencyNode {
    pub id: String,
    pub name: String,
    pub asset_type: String,
    pub state: String,
}

/// Get dependency graph tool response
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetDependencyGraphResponse {
    pub asset_id: String,
    pub direction: String,
    pub dependencies: Vec<DependencyNode>,
}

/// Get virtual context tool request
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetVirtualContextRequest {
    /// Virtual instance ID
    pub virtual_asset_id: String,
}

/// Context asset in virtual context response
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ContextAsset {
    pub id: String,
    pub name: String,
    pub asset_type: String,
    pub relevance: String,
}

/// Get virtual context tool response
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetVirtualContextResponse {
    pub virtual_asset_id: String,
    pub target_type: String,
    pub anchors: Vec<String>,
    pub context_assets: Vec<ContextAsset>,
}

/// Publish asset tool request
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PublishAssetRequest {
    /// Asset ID to publish
    pub asset_id: String,
    /// Version to publish (optional, will suggest if not provided)
    pub version: Option<String>,
    /// Dependency IDs to include in publish
    pub dependencies: Option<Vec<String>>,
    /// Relationship types for each dependency (snake_case: depends_on, references, implements, fixes, verifies, executes, produces, blocks, relates_to)
    pub relationships: Option<Vec<String>>,
    /// Propagation policies for each dependency (snake_case: dirty, context_only, audit_only)
    pub propagation_policies: Option<Vec<String>>,
}

/// Published version info
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PublishedVersionInfo {
    pub version: String,
    pub published_at: String,
}

/// Publish asset tool response
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PublishAssetResponse {
    pub asset_id: String,
    pub version: String,
    pub published_version: PublishedVersionInfo,
}

/// Suggest version tool request
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SuggestVersionRequest {
    /// Asset ID to suggest version for
    pub asset_id: String,
    /// Type of change: "major", "minor", "patch"
    pub change_type: Option<String>,
}

/// Suggest version tool response
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SuggestVersionResponse {
    pub asset_id: String,
    pub suggested_version: String,
    pub current_version: Option<String>,
    pub reason: String,
}

/// Refresh asset state tool request
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RefreshAssetStateRequest {
    /// Asset ID to refresh
    pub asset_id: String,
}

/// Refresh asset state tool response
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct RefreshAssetStateResponse {
    pub asset_id: String,
    pub previous_state: String,
    pub current_state: String,
    pub upstream_changes_detected: bool,
}

/// Manual clean asset tool request
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ManualCleanAssetRequest {
    /// Asset ID to clean
    pub asset_id: String,
    /// Asset version being reviewed
    pub resolved_version: Option<String>,
    /// Explicit reviewer ID
    pub reviewed_by: Option<String>,
    /// Explicit upstream resolutions. If omitted, unresolved dirty entries are accepted.
    pub resolutions: Option<Vec<ManualCleanResolutionInput>>,
    /// Review notes
    pub review_notes: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ManualCleanResolutionInput {
    pub upstream_asset_id: String,
    pub from_version: String,
    pub to_version: String,
    pub review_result: String,
    pub comment: Option<String>,
}

/// Manual clean asset tool response
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ManualCleanAssetResponse {
    pub asset_id: String,
    pub previous_state: String,
    pub current_state: String,
    pub review_id: String,
}

/// Request for the `query_workflow_state` tool (Slice 1, design §14).
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct QueryWorkflowStateRequest {
    /// Workflow instance id to inspect.
    pub workflow_instance_id: String,
}

/// Response for the `query_workflow_state` tool.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct QueryWorkflowStateResponse {
    pub instance_id: String,
    pub status: String,
    pub template: String,
    pub correlation_id: String,
    pub cascade_depth: i32,
    pub actions: Vec<WorkflowActionInfo>,
}

/// Minimal action info returned by `query_workflow_state`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct WorkflowActionInfo {
    pub id: String,
    pub action_type: String,
    pub status: String,
    pub target_asset_id: Option<String>,
    pub automation_level: String,
}

/// Request for pending task listing (Slice 2, design §14).
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListPendingAgentTasksRequest {
    pub project_id: Option<String>,
    pub capability_filter: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct AgentTaskInfo {
    pub id: String,
    pub project_id: Option<String>,
    pub action_id: String,
    pub capability: String,
    pub status: String,
    pub agent_id: Option<String>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListPendingAgentTasksResponse {
    pub tasks: Vec<AgentTaskInfo>,
    pub total: usize,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ClaimAgentTaskRequest {
    pub task_id: String,
    pub agent_id: String,
    pub lease_seconds: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ClaimAgentTaskResponse {
    pub task: Option<AgentTaskInfo>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SubmitAgentTaskResultRequest {
    pub task_id: String,
    pub result_payload: serde_json::Value,
    #[serde(default)]
    pub produced_asset_ids: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SubmitAgentTaskResultResponse {
    pub task: AgentTaskInfo,
}

// ============================================================================
// MCP Server Handler
// ============================================================================

/// ADAM MCP Server handler
pub struct AdamMcpServer {
    state: McpServerState,
}

impl AdamMcpServer {
    /// Create a new MCP server with authentication
    pub fn new(state: McpServerState) -> Self {
        Self { state }
    }

    /// Check permission for the current principal
    fn check_permission(
        &self,
        permission: Permission,
        org_id: OrganizationId,
        project_id: Option<ProjectId>,
    ) -> Result<(), McpToolError> {
        AuthorizationService::check(&self.state.principal, permission, org_id, project_id)
            .map_err(McpToolError::from)
    }

    /// Query assets tool implementation
    async fn query_assets(
        &self,
        request: QueryAssetsRequest,
    ) -> Result<CallToolResult, rmcp::Error> {
        // Parse project ID
        let project_id = match parse_uuid(&request.project_id) {
            Some(id) => ProjectId::from_uuid(id),
            None => {
                return Ok(CallToolResult::error(vec![Content::text(
                    "Invalid project_id format",
                )]));
            }
        };

        // Get project to find its organization
        // For MVP: We'll use the principal's organization since we don't have project lookup yet
        // TODO: Add project repository to lookup actual project organization
        let org_id = self.state.principal.organization_id;

        // Check permission: QueryAssets
        if let Err(e) = self.check_permission(Permission::QueryAssets, org_id, Some(project_id)) {
            return Ok(CallToolResult::error(vec![Content::text(e.to_string())]));
        }

        // Get project-level assets
        let project_assets = match self.state.asset_repo.find_by_project_id(&project_id).await {
            Ok(assets) => assets,
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Repository error: {e}"
                ))]));
            }
        };

        // Get organization-level assets
        // Now using the principal's actual organization ID instead of random
        let org_assets = match self.state.asset_repo.find_by_organization_id(&org_id).await {
            Ok(assets) => assets,
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Repository error: {e}"
                ))]));
            }
        };

        // Merge and filter
        let mut assets: Vec<AssetInfo> = Vec::new();

        // Add project assets
        for asset in project_assets {
            if matches_filters(&asset, &request) {
                assets.push(asset_to_info(&asset));
            }
        }

        // Add org-level assets (only if project_id is None)
        for asset in org_assets {
            if asset.project_id.is_none() && matches_filters(&asset, &request) {
                assets.push(asset_to_info(&asset));
            }
        }

        let total = assets.len();
        let response = QueryAssetsResponse { assets, total };

        match serde_json::to_string(&response) {
            Ok(json) => Ok(CallToolResult::success(vec![Content::text(json)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Serialization error: {e}"
            ))])),
        }
    }

    /// Create virtual asset tool implementation
    async fn create_virtual_asset(
        &self,
        request: CreateVirtualAssetRequest,
    ) -> Result<CallToolResult, rmcp::Error> {
        // Parse project ID with validation
        let project_id = match parse_uuid(&request.project_id) {
            Some(id) => ProjectId::from_uuid(id),
            None => {
                return Ok(CallToolResult::error(vec![Content::text(
                    "Invalid project_id format",
                )]));
            }
        };

        // Get organization from principal
        let org_id = self.state.principal.organization_id;

        // Check permission: QueryVirtualContext
        // This also validates project membership (bypassed for OrgAdmin/SystemAdmin)
        if let Err(e) =
            self.check_permission(Permission::QueryVirtualContext, org_id, Some(project_id))
        {
            return Ok(CallToolResult::error(vec![Content::text(e.to_string())]));
        }

        // Parse and validate anchor IDs with boundary checking
        let mut valid_anchors = Vec::new();
        for anchor_id_str in &request.anchors {
            let anchor_id = match parse_uuid(anchor_id_str) {
                Some(id) => AssetId::from_uuid(id),
                None => {
                    return Ok(CallToolResult::error(vec![Content::text(format!(
                        "Invalid anchor ID format: {anchor_id_str}"
                    ))]));
                }
            };

            // Verify anchor exists
            match self.state.asset_repo.find_by_id(&anchor_id).await {
                Ok(Some(asset)) => {
                    // Boundary check: anchor must be in same organization
                    if asset.organization_id != org_id {
                        return Ok(CallToolResult::error(vec![Content::text(format!(
                            "Anchor asset {anchor_id_str} is outside organization boundary"
                        ))]));
                    }

                    // Boundary check: for project-level anchors, must be in same project
                    // or be organization-level assets
                    if let Some(asset_project_id) = asset.project_id {
                        if asset_project_id != project_id {
                            return Ok(CallToolResult::error(vec![Content::text(format!(
                                "Anchor asset {anchor_id_str} is not accessible in this project"
                            ))]));
                        }
                    }

                    valid_anchors.push(anchor_id);
                }
                Ok(None) => {
                    return Ok(CallToolResult::error(vec![Content::text(format!(
                        "Anchor asset not found: {anchor_id_str}"
                    ))]));
                }
                Err(e) => {
                    return Ok(CallToolResult::error(vec![Content::text(format!(
                        "Repository error: {e}"
                    ))]));
                }
            }
        }

        if valid_anchors.is_empty() {
            return Ok(CallToolResult::error(vec![Content::text(
                "At least one valid anchor is required",
            )]));
        }

        // Parse target asset type
        let target_type_id = match parse_uuid(&request.target_type) {
            Some(id) => AssetTypeId::from_uuid(id),
            None => {
                // Try using target_type as a name - for now just create a new ID
                // In production, this would lookup asset type by name
                AssetTypeId::new()
            }
        };

        // Create and persist the virtual instance
        let virtual_instance = VirtualInstance::new(
            target_type_id,
            request.target_type.clone(),
            valid_anchors,
            project_id,
            org_id,
            self.state.principal.id.clone(),
        );

        // Save to repository
        match self.state.virtual_repo.create(&virtual_instance).await {
            Ok(_) => {}
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Failed to create virtual instance: {e}"
                ))]));
            }
        }

        let response = CreateVirtualAssetResponse {
            virtual_asset_id: virtual_instance.id.0.to_string(),
            context_summary: virtual_instance.context_summary,
        };

        match serde_json::to_string(&response) {
            Ok(json) => Ok(CallToolResult::success(vec![Content::text(json)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Serialization error: {e}"
            ))])),
        }
    }

    /// Get asset tool implementation
    async fn get_asset(&self, request: GetAssetRequest) -> Result<CallToolResult, rmcp::Error> {
        // Parse asset ID
        let asset_id = match parse_uuid(&request.asset_id) {
            Some(id) => AssetId::from_uuid(id),
            None => {
                return Ok(CallToolResult::error(vec![Content::text(
                    "Invalid asset_id format",
                )]));
            }
        };

        // Get asset from repository
        let asset = match self.state.asset_repo.find_by_id(&asset_id).await {
            Ok(Some(asset)) => asset,
            Ok(None) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Asset not found: {}",
                    request.asset_id
                ))]));
            }
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Repository error: {e}"
                ))]));
            }
        };

        // Check permission
        let org_id = asset.organization_id;
        if let Err(e) = self.check_permission(Permission::AssetRead, org_id, asset.project_id) {
            return Ok(CallToolResult::error(vec![Content::text(e.to_string())]));
        }

        let response = GetAssetResponse {
            id: asset.id.0.to_string(),
            name: asset.name.clone(),
            asset_type: asset.asset_type_id.0.to_string(),
            state: format!("{:?}", asset.state()),
            level: format!("{:?}", asset.level),
            created_at: asset.created_at.to_rfc3339(),
            updated_at: asset.updated_at().to_rfc3339(),
        };

        match serde_json::to_string(&response) {
            Ok(json) => Ok(CallToolResult::success(vec![Content::text(json)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Serialization error: {e}"
            ))])),
        }
    }

    /// Get asset content tool implementation
    /// Note: Content is stored externally, this returns a reference
    async fn get_asset_content(
        &self,
        request: GetAssetContentRequest,
    ) -> Result<CallToolResult, rmcp::Error> {
        // Parse asset ID
        let asset_id = match parse_uuid(&request.asset_id) {
            Some(id) => AssetId::from_uuid(id),
            None => {
                return Ok(CallToolResult::error(vec![Content::text(
                    "Invalid asset_id format",
                )]));
            }
        };

        // Get asset from repository
        let asset = match self.state.asset_repo.find_by_id(&asset_id).await {
            Ok(Some(asset)) => asset,
            Ok(None) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Asset not found: {}",
                    request.asset_id
                ))]));
            }
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Repository error: {e}"
                ))]));
            }
        };

        // Check permission
        let org_id = asset.organization_id;
        if let Err(e) = self.check_permission(Permission::AssetRead, org_id, asset.project_id) {
            return Ok(CallToolResult::error(vec![Content::text(e.to_string())]));
        }

        // Content is stored externally - return reference
        // In a real implementation, this would fetch from external storage
        let response = GetAssetContentResponse {
            asset_id: asset.id.0.to_string(),
            content: format!(
                "Content for asset '{}' is stored externally. \
                Use the appropriate external system (Git, Wiki, etc.) to access the full content.",
                asset.name
            ),
            mime_type: "text/plain".to_string(),
        };

        match serde_json::to_string(&response) {
            Ok(json) => Ok(CallToolResult::success(vec![Content::text(json)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Serialization error: {e}"
            ))])),
        }
    }

    /// Get dependency graph tool implementation
    async fn get_dependency_graph(
        &self,
        request: GetDependencyGraphRequest,
    ) -> Result<CallToolResult, rmcp::Error> {
        // Parse asset ID
        let asset_id = match parse_uuid(&request.asset_id) {
            Some(id) => AssetId::from_uuid(id),
            None => {
                return Ok(CallToolResult::error(vec![Content::text(
                    "Invalid asset_id format",
                )]));
            }
        };

        // Get asset from repository
        let asset = match self.state.asset_repo.find_by_id(&asset_id).await {
            Ok(Some(asset)) => asset,
            Ok(None) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Asset not found: {}",
                    request.asset_id
                ))]));
            }
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Repository error: {e}"
                ))]));
            }
        };

        // Check permission
        let org_id = asset.organization_id;
        if let Err(e) = self.check_permission(Permission::DependencyRead, org_id, asset.project_id)
        {
            return Ok(CallToolResult::error(vec![Content::text(e.to_string())]));
        }

        // Get dependencies based on direction
        let dependency_ids = match request.direction.as_str() {
            "upstream" => self.state.dependency_repo.find_upstream(&asset_id).await,
            "downstream" => self.state.dependency_repo.find_downstream(&asset_id).await,
            _ => {
                return Ok(CallToolResult::error(vec![Content::text(
                    "Invalid direction. Use 'upstream' or 'downstream'.",
                )]));
            }
        };

        let dependency_ids = match dependency_ids {
            Ok(ids) => ids,
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Repository error: {e}"
                ))]));
            }
        };

        // Build dependency nodes
        let mut dependencies = Vec::new();
        for dep_id in dependency_ids {
            if let Ok(Some(dep_asset)) = self.state.asset_repo.find_by_id(&dep_id).await {
                dependencies.push(DependencyNode {
                    id: dep_asset.id.0.to_string(),
                    name: dep_asset.name.clone(),
                    asset_type: dep_asset.asset_type_id.0.to_string(),
                    state: format!("{:?}", dep_asset.state()),
                });
            }
        }

        let response = GetDependencyGraphResponse {
            asset_id: asset.id.0.to_string(),
            direction: request.direction.clone(),
            dependencies,
        };

        match serde_json::to_string(&response) {
            Ok(json) => Ok(CallToolResult::success(vec![Content::text(json)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Serialization error: {e}"
            ))])),
        }
    }

    /// Get virtual context tool implementation
    async fn get_virtual_context(
        &self,
        request: GetVirtualContextRequest,
    ) -> Result<CallToolResult, rmcp::Error> {
        // Parse virtual asset ID
        let virtual_id = match parse_uuid(&request.virtual_asset_id) {
            Some(id) => adam_domain::VirtualInstanceId::from_uuid(id),
            None => {
                return Ok(CallToolResult::error(vec![Content::text(
                    "Invalid virtual_asset_id format",
                )]));
            }
        };

        // Get virtual instance
        let virtual_instance = match self.state.virtual_repo.find_by_id(&virtual_id).await {
            Ok(Some(instance)) => instance,
            Ok(None) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Virtual instance not found: {}",
                    request.virtual_asset_id
                ))]));
            }
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Repository error: {e}"
                ))]));
            }
        };

        // Check permission
        let org_id = virtual_instance.organization_id;
        let project_id = virtual_instance.project_id;
        if let Err(e) =
            self.check_permission(Permission::QueryVirtualContext, org_id, Some(project_id))
        {
            return Ok(CallToolResult::error(vec![Content::text(e.to_string())]));
        }

        // Build context assets from anchors
        let mut context_assets = Vec::new();
        for anchor_id in &virtual_instance.anchors {
            if let Ok(Some(asset)) = self.state.asset_repo.find_by_id(anchor_id).await {
                context_assets.push(ContextAsset {
                    id: asset.id.0.to_string(),
                    name: asset.name.clone(),
                    asset_type: asset.asset_type_id.0.to_string(),
                    relevance: "anchor".to_string(),
                });
            }
        }

        // Add upstream dependencies of anchors
        for anchor_id in &virtual_instance.anchors {
            if let Ok(upstream_ids) = self.state.dependency_repo.find_upstream(anchor_id).await {
                for upstream_id in upstream_ids {
                    if let Ok(Some(asset)) = self.state.asset_repo.find_by_id(&upstream_id).await {
                        // Avoid duplicates
                        if !context_assets
                            .iter()
                            .any(|ca| ca.id == asset.id.0.to_string())
                        {
                            context_assets.push(ContextAsset {
                                id: asset.id.0.to_string(),
                                name: asset.name.clone(),
                                asset_type: asset.asset_type_id.0.to_string(),
                                relevance: "upstream".to_string(),
                            });
                        }
                    }
                }
            }
        }

        let response = GetVirtualContextResponse {
            virtual_asset_id: virtual_instance.id.0.to_string(),
            target_type: virtual_instance.target_type_name.clone(),
            anchors: virtual_instance
                .anchors
                .iter()
                .map(|a| a.0.to_string())
                .collect(),
            context_assets,
        };

        match serde_json::to_string(&response) {
            Ok(json) => Ok(CallToolResult::success(vec![Content::text(json)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Serialization error: {e}"
            ))])),
        }
    }

    /// Publish asset tool implementation
    async fn publish_asset(
        &self,
        request: PublishAssetRequest,
    ) -> Result<CallToolResult, rmcp::Error> {
        // Parse asset ID
        let asset_id = match parse_uuid(&request.asset_id) {
            Some(id) => AssetId::from_uuid(id),
            None => {
                return Ok(CallToolResult::error(vec![Content::text(
                    "Invalid asset_id format",
                )]));
            }
        };

        // Get asset from repository
        let asset = match self.state.asset_repo.find_by_id(&asset_id).await {
            Ok(Some(asset)) => asset,
            Ok(None) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Asset not found: {}",
                    request.asset_id
                ))]));
            }
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Repository error: {e}"
                ))]));
            }
        };

        // Check permission
        let org_id = asset.organization_id;
        if let Err(e) = self.check_permission(Permission::VersionPublish, org_id, asset.project_id)
        {
            return Ok(CallToolResult::error(vec![Content::text(e.to_string())]));
        }

        // Generate version if not provided
        let version = request.version.unwrap_or_else(|| "1.0.0".to_string());

        let mut dependencies = Vec::new();
        for dependency_id in request.dependencies.unwrap_or_default() {
            let upstream_asset_id = match parse_uuid(&dependency_id) {
                Some(id) => AssetId::from_uuid(id),
                None => {
                    return Ok(CallToolResult::error(vec![Content::text(format!(
                        "Invalid dependency ID format: {dependency_id}"
                    ))]));
                }
            };
            let upstream = match self.state.asset_repo.find_by_id(&upstream_asset_id).await {
                Ok(Some(asset)) => asset,
                Ok(None) => {
                    return Ok(CallToolResult::error(vec![Content::text(format!(
                        "Dependency asset not found: {dependency_id}"
                    ))]));
                }
                Err(e) => {
                    return Ok(CallToolResult::error(vec![Content::text(format!(
                        "Repository error: {e}"
                    ))]));
                }
            };
            dependencies.push(PublishDependency {
                upstream_asset_id,
                version: upstream.current_version().to_string(),
                relationship: None,
                propagation_policy: None,
                upgrade_policy: None,
            });
        }

        let service = VersionService::new(
            self.state.asset_repo.clone(),
            self.state.dirty_repo.clone(),
            self.state.version_repo.clone(),
            self.state.dependency_repo.clone(),
            self.state.dirty_log_repo.clone(),
        );
        let published = match service
            .publish(
                PublishAssetCommand {
                    asset_id,
                    version: version.clone(),
                    publisher: self.state.principal.id.clone(),
                    release_notes: String::new(),
                    dependencies,
                    suggested_type: None,
                },
                self.state.dependency_rule_repo.as_ref(),
            )
            .await
        {
            Ok(version) => version,
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Publish failed: {e}"
                ))]));
            }
        };

        let response = PublishAssetResponse {
            asset_id: asset.id.0.to_string(),
            version: published.version_number.clone(),
            published_version: PublishedVersionInfo {
                version: published.version_number,
                published_at: published.released_at.to_rfc3339(),
            },
        };

        match serde_json::to_string(&response) {
            Ok(json) => Ok(CallToolResult::success(vec![Content::text(json)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Serialization error: {e}"
            ))])),
        }
    }

    /// Suggest version tool implementation
    async fn suggest_version(
        &self,
        request: SuggestVersionRequest,
    ) -> Result<CallToolResult, rmcp::Error> {
        // Parse asset ID
        let asset_id = match parse_uuid(&request.asset_id) {
            Some(id) => AssetId::from_uuid(id),
            None => {
                return Ok(CallToolResult::error(vec![Content::text(
                    "Invalid asset_id format",
                )]));
            }
        };

        // Get asset from repository
        let asset = match self.state.asset_repo.find_by_id(&asset_id).await {
            Ok(Some(asset)) => asset,
            Ok(None) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Asset not found: {}",
                    request.asset_id
                ))]));
            }
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Repository error: {e}"
                ))]));
            }
        };

        // Check permission
        let org_id = asset.organization_id;
        if let Err(e) = self.check_permission(Permission::VersionRead, org_id, asset.project_id) {
            return Ok(CallToolResult::error(vec![Content::text(e.to_string())]));
        }

        // Get current version from asset
        let current_version_str = asset.current_version().to_string();
        let current_version: &str = &current_version_str;

        // Map change_type string to ChangeType enum
        let change_type = match request.change_type.as_deref() {
            Some("major") => ChangeType::Breaking,
            Some("minor") => ChangeType::Feature,
            Some("patch") => ChangeType::Bugfix,
            _ => ChangeType::Feature, // default to minor bump
        };

        // Use VersionService to suggest version - import concrete types for turbofish
        use adam_domain::{
            InMemoryAssetRepository, InMemoryAssetVersionRepository, InMemoryDependencyRepository,
            InMemoryDirtyQueueRepository, InMemoryDirtyResolutionLogRepository,
        };
        let suggested_version = match VersionService::<
            InMemoryAssetRepository,
            InMemoryDirtyQueueRepository,
            InMemoryAssetVersionRepository,
            InMemoryDependencyRepository,
            InMemoryDirtyResolutionLogRepository,
        >::suggest_version(current_version, change_type)
        {
            Ok(version) => version,
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Version suggestion error: {e}"
                ))]));
            }
        };

        let response = SuggestVersionResponse {
            asset_id: asset.id.0.to_string(),
            suggested_version,
            current_version: Some(asset.current_version().to_string()),
            reason: format!(
                "Suggested {:?} version bump from {}",
                request.change_type, current_version
            ),
        };

        match serde_json::to_string(&response) {
            Ok(json) => Ok(CallToolResult::success(vec![Content::text(json)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Serialization error: {e}"
            ))])),
        }
    }

    /// Refresh asset state tool implementation
    async fn refresh_asset_state(
        &self,
        request: RefreshAssetStateRequest,
    ) -> Result<CallToolResult, rmcp::Error> {
        // Parse asset ID
        let asset_id = match parse_uuid(&request.asset_id) {
            Some(id) => AssetId::from_uuid(id),
            None => {
                return Ok(CallToolResult::error(vec![Content::text(
                    "Invalid asset_id format",
                )]));
            }
        };

        // Get asset from repository
        let asset = match self.state.asset_repo.find_by_id(&asset_id).await {
            Ok(Some(asset)) => asset,
            Ok(None) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Asset not found: {}",
                    request.asset_id
                ))]));
            }
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Repository error: {e}"
                ))]));
            }
        };

        // Check permission
        let org_id = asset.organization_id;
        if let Err(e) = self.check_permission(Permission::StateRefresh, org_id, asset.project_id) {
            return Ok(CallToolResult::error(vec![Content::text(e.to_string())]));
        }

        let previous_state = format!("{:?}", asset.state());

        // Check for upstream changes by comparing current upstream versions
        // with the effective versions recorded in dependencies
        let upstream_assets = self
            .state
            .dependency_repo
            .find_upstream(&asset_id)
            .await
            .map_err(|e| {
                rmcp::Error::internal_error(
                    format!("Failed to find upstream assets: {e}"),
                    None::<serde_json::Value>,
                )
            })?;

        let mut upstream_changes = false;
        for upstream_id in upstream_assets {
            if let Ok(Some(upstream)) = self.state.asset_repo.find_by_id(&upstream_id).await {
                // If upstream has a newer version than what we recorded, there's a change
                if upstream.current_version() != asset.current_version() {
                    upstream_changes = true;
                    break;
                }
            }
        }

        let response = RefreshAssetStateResponse {
            asset_id: asset.id.0.to_string(),
            previous_state: previous_state.clone(),
            current_state: previous_state,
            upstream_changes_detected: upstream_changes,
        };

        match serde_json::to_string(&response) {
            Ok(json) => Ok(CallToolResult::success(vec![Content::text(json)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Serialization error: {e}"
            ))])),
        }
    }

    /// Manual clean asset tool implementation
    async fn manual_clean_asset(
        &self,
        request: ManualCleanAssetRequest,
    ) -> Result<CallToolResult, rmcp::Error> {
        // Parse asset ID
        let asset_id = match parse_uuid(&request.asset_id) {
            Some(id) => AssetId::from_uuid(id),
            None => {
                return Ok(CallToolResult::error(vec![Content::text(
                    "Invalid asset_id format",
                )]));
            }
        };

        // Get asset from repository
        let asset = match self.state.asset_repo.find_by_id(&asset_id).await {
            Ok(Some(asset)) => asset,
            Ok(None) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Asset not found: {}",
                    request.asset_id
                ))]));
            }
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Repository error: {e}"
                ))]));
            }
        };

        // Check permission
        let org_id = asset.organization_id;
        if let Err(e) =
            self.check_permission(Permission::StateManualClean, org_id, asset.project_id)
        {
            return Ok(CallToolResult::error(vec![Content::text(e.to_string())]));
        }

        let previous_state = format!("{:?}", asset.state());

        let unresolved = self
            .state
            .dirty_repo
            .find_unresolved_by_asset(&asset_id)
            .await
            .map_err(|e| {
                rmcp::Error::internal_error(
                    format!("Failed to find unresolved entries: {e}"),
                    None::<serde_json::Value>,
                )
            })?;

        let asset_version = request
            .resolved_version
            .or_else(|| Some(asset.current_version().to_string()))
            .unwrap_or_else(|| "0.0.0".to_string());
        let resolutions = match request.resolutions {
            Some(resolutions) => {
                let mut parsed = Vec::new();
                for resolution in resolutions {
                    let upstream_asset_id = match parse_uuid(&resolution.upstream_asset_id) {
                        Some(id) => AssetId::from_uuid(id),
                        None => {
                            return Ok(CallToolResult::error(vec![Content::text(format!(
                                "Invalid upstream_asset_id format: {}",
                                resolution.upstream_asset_id
                            ))]));
                        }
                    };
                    parsed.push(ManualCleanResolution {
                        upstream_asset_id,
                        from_version: resolution.from_version,
                        to_version: resolution.to_version,
                        review_result: resolution.review_result,
                        comment: resolution.comment.or_else(|| request.review_notes.clone()),
                    });
                }
                parsed
            }
            None => unresolved
                .iter()
                .map(|entry| ManualCleanResolution {
                    upstream_asset_id: entry.upstream_asset_id,
                    from_version: entry.upstream_old_version.clone(),
                    to_version: entry.upstream_version.clone(),
                    review_result: "accepted".to_string(),
                    comment: request.review_notes.clone(),
                })
                .collect(),
        };

        let service = VersionService::new(
            self.state.asset_repo.clone(),
            self.state.dirty_repo.clone(),
            self.state.version_repo.clone(),
            self.state.dependency_repo.clone(),
            self.state.dirty_log_repo.clone(),
        );
        if let Err(e) = service
            .manual_clean(ManualCleanCommand {
                asset_id,
                asset_version,
                reviewed_by: request
                    .reviewed_by
                    .unwrap_or_else(|| self.state.principal.id.clone()),
                resolutions,
            })
            .await
        {
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "Manual clean failed: {e}"
            ))]));
        }

        let review_id = uuid::Uuid::new_v4().to_string();
        let updated = self
            .state
            .asset_repo
            .find_by_id(&asset_id)
            .await
            .map_err(|e| {
                rmcp::Error::internal_error(
                    format!("Failed to reload asset: {e}"),
                    None::<serde_json::Value>,
                )
            })?
            .ok_or_else(|| {
                rmcp::Error::internal_error(
                    "Asset disappeared after clean",
                    None::<serde_json::Value>,
                )
            })?;

        let response = ManualCleanAssetResponse {
            asset_id: asset.id.0.to_string(),
            previous_state,
            current_state: format!("{:?}", updated.state()),
            review_id,
        };

        match serde_json::to_string(&response) {
            Ok(json) => Ok(CallToolResult::success(vec![Content::text(json)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Serialization error: {e}"
            ))])),
        }
    }

    /// Query the state of a workflow instance and its actions (Slice 1, §14).
    async fn query_workflow_state(
        &self,
        request: QueryWorkflowStateRequest,
    ) -> Result<CallToolResult, rmcp::Error> {
        let instance_id = match parse_uuid(&request.workflow_instance_id) {
            Some(id) => adam_domain::workflow::instance::WorkflowInstanceId::from_uuid(id),
            None => {
                return Ok(CallToolResult::error(vec![Content::text(
                    "Invalid workflow_instance_id format",
                )]));
            }
        };

        let instance = match self
            .state
            .workflow_instance_repo
            .find_by_id(&instance_id)
            .await
        {
            Ok(Some(i)) => i,
            Ok(None) => {
                return Ok(CallToolResult::error(vec![Content::text(
                    "Workflow instance not found",
                )]));
            }
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(e.to_string())]));
            }
        };

        // Enforce organization boundary: an instance in another org is treated
        // as not-found so cross-org UUID guessing cannot leak workflow state.
        if instance.organization_id != self.state.principal.organization_id {
            return Ok(CallToolResult::error(vec![Content::text(
                "Workflow instance not found",
            )]));
        }

        let actions = match self
            .state
            .workflow_action_repo
            .find_by_instance(&instance_id)
            .await
        {
            Ok(a) => a,
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(e.to_string())]));
            }
        };

        let response = QueryWorkflowStateResponse {
            instance_id: instance.id.0.to_string(),
            status: instance.status.to_string(),
            template: instance.template.to_string(),
            correlation_id: instance.correlation_id.0.to_string(),
            cascade_depth: instance.cascade_depth,
            actions: actions
                .into_iter()
                .map(|a| WorkflowActionInfo {
                    id: a.id.0.to_string(),
                    action_type: a.action_type.to_string(),
                    status: a.status.to_string(),
                    target_asset_id: a.target_asset_id.map(|x| x.0.to_string()),
                    automation_level: a.automation_level.as_str().to_string(),
                })
                .collect(),
        };

        match serde_json::to_string(&response) {
            Ok(json) => Ok(CallToolResult::success(vec![Content::text(json)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Serialization error: {e}"
            ))])),
        }
    }

    async fn list_pending_agent_tasks(
        &self,
        request: ListPendingAgentTasksRequest,
    ) -> Result<CallToolResult, rmcp::Error> {
        let project_id = match request.project_id.as_deref().map(parse_uuid) {
            Some(Some(id)) => Some(ProjectId::from_uuid(id)),
            Some(None) => {
                return Ok(CallToolResult::error(vec![Content::text(
                    "Invalid project_id format",
                )]));
            }
            None => None,
        };
        let capability = request
            .capability_filter
            .as_deref()
            .map(Capability::new)
            .unwrap_or_else(Capability::create_virtual_asset_context);

        let tasks = match self
            .state
            .agent_task_repo
            .list_queued(
                &self.state.principal.organization_id,
                &capability,
                project_id.as_ref(),
            )
            .await
        {
            Ok(tasks) => tasks,
            Err(e) => return Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        };

        let response = ListPendingAgentTasksResponse {
            total: tasks.len(),
            tasks: tasks.into_iter().map(agent_task_info).collect(),
        };
        json_tool_response(&response)
    }

    async fn claim_agent_task(
        &self,
        request: ClaimAgentTaskRequest,
    ) -> Result<CallToolResult, rmcp::Error> {
        let task_id = match parse_uuid(&request.task_id) {
            Some(id) => AgentTaskId::from_uuid(id),
            None => {
                return Ok(CallToolResult::error(vec![Content::text(
                    "Invalid task_id format",
                )]));
            }
        };

        let existing = match self.state.agent_task_repo.find_by_id(&task_id).await {
            Ok(Some(task)) => task,
            Ok(None) => return Ok(CallToolResult::error(vec![Content::text("Task not found")])),
            Err(e) => return Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        };
        if existing.organization_id != self.state.principal.organization_id {
            return Ok(CallToolResult::error(vec![Content::text("Task not found")]));
        }

        let service = AgentTaskService::new(
            self.state.agent_task_repo.clone(),
            self.state.workflow_action_repo.clone(),
            self.state.workflow_event_repo.clone(),
            self.state.workflow_instance_repo.clone(),
        );
        let claimed = match service
            .claim_task(
                task_id,
                &request.agent_id,
                chrono::Duration::seconds(
                    request
                        .lease_seconds
                        .unwrap_or(DEFAULT_AGENT_TASK_LEASE_SECONDS),
                ),
            )
            .await
        {
            Ok(task) => task,
            Err(e) => return Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        };

        json_tool_response(&ClaimAgentTaskResponse {
            task: claimed.map(agent_task_info),
        })
    }

    async fn submit_agent_task_result(
        &self,
        request: SubmitAgentTaskResultRequest,
    ) -> Result<CallToolResult, rmcp::Error> {
        let task_id = match parse_uuid(&request.task_id) {
            Some(id) => AgentTaskId::from_uuid(id),
            None => {
                return Ok(CallToolResult::error(vec![Content::text(
                    "Invalid task_id format",
                )]));
            }
        };
        let produced_asset_ids = match request
            .produced_asset_ids
            .iter()
            .map(|id| parse_uuid(id))
            .collect::<Option<Vec<_>>>()
        {
            Some(ids) => ids,
            None => {
                return Ok(CallToolResult::error(vec![Content::text(
                    "Invalid produced_asset_ids format",
                )]));
            }
        };

        let existing = match self.state.agent_task_repo.find_by_id(&task_id).await {
            Ok(Some(task)) => task,
            Ok(None) => return Ok(CallToolResult::error(vec![Content::text("Task not found")])),
            Err(e) => return Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        };
        if existing.organization_id != self.state.principal.organization_id {
            return Ok(CallToolResult::error(vec![Content::text("Task not found")]));
        }

        let service = AgentTaskService::new(
            self.state.agent_task_repo.clone(),
            self.state.workflow_action_repo.clone(),
            self.state.workflow_event_repo.clone(),
            self.state.workflow_instance_repo.clone(),
        );
        let task = match service
            .submit_result(task_id, request.result_payload, produced_asset_ids)
            .await
        {
            Ok(task) => task,
            Err(e) => return Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        };

        json_tool_response(&SubmitAgentTaskResultResponse {
            task: agent_task_info(task),
        })
    }
}

fn agent_task_info(task: adam_domain::workflow::AgentTask) -> AgentTaskInfo {
    AgentTaskInfo {
        id: task.id.0.to_string(),
        project_id: task.project_id.map(|p| p.0.to_string()),
        action_id: task.action_id.0.to_string(),
        capability: task.capability.0,
        status: task.status.to_string(),
        agent_id: task.agent_id,
        expires_at: task.expires_at.map(|d| d.to_rfc3339()),
    }
}

fn json_tool_response<T: Serialize>(response: &T) -> Result<CallToolResult, rmcp::Error> {
    match serde_json::to_string(response) {
        Ok(json) => Ok(CallToolResult::success(vec![Content::text(json)])),
        Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
            "Serialization error: {e}"
        ))])),
    }
}

impl ServerHandler for AdamMcpServer {
    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        match request.name.as_ref() {
            "query_assets" => {
                let params: QueryAssetsRequest = match parse_args(request.arguments) {
                    Ok(p) => p,
                    Err(e) => return Ok(e),
                };
                self.query_assets(params).await
            }
            "get_asset" => {
                let params: GetAssetRequest = match parse_args(request.arguments) {
                    Ok(p) => p,
                    Err(e) => return Ok(e),
                };
                self.get_asset(params).await
            }
            "get_asset_content" => {
                let params: GetAssetContentRequest = match parse_args(request.arguments) {
                    Ok(p) => p,
                    Err(e) => return Ok(e),
                };
                self.get_asset_content(params).await
            }
            "get_dependency_graph" => {
                let params: GetDependencyGraphRequest = match parse_args(request.arguments) {
                    Ok(p) => p,
                    Err(e) => return Ok(e),
                };
                self.get_dependency_graph(params).await
            }
            "create_virtual_asset" => {
                let params: CreateVirtualAssetRequest = match parse_args(request.arguments) {
                    Ok(p) => p,
                    Err(e) => return Ok(e),
                };
                self.create_virtual_asset(params).await
            }
            "get_virtual_context" => {
                let params: GetVirtualContextRequest = match parse_args(request.arguments) {
                    Ok(p) => p,
                    Err(e) => return Ok(e),
                };
                self.get_virtual_context(params).await
            }
            "publish_asset" => {
                let params: PublishAssetRequest = match parse_args(request.arguments) {
                    Ok(p) => p,
                    Err(e) => return Ok(e),
                };
                self.publish_asset(params).await
            }
            "suggest_version" => {
                let params: SuggestVersionRequest = match parse_args(request.arguments) {
                    Ok(p) => p,
                    Err(e) => return Ok(e),
                };
                self.suggest_version(params).await
            }
            "refresh_asset_state" => {
                let params: RefreshAssetStateRequest = match parse_args(request.arguments) {
                    Ok(p) => p,
                    Err(e) => return Ok(e),
                };
                self.refresh_asset_state(params).await
            }
            "manual_clean_asset" => {
                let params: ManualCleanAssetRequest = match parse_args(request.arguments) {
                    Ok(p) => p,
                    Err(e) => return Ok(e),
                };
                self.manual_clean_asset(params).await
            }
            "query_workflow_state" => {
                let params: QueryWorkflowStateRequest = match parse_args(request.arguments) {
                    Ok(p) => p,
                    Err(e) => return Ok(e),
                };
                self.query_workflow_state(params).await
            }
            "list_pending_agent_tasks" => {
                let params: ListPendingAgentTasksRequest = match parse_args(request.arguments) {
                    Ok(p) => p,
                    Err(e) => return Ok(e),
                };
                self.list_pending_agent_tasks(params).await
            }
            "claim_agent_task" => {
                let params: ClaimAgentTaskRequest = match parse_args(request.arguments) {
                    Ok(p) => p,
                    Err(e) => return Ok(e),
                };
                self.claim_agent_task(params).await
            }
            "submit_agent_task_result" => {
                let params: SubmitAgentTaskResultRequest = match parse_args(request.arguments) {
                    Ok(p) => p,
                    Err(e) => return Ok(e),
                };
                self.submit_agent_task_result(params).await
            }
            _ => {
                let msg = format!("Unknown tool: {}", request.name);
                Ok(CallToolResult::error(vec![Content::text(msg)]))
            }
        }
    }

    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListToolsResult, rmcp::ErrorData> {
        let schema = schemars::schema_for!(QueryAssetsRequest);
        let schema_json = serde_json::to_value(schema).unwrap();
        let query_schema: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(schema_json).unwrap();

        let schema = schemars::schema_for!(GetAssetRequest);
        let schema_json = serde_json::to_value(schema).unwrap();
        let get_asset_schema: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(schema_json).unwrap();

        let schema = schemars::schema_for!(GetAssetContentRequest);
        let schema_json = serde_json::to_value(schema).unwrap();
        let get_content_schema: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(schema_json).unwrap();

        let schema = schemars::schema_for!(GetDependencyGraphRequest);
        let schema_json = serde_json::to_value(schema).unwrap();
        let get_deps_schema: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(schema_json).unwrap();

        let schema = schemars::schema_for!(CreateVirtualAssetRequest);
        let schema_json = serde_json::to_value(schema).unwrap();
        let create_schema: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(schema_json).unwrap();

        let schema = schemars::schema_for!(GetVirtualContextRequest);
        let schema_json = serde_json::to_value(schema).unwrap();
        let get_virtual_schema: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(schema_json).unwrap();

        let schema = schemars::schema_for!(PublishAssetRequest);
        let schema_json = serde_json::to_value(schema).unwrap();
        let publish_schema: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(schema_json).unwrap();

        let schema = schemars::schema_for!(SuggestVersionRequest);
        let schema_json = serde_json::to_value(schema).unwrap();
        let suggest_schema: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(schema_json).unwrap();

        let schema = schemars::schema_for!(RefreshAssetStateRequest);
        let schema_json = serde_json::to_value(schema).unwrap();
        let refresh_schema: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(schema_json).unwrap();

        let schema = schemars::schema_for!(ManualCleanAssetRequest);
        let schema_json = serde_json::to_value(schema).unwrap();
        let clean_schema: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(schema_json).unwrap();

        let schema = schemars::schema_for!(QueryWorkflowStateRequest);
        let schema_json = serde_json::to_value(schema).unwrap();
        let workflow_state_schema: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(schema_json).unwrap();

        let schema = schemars::schema_for!(ListPendingAgentTasksRequest);
        let schema_json = serde_json::to_value(schema).unwrap();
        let list_agent_tasks_schema: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(schema_json).unwrap();

        let schema = schemars::schema_for!(ClaimAgentTaskRequest);
        let schema_json = serde_json::to_value(schema).unwrap();
        let claim_agent_task_schema: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(schema_json).unwrap();

        let schema = schemars::schema_for!(SubmitAgentTaskResultRequest);
        let schema_json = serde_json::to_value(schema).unwrap();
        let submit_agent_task_schema: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(schema_json).unwrap();

        let tools = vec![
            Tool::new(
                "query_assets",
                "Query assets within a project scope, including project-level and organization-level assets",
                Arc::new(query_schema),
            ),
            Tool::new(
                "get_asset",
                "Get asset details by ID",
                Arc::new(get_asset_schema),
            ),
            Tool::new(
                "get_asset_content",
                "Get asset content reference (stored externally)",
                Arc::new(get_content_schema),
            ),
            Tool::new(
                "get_dependency_graph",
                "Get upstream or downstream dependencies for an asset",
                Arc::new(get_deps_schema),
            ),
            Tool::new(
                "create_virtual_asset",
                "Create a temporary virtual asset for AI context with anchor references",
                Arc::new(create_schema),
            ),
            Tool::new(
                "get_virtual_context",
                "Get virtual asset context with anchors and upstream dependencies",
                Arc::new(get_virtual_schema),
            ),
            Tool::new(
                "publish_asset",
                "Publish a new version of an asset",
                Arc::new(publish_schema),
            ),
            Tool::new(
                "suggest_version",
                "Get version suggestion for asset based on change type",
                Arc::new(suggest_schema),
            ),
            Tool::new(
                "refresh_asset_state",
                "Refresh asset state and check for upstream changes",
                Arc::new(refresh_schema),
            ),
            Tool::new(
                "manual_clean_asset",
                "Manually clean an asset after reviewing upstream changes",
                Arc::new(clean_schema),
            ),
            Tool::new(
                "query_workflow_state",
                "Query the state of a workflow instance and its actions by workflow_instance_id",
                Arc::new(workflow_state_schema),
            ),
            Tool::new(
                "list_pending_agent_tasks",
                "List queued agent tasks by optional project_id and capability_filter",
                Arc::new(list_agent_tasks_schema),
            ),
            Tool::new(
                "claim_agent_task",
                "Atomically claim a queued agent task and set its lease",
                Arc::new(claim_agent_task_schema),
            ),
            Tool::new(
                "submit_agent_task_result",
                "Submit an agent task result and complete the parent workflow action",
                Arc::new(submit_agent_task_schema),
            ),
        ];
        Ok(rmcp::model::ListToolsResult {
            tools,
            next_cursor: None,
        })
    }

    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: Implementation {
                name: "ADAM MCP Server".into(),
                version: "0.1.0".into(),
            },
            instructions: Some(
                "ADAM Agent-oriented Development Asset Management MCP Server for querying and managing R&D assets".into(),
            ),
            ..Default::default()
        }
    }
}

// ============================================================================
// MCP Argument Parsing Helper
// ============================================================================

/// Parse arguments from CallToolRequestParam into a typed struct.
/// Returns a CallToolResult error on failure, making it easy to use in call_tool handlers.
fn parse_args<T: for<'de> serde::Deserialize<'de>>(
    args: Option<serde_json::Map<String, serde_json::Value>>,
) -> Result<T, CallToolResult> {
    match args {
        None => Err(CallToolResult::error(vec![Content::text(
            "Missing arguments",
        )])),
        Some(map) => {
            let value = serde_json::Value::Object(map);
            match serde_json::from_value(value) {
                Ok(parsed) => Ok(parsed),
                Err(e) => Err(CallToolResult::error(vec![Content::text(format!(
                    "Invalid parameters: {e}"
                ))])),
            }
        }
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Parse UUID from string
fn parse_uuid(s: &str) -> Option<uuid::Uuid> {
    uuid::Uuid::parse_str(s).ok()
}

/// Convert AssetInstance to AssetInfo
fn asset_to_info(asset: &AssetInstance) -> AssetInfo {
    AssetInfo {
        id: asset.id.0.to_string(),
        name: asset.name.clone(),
        asset_type: asset.asset_type_id.0.to_string(),
        state: format!("{:?}", asset.state()),
        level: format!("{:?}", asset.level),
    }
}

/// Check if asset matches query filters
fn matches_filters(asset: &AssetInstance, request: &QueryAssetsRequest) -> bool {
    // Filter by asset type
    if let Some(ref asset_type) = request.asset_type {
        if !asset.asset_type_id.0.to_string().contains(asset_type) {
            return false;
        }
    }

    // Filter by state
    if let Some(ref state) = request.state {
        let asset_state = format!("{:?}", asset.state()).to_lowercase();
        if !asset_state.contains(&state.to_lowercase()) {
            return false;
        }
    }

    // Filter by name contains
    if let Some(ref name_pattern) = request.name_contains {
        if !asset
            .name
            .to_lowercase()
            .contains(&name_pattern.to_lowercase())
        {
            return false;
        }
    }

    true
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use adam_application::services::workflow::AgentTaskService;
    use adam_domain::workflow::action::CreateActionCommand;
    use adam_domain::workflow::instance::{CreateInstanceCommand, WorkflowTemplate};
    use adam_domain::workflow::repository::{WorkflowActionRepository, WorkflowInstanceRepository};
    use adam_domain::workflow::rule::{ActionType, AutomationLevel};
    use adam_domain::workflow::state_machine::{ActionStatus, InstanceStatus};
    use adam_domain::workflow::{Capability, CompensationPolicy, CorrelationId};
    use adam_domain::{
        AssetState, CreateAssetCommand, InMemoryAssetRepository, InMemoryDirtyQueueRepository,
        InMemoryVirtualInstanceRepository, Role,
    };

    fn create_test_state_with_role(role: Role) -> McpServerState {
        use adam_domain::workflow::in_memory::{
            InMemoryAgentTaskRepository, InMemoryPromotionRuleRepository,
            InMemoryWorkflowActionRepository, InMemoryWorkflowEventRepository,
            InMemoryWorkflowInstanceRepository,
        };
        let org_id = OrganizationId::new();
        let project_id = ProjectId::new();

        McpServerState {
            asset_repo: Arc::new(InMemoryAssetRepository::new()),
            dependency_repo: Arc::new(adam_domain::InMemoryDependencyRepository::new()),
            dependency_rule_repo: Arc::new(adam_domain::InMemoryDependencyRuleRepository::new()),
            dirty_repo: Arc::new(InMemoryDirtyQueueRepository::new()),
            version_repo: Arc::new(adam_domain::InMemoryAssetVersionRepository::new()),
            dirty_log_repo: Arc::new(adam_domain::InMemoryDirtyResolutionLogRepository::new()),
            virtual_repo: Arc::new(InMemoryVirtualInstanceRepository::new()),
            workflow_event_repo: Arc::new(InMemoryWorkflowEventRepository::default()),
            workflow_rule_repo: Arc::new(InMemoryPromotionRuleRepository::default()),
            workflow_instance_repo: Arc::new(InMemoryWorkflowInstanceRepository::default()),
            workflow_action_repo: Arc::new(InMemoryWorkflowActionRepository::default()),
            agent_task_repo: Arc::new(InMemoryAgentTaskRepository::default()),
            principal: AuthPrincipal {
                id: "test-user".to_string(),
                organization_id: org_id,
                project_memberships: vec![project_id],
                roles: vec![role],
            },
        }
    }

    fn create_test_state() -> McpServerState {
        create_test_state_with_role(Role::Developer)
    }

    async fn seed_ready_agent_task(state: &McpServerState) -> AgentTaskId {
        let instance = state
            .workflow_instance_repo
            .create(&CreateInstanceCommand {
                organization_id: state.principal.organization_id,
                project_id: None,
                correlation_id: CorrelationId::new(),
                template: WorkflowTemplate::Feature,
                cascade_depth: 0,
            })
            .await
            .unwrap();
        state
            .workflow_instance_repo
            .update_cas(&instance.id, instance.lock_version, InstanceStatus::Ready)
            .await
            .unwrap();

        let action = state
            .workflow_action_repo
            .create(&CreateActionCommand {
                organization_id: state.principal.organization_id,
                instance_id: instance.id,
                action_type: ActionType::UpsertWorkItem,
                target_asset_id: Some(AssetId::new()),
                target_asset_type_id: Some(AssetTypeId::new()),
                idempotency_key: format!("action:{}", uuid::Uuid::new_v4()),
                preconditions: serde_json::json!([]),
                postconditions: serde_json::json!({}),
                automation_level: AutomationLevel::AgentSuggested,
                is_required: true,
                order_index: 0,
                compensation_action_type: None,
                compensation_payload: None,
                compensation_policy: CompensationPolicy::None,
                max_retries: 3,
            })
            .await
            .unwrap();
        let mut ready = action.clone();
        ready.status = ActionStatus::Ready;
        state
            .workflow_action_repo
            .update_cas(&ready, action.lock_version)
            .await
            .unwrap();

        AgentTaskService::new(
            state.agent_task_repo.clone(),
            state.workflow_action_repo.clone(),
            state.workflow_event_repo.clone(),
            state.workflow_instance_repo.clone(),
        )
        .create_task_for_action(action.id, Capability::create_virtual_asset_context())
        .await
        .unwrap()
        .id
    }

    #[tokio::test]
    async fn query_assets_tool_returns_assets() {
        let state = create_test_state();
        let server = AdamMcpServer::new(state.clone());

        // Create a test project and assets
        let project_id = state.principal.project_memberships[0];
        let org_id = state.principal.organization_id;

        // Create a project-level asset
        let cmd = CreateAssetCommand {
            name: "Test Asset".to_string(),
            asset_type_id: AssetTypeId::new(),
            project_id: Some(project_id),
            organization_id: org_id,
            level: adam_domain::dependency::boundary::AssetLevel::Project,
            external_ref: "https://example.com/asset".to_string(),
            source: "manual".to_string(),
            metadata: serde_json::json!({}),
            idempotency_key: None,
        };
        state.asset_repo.create(&cmd).await.unwrap();

        // Query assets
        let request = QueryAssetsRequest {
            project_id: project_id.0.to_string(),
            asset_type: None,
            state: None,
            name_contains: None,
        };

        let result = server.query_assets(request).await;
        assert!(result.is_ok());

        let tool_result = result.unwrap();
        assert!(!tool_result.is_error.unwrap_or(true));

        // Parse the content
        let content_text = &tool_result.content[0].as_text().unwrap().text;
        let response: QueryAssetsResponse = serde_json::from_str(content_text).unwrap();
        assert_eq!(response.total, 1);
        assert_eq!(response.assets[0].name, "Test Asset");
    }

    #[tokio::test]
    async fn query_assets_with_name_filter() {
        let state = create_test_state();
        let server = AdamMcpServer::new(state.clone());

        let project_id = state.principal.project_memberships[0];
        let org_id = state.principal.organization_id;

        // Create multiple assets
        state
            .asset_repo
            .create(&CreateAssetCommand {
                name: "User Service API".to_string(),
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

        state
            .asset_repo
            .create(&CreateAssetCommand {
                name: "Order Service".to_string(),
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

        // Query with name filter
        let request = QueryAssetsRequest {
            project_id: project_id.0.to_string(),
            asset_type: None,
            state: None,
            name_contains: Some("User".to_string()),
        };

        let result = server.query_assets(request).await;
        assert!(result.is_ok());

        let tool_result = result.unwrap();
        let content_text = &tool_result.content[0].as_text().unwrap().text;
        let response: QueryAssetsResponse = serde_json::from_str(content_text).unwrap();
        assert_eq!(response.total, 1);
        assert_eq!(response.assets[0].name, "User Service API");
    }

    #[tokio::test]
    async fn create_virtual_asset_denied_without_permission() {
        // Reader role does NOT have QueryVirtualContext permission
        let state = create_test_state_with_role(Role::Reader);
        let server = AdamMcpServer::new(state.clone());

        let request = CreateVirtualAssetRequest {
            target_type: "code_commit".to_string(),
            anchors: vec![],
            project_id: state.principal.project_memberships[0].0.to_string(),
        };

        let result = server.create_virtual_asset(request).await;
        assert!(result.is_ok());

        let tool_result = result.unwrap();
        // Reader should NOT have QueryVirtualContext permission
        assert!(tool_result.is_error.unwrap_or(false));
        let error_text = &tool_result.content[0].as_text().unwrap().text;
        assert!(error_text.contains("Permission denied") || error_text.contains("Access denied"));
    }

    #[tokio::test]
    async fn create_virtual_asset_with_valid_anchor() {
        let state = create_test_state();
        let server = AdamMcpServer::new(state.clone());

        // Create an anchor asset
        let project_id = state.principal.project_memberships[0];
        let org_id = state.principal.organization_id;
        let anchor = state
            .asset_repo
            .create(&CreateAssetCommand {
                name: "Anchor Asset".to_string(),
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

        let request = CreateVirtualAssetRequest {
            target_type: "code_commit".to_string(),
            anchors: vec![anchor.id.0.to_string()],
            project_id: project_id.0.to_string(),
        };

        let result = server.create_virtual_asset(request).await;
        assert!(result.is_ok());

        let tool_result = result.unwrap();
        assert!(!tool_result.is_error.unwrap_or(true));

        let content_text = &tool_result.content[0].as_text().unwrap().text;
        let response: CreateVirtualAssetResponse = serde_json::from_str(content_text).unwrap();
        assert!(!response.virtual_asset_id.is_empty());
        assert!(response.context_summary.contains("code_commit"));

        // Verify the virtual instance was persisted
        let virtual_id = adam_domain::VirtualInstanceId(
            uuid::Uuid::parse_str(&response.virtual_asset_id).unwrap(),
        );
        let persisted = state.virtual_repo.find_by_id(&virtual_id).await.unwrap();
        assert!(persisted.is_some());
        assert_eq!(persisted.unwrap().anchors.len(), 1);
    }

    #[tokio::test]
    async fn create_virtual_asset_with_invalid_anchor_returns_error() {
        let state = create_test_state();
        let server = AdamMcpServer::new(state.clone());

        let request = CreateVirtualAssetRequest {
            target_type: "code_commit".to_string(),
            anchors: vec!["invalid-uuid".to_string()],
            project_id: state.principal.project_memberships[0].0.to_string(),
        };

        let result = server.create_virtual_asset(request).await;
        assert!(result.is_ok());

        let tool_result = result.unwrap();
        assert!(tool_result.is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn create_virtual_asset_with_missing_anchor_returns_error() {
        let state = create_test_state();
        let server = AdamMcpServer::new(state.clone());

        let request = CreateVirtualAssetRequest {
            target_type: "code_commit".to_string(),
            anchors: vec![uuid::Uuid::new_v4().to_string()], // Valid UUID but non-existent
            project_id: state.principal.project_memberships[0].0.to_string(),
        };

        let result = server.create_virtual_asset(request).await;
        assert!(result.is_ok());

        let tool_result = result.unwrap();
        assert!(tool_result.is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn create_virtual_asset_with_empty_anchors_returns_error() {
        let state = create_test_state();
        let server = AdamMcpServer::new(state.clone());

        let request = CreateVirtualAssetRequest {
            target_type: "code_commit".to_string(),
            anchors: vec![],
            project_id: state.principal.project_memberships[0].0.to_string(),
        };

        let result = server.create_virtual_asset(request).await;
        assert!(result.is_ok());

        let tool_result = result.unwrap();
        assert!(tool_result.is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn create_virtual_asset_cross_project_denied() {
        let state = create_test_state();
        let server = AdamMcpServer::new(state.clone());

        // Create anchor in project A
        let project_a = state.principal.project_memberships[0];
        let org_id = state.principal.organization_id;
        let anchor = state
            .asset_repo
            .create(&CreateAssetCommand {
                name: "Anchor Asset".to_string(),
                asset_type_id: AssetTypeId::new(),
                project_id: Some(project_a),
                organization_id: org_id,
                level: adam_domain::dependency::boundary::AssetLevel::Project,
                external_ref: "https://example.com/asset".to_string(),
                source: "manual".to_string(),
                metadata: serde_json::json!({}),
                idempotency_key: None,
            })
            .await
            .unwrap();

        // Try to create virtual asset in project B using anchor from project A
        let project_b = ProjectId::new();
        let request = CreateVirtualAssetRequest {
            target_type: "code_commit".to_string(),
            anchors: vec![anchor.id.0.to_string()],
            project_id: project_b.0.to_string(),
        };

        let result = server.create_virtual_asset(request).await;
        assert!(result.is_ok());

        let tool_result = result.unwrap();
        // Should fail because principal is not member of project B
        assert!(tool_result.is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn create_virtual_asset_persists_instance() {
        let state = create_test_state();
        let server = AdamMcpServer::new(state.clone());

        let project_id = state.principal.project_memberships[0];
        let org_id = state.principal.organization_id;

        // Create anchor
        let anchor = state
            .asset_repo
            .create(&CreateAssetCommand {
                name: "Anchor Asset".to_string(),
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

        let request = CreateVirtualAssetRequest {
            target_type: "code_commit".to_string(),
            anchors: vec![anchor.id.0.to_string()],
            project_id: project_id.0.to_string(),
        };

        let result = server.create_virtual_asset(request).await;
        assert!(result.is_ok());

        let tool_result = result.unwrap();
        let content_text = &tool_result.content[0].as_text().unwrap().text;
        let response: CreateVirtualAssetResponse = serde_json::from_str(content_text).unwrap();

        // Verify it was persisted
        let virtual_id = adam_domain::VirtualInstanceId(
            uuid::Uuid::parse_str(&response.virtual_asset_id).unwrap(),
        );
        let instance = state
            .virtual_repo
            .find_by_id(&virtual_id)
            .await
            .unwrap()
            .expect("Virtual instance should be persisted");

        assert_eq!(instance.project_id, project_id);
        assert_eq!(instance.organization_id, org_id);
        assert_eq!(instance.created_by, "test-user");
        assert_eq!(instance.anchors.len(), 1);
    }

    #[tokio::test]
    async fn publish_asset_publishes_version() {
        let state = create_test_state();
        let server = AdamMcpServer::new(state.clone());

        // Create an asset
        let project_id = state.principal.project_memberships[0];
        let org_id = state.principal.organization_id;
        let asset = state
            .asset_repo
            .create(&CreateAssetCommand {
                name: "Test Asset".to_string(),
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

        let request = PublishAssetRequest {
            asset_id: asset.id.0.to_string(),
            version: Some("1.0.0".to_string()),
            dependencies: None,
            relationships: None,
            propagation_policies: None,
        };

        let result = server.publish_asset(request).await;
        assert!(result.is_ok());

        let tool_result = result.unwrap();
        assert!(!tool_result.is_error.unwrap_or(true));

        let content_text = &tool_result.content[0].as_text().unwrap().text;
        let response: PublishAssetResponse = serde_json::from_str(content_text).unwrap();
        assert_eq!(response.version, "1.0.0");

        let versions = state.version_repo.find_by_asset(&asset.id).await.unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].version_number, "1.0.0");

        let updated = state
            .asset_repo
            .find_by_id(&asset.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.current_version().to_string(), "1.0.0");
    }

    #[tokio::test]
    async fn suggest_version_uses_current_persisted_version() {
        let state = create_test_state();
        let server = AdamMcpServer::new(state.clone());

        let project_id = state.principal.project_memberships[0];
        let org_id = state.principal.organization_id;
        let asset = state
            .asset_repo
            .create(&CreateAssetCommand {
                name: "Versioned Asset".to_string(),
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
        state
            .asset_repo
            .update_publication(
                &asset.id,
                "1.2.3".to_string(),
                "publisher".to_string(),
                AssetState::Clean,
            )
            .await
            .unwrap();

        let result = server
            .suggest_version(SuggestVersionRequest {
                asset_id: asset.id.0.to_string(),
                change_type: Some("patch".to_string()),
            })
            .await
            .unwrap();

        assert!(!result.is_error.unwrap_or(true));
        let content_text = &result.content[0].as_text().unwrap().text;
        let response: SuggestVersionResponse = serde_json::from_str(content_text).unwrap();
        assert_eq!(response.current_version, Some("1.2.3".to_string()));
        assert_eq!(response.suggested_version, "1.2.4");
    }

    #[tokio::test]
    async fn publish_asset_invalid_id_returns_error() {
        let state = create_test_state();
        let server = AdamMcpServer::new(state.clone());

        let request = PublishAssetRequest {
            asset_id: "invalid-uuid".to_string(),
            version: Some("1.0.0".to_string()),
            dependencies: None,
            relationships: None,
            propagation_policies: None,
        };

        let result = server.publish_asset(request).await;
        assert!(result.is_ok());

        let tool_result = result.unwrap();
        assert!(tool_result.is_error.unwrap_or(false));
        let error_text = &tool_result.content[0].as_text().unwrap().text;
        assert!(error_text.contains("Invalid asset_id"));
    }

    #[tokio::test]
    async fn publish_asset_nonexistent_asset_returns_error() {
        let state = create_test_state();
        let server = AdamMcpServer::new(state.clone());

        let request = PublishAssetRequest {
            asset_id: uuid::Uuid::new_v4().to_string(),
            version: Some("1.0.0".to_string()),
            dependencies: None,
            relationships: None,
            propagation_policies: None,
        };

        let result = server.publish_asset(request).await;
        assert!(result.is_ok());

        let tool_result = result.unwrap();
        assert!(tool_result.is_error.unwrap_or(false));
        let error_text = &tool_result.content[0].as_text().unwrap().text;
        assert!(error_text.contains("Asset not found"));
    }

    #[tokio::test]
    async fn publish_asset_without_permission_denied() {
        // Reader role does NOT have VersionPublish permission
        let state = create_test_state_with_role(Role::Reader);
        let server = AdamMcpServer::new(state.clone());

        // Create an asset
        let project_id = state.principal.project_memberships[0];
        let org_id = state.principal.organization_id;
        let asset = state
            .asset_repo
            .create(&CreateAssetCommand {
                name: "Test Asset".to_string(),
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

        let request = PublishAssetRequest {
            asset_id: asset.id.0.to_string(),
            version: Some("1.0.0".to_string()),
            dependencies: None,
            relationships: None,
            propagation_policies: None,
        };

        let result = server.publish_asset(request).await;
        assert!(result.is_ok());

        let tool_result = result.unwrap();
        assert!(tool_result.is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn publish_asset_propagates_to_downstream() {
        let state = create_test_state();
        let server = AdamMcpServer::new(state.clone());

        // Create upstream asset
        let project_id = state.principal.project_memberships[0];
        let org_id = state.principal.organization_id;
        let upstream = state
            .asset_repo
            .create(&CreateAssetCommand {
                name: "Upstream Asset".to_string(),
                asset_type_id: AssetTypeId::new(),
                project_id: Some(project_id),
                organization_id: org_id,
                level: adam_domain::dependency::boundary::AssetLevel::Project,
                external_ref: "https://example.com/upstream".to_string(),
                source: "manual".to_string(),
                metadata: serde_json::json!({}),
                idempotency_key: None,
            })
            .await
            .unwrap();

        // Create downstream asset
        let downstream = state
            .asset_repo
            .create(&CreateAssetCommand {
                name: "Downstream Asset".to_string(),
                asset_type_id: AssetTypeId::new(),
                project_id: Some(project_id),
                organization_id: org_id,
                level: adam_domain::dependency::boundary::AssetLevel::Project,
                external_ref: "https://example.com/downstream".to_string(),
                source: "manual".to_string(),
                metadata: serde_json::json!({}),
                idempotency_key: None,
            })
            .await
            .unwrap();

        // Create dependency: downstream depends on upstream
        state
            .dependency_repo
            .create_dependency(&downstream.id, &upstream.id)
            .await
            .unwrap();

        // Publish upstream asset
        let request = PublishAssetRequest {
            asset_id: upstream.id.0.to_string(),
            version: Some("2.0.0".to_string()),
            dependencies: None,
            relationships: None,
            propagation_policies: None,
        };

        let result = server.publish_asset(request).await;
        assert!(result.is_ok());

        let tool_result = result.unwrap();
        assert!(!tool_result.is_error.unwrap_or(true));

        // Verify downstream asset is marked as dirty
        let downstream_asset = state
            .asset_repo
            .find_by_id(&downstream.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(downstream_asset.state(), AssetState::Dirty);

        // Verify dirty queue entry was created
        let dirty_entries = state
            .dirty_repo
            .find_unresolved_by_asset(&downstream.id)
            .await
            .unwrap();
        assert_eq!(dirty_entries.len(), 1);
        assert_eq!(dirty_entries[0].upstream_version, "2.0.0");
    }

    #[tokio::test]
    async fn manual_clean_asset_logs_review() {
        let state = create_test_state();
        let server = AdamMcpServer::new(state.clone());

        let project_id = state.principal.project_memberships[0];
        let org_id = state.principal.organization_id;
        let asset = state
            .asset_repo
            .create(&CreateAssetCommand {
                name: "Dirty Asset".to_string(),
                asset_type_id: AssetTypeId::new(),
                project_id: Some(project_id),
                organization_id: org_id,
                level: adam_domain::dependency::boundary::AssetLevel::Project,
                external_ref: "https://example.com/dirty".to_string(),
                source: "manual".to_string(),
                metadata: serde_json::json!({}),
                idempotency_key: None,
            })
            .await
            .unwrap();
        state
            .asset_repo
            .update_state(&asset.id, AssetState::Dirty)
            .await
            .unwrap();
        let upstream_id = AssetId::new();
        state
            .dependency_repo
            .create_dependency_record(
                &adam_domain::AssetDependencyRecord::new(adam_domain::NewDependencyRecord {
                    source_id: asset.id,
                    target_id: upstream_id,
                    relationship: adam_domain::RelationshipType::DependsOn,
                    declared_constraint: adam_domain::VersionConstraint::parse("^1.0.0")
                        .unwrap_or_else(|_| {
                            adam_domain::VersionConstraint::Exact(adam_domain::SemVer::new(1, 0, 0))
                        }),
                    effective_version: adam_domain::SemVer::parse("1.0.0")
                        .unwrap_or_else(|_| adam_domain::SemVer::new(0, 0, 0)),
                    updated_by: "publisher".to_string(),
                    effective_reason: adam_domain::EffectiveUpdateReason::Publish,
                })
                .with_constraint_str("1.0.0"),
            )
            .await
            .unwrap();
        state
            .dirty_repo
            .upsert(&adam_domain::DirtyQueueEntry {
                id: uuid::Uuid::new_v4(),
                asset_id: asset.id,
                upstream_asset_id: upstream_id,
                upstream_version: "1.1.0".to_string(),
                upstream_old_version: "1.0.0".to_string(),
                impact_level: "medium".to_string(),
                since: chrono::Utc::now(),
                created_at: chrono::Utc::now(),
                resolved_at: None,
            })
            .await
            .unwrap();

        let result = server
            .manual_clean_asset(ManualCleanAssetRequest {
                asset_id: asset.id.0.to_string(),
                resolved_version: Some("1.0.1".to_string()),
                reviewed_by: Some("reviewer".to_string()),
                resolutions: None,
                review_notes: Some("no impact".to_string()),
            })
            .await
            .unwrap();

        assert!(!result.is_error.unwrap_or(true));
        let logs = state.dirty_log_repo.find_by_asset(&asset.id).await.unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].reviewed_by, "reviewer");
        assert_eq!(logs[0].comment, Some("no impact".to_string()));
    }

    #[tokio::test]
    async fn get_virtual_context_includes_anchor_upstream() {
        let state = create_test_state();
        let server = AdamMcpServer::new(state.clone());

        let project_id = state.principal.project_memberships[0];
        let org_id = state.principal.organization_id;
        let upstream = state
            .asset_repo
            .create(&CreateAssetCommand {
                name: "Upstream Context".to_string(),
                asset_type_id: AssetTypeId::new(),
                project_id: Some(project_id),
                organization_id: org_id,
                level: adam_domain::dependency::boundary::AssetLevel::Project,
                external_ref: "https://example.com/upstream".to_string(),
                source: "manual".to_string(),
                metadata: serde_json::json!({}),
                idempotency_key: None,
            })
            .await
            .unwrap();
        let anchor = state
            .asset_repo
            .create(&CreateAssetCommand {
                name: "Anchor Context".to_string(),
                asset_type_id: AssetTypeId::new(),
                project_id: Some(project_id),
                organization_id: org_id,
                level: adam_domain::dependency::boundary::AssetLevel::Project,
                external_ref: "https://example.com/anchor".to_string(),
                source: "manual".to_string(),
                metadata: serde_json::json!({}),
                idempotency_key: None,
            })
            .await
            .unwrap();
        state
            .dependency_repo
            .create_dependency(&anchor.id, &upstream.id)
            .await
            .unwrap();

        let create_result = server
            .create_virtual_asset(CreateVirtualAssetRequest {
                target_type: AssetTypeId::new().0.to_string(),
                anchors: vec![anchor.id.0.to_string()],
                project_id: project_id.0.to_string(),
            })
            .await
            .unwrap();
        let create_text = &create_result.content[0].as_text().unwrap().text;
        let created: CreateVirtualAssetResponse = serde_json::from_str(create_text).unwrap();

        let result = server
            .get_virtual_context(GetVirtualContextRequest {
                virtual_asset_id: created.virtual_asset_id,
            })
            .await
            .unwrap();

        assert!(!result.is_error.unwrap_or(true));
        let content_text = &result.content[0].as_text().unwrap().text;
        let context: GetVirtualContextResponse = serde_json::from_str(content_text).unwrap();
        assert!(
            context.context_assets.iter().any(|asset| {
                asset.id == anchor.id.0.to_string() && asset.relevance == "anchor"
            })
        );
        assert!(context.context_assets.iter().any(|asset| {
            asset.id == upstream.id.0.to_string() && asset.relevance == "upstream"
        }));
    }

    #[tokio::test]
    async fn agent_task_tools_list_claim_and_submit_result() {
        let state = create_test_state_with_role(Role::AiAgent);
        let server = AdamMcpServer::new(state.clone());
        let task_id = seed_ready_agent_task(&state).await;

        let list_result = server
            .list_pending_agent_tasks(ListPendingAgentTasksRequest {
                project_id: None,
                capability_filter: Some("create_virtual_asset_context".to_string()),
            })
            .await
            .unwrap();
        assert!(!list_result.is_error.unwrap_or(true));
        let content_text = &list_result.content[0].as_text().unwrap().text;
        let listed: ListPendingAgentTasksResponse = serde_json::from_str(content_text).unwrap();
        assert_eq!(listed.total, 1);
        assert_eq!(listed.tasks[0].id, task_id.0.to_string());

        let claim_result = server
            .claim_agent_task(ClaimAgentTaskRequest {
                task_id: task_id.0.to_string(),
                agent_id: "agent-1".to_string(),
                lease_seconds: Some(60),
            })
            .await
            .unwrap();
        assert!(!claim_result.is_error.unwrap_or(true));
        let content_text = &claim_result.content[0].as_text().unwrap().text;
        let claimed: ClaimAgentTaskResponse = serde_json::from_str(content_text).unwrap();
        assert_eq!(claimed.task.unwrap().status, "claimed");

        let submit_result = server
            .submit_agent_task_result(SubmitAgentTaskResultRequest {
                task_id: task_id.0.to_string(),
                result_payload: serde_json::json!({"ok": true}),
                produced_asset_ids: vec![uuid::Uuid::new_v4().to_string()],
            })
            .await
            .unwrap();
        assert!(!submit_result.is_error.unwrap_or(true));
        let content_text = &submit_result.content[0].as_text().unwrap().text;
        let submitted: SubmitAgentTaskResultResponse = serde_json::from_str(content_text).unwrap();
        assert_eq!(submitted.task.status, "succeeded");
    }
}
