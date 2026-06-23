//! Workflow automation REST handlers (Slice 1).
//!
//! Endpoints (design §13 Slice 1):
//! - `GET  /api/workflow/events?project_id=&correlation_id=&asset_id=`
//! - `GET  /api/workflow/instances/{workflow_instance_id}`
//! - `GET  /api/workflow/actions?project_id=&status=&target_asset_id=`
//! - `POST /api/workflow/events` (requires `Idempotency-Key` header)
//!
//! Handlers construct the generic application services inline from the trait
//! objects held in [`super::AppState`]; the `Arc<dyn Workflow...Repository>`
//! blanket-impls satisfy the services' `?Sized` repo bounds.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use adam_application::services::workflow::{
    AgentTaskService, AgentTaskServiceError, AppendEventRequest, PromotionRuleEvaluator,
    RuleEvaluatorError, WorkflowEventService, WorkflowEventServiceError,
};
use adam_domain::workflow::agent_task::{AgentTask, AgentTaskId, Capability};
use adam_domain::workflow::event::{CorrelationId, EventType};
use adam_domain::workflow::instance::WorkflowInstanceId;
use adam_domain::workflow::repository::{
    AgentTaskRepository, WorkflowActionRepository, WorkflowEventRepository,
    WorkflowInstanceRepository,
};
use adam_domain::workflow::state_machine::AgentTaskStatus;
use adam_domain::{AssetId, AssetTypeId, ProjectId};

use super::{ApiError, AppState, ExtractAuth};

/// Default agent-task lease duration (seconds) when the caller omits it.
const DEFAULT_AGENT_TASK_LEASE_SECONDS: i64 = 900;

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ListEventsQuery {
    pub project_id: Option<Uuid>,
    pub correlation_id: Option<Uuid>,
    pub asset_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct ListActionsQuery {
    pub project_id: Option<Uuid>,
    pub status: Option<String>,
    pub target_asset_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct ListAgentTasksQuery {
    pub project_id: Option<Uuid>,
    pub status: Option<String>,
    pub capability: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateEventRequest {
    pub event_type: String,
    pub source_asset_id: Uuid,
    pub source_asset_type_id: Uuid,
    pub project_id: Option<Uuid>,
    pub correlation_id: Option<Uuid>,
    pub payload: serde_json::Value,
    pub cascade_depth: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct ClaimAgentTaskRequest {
    pub agent_id: String,
    pub lease_seconds: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct SubmitAgentTaskResultRequest {
    pub result_payload: serde_json::Value,
    #[serde(default)]
    pub produced_asset_ids: Vec<Uuid>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkflowEventDto {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub project_id: Option<Uuid>,
    pub correlation_id: Uuid,
    pub event_type: String,
    pub source_asset_id: Uuid,
    pub source_asset_type_id: Uuid,
    pub payload: serde_json::Value,
    pub cascade_depth: i32,
    pub triggering_action_id: Option<Uuid>,
    pub idempotency_key: String,
    pub created_at: String,
}

impl From<adam_domain::workflow::event::WorkflowEvent> for WorkflowEventDto {
    fn from(e: adam_domain::workflow::event::WorkflowEvent) -> Self {
        Self {
            id: e.id.0,
            organization_id: e.organization_id.0,
            project_id: e.project_id.map(|p| p.0),
            correlation_id: e.correlation_id.0,
            event_type: e.event_type.to_string(),
            source_asset_id: e.source_asset_id.0,
            source_asset_type_id: e.source_asset_type_id.0,
            payload: e.payload,
            cascade_depth: e.cascade_depth,
            triggering_action_id: e.triggering_action_id,
            idempotency_key: e.idempotency_key,
            created_at: e.created_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct WorkflowInstanceDto {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub project_id: Option<Uuid>,
    pub correlation_id: Uuid,
    pub template: String,
    pub status: String,
    pub cascade_depth: i32,
    pub lock_version: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl From<adam_domain::workflow::instance::WorkflowInstance> for WorkflowInstanceDto {
    fn from(i: adam_domain::workflow::instance::WorkflowInstance) -> Self {
        Self {
            id: i.id.0,
            organization_id: i.organization_id.0,
            project_id: i.project_id.map(|p| p.0),
            correlation_id: i.correlation_id.0,
            template: i.template.to_string(),
            status: i.status.to_string(),
            cascade_depth: i.cascade_depth,
            lock_version: i.lock_version,
            created_at: i.created_at.to_rfc3339(),
            updated_at: i.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct WorkflowActionDto {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub instance_id: Uuid,
    pub action_type: String,
    pub target_asset_id: Option<Uuid>,
    pub target_asset_type_id: Option<Uuid>,
    pub status: String,
    pub lock_version: i64,
    pub idempotency_key: String,
    pub automation_level: String,
    pub is_required: bool,
    pub order_index: i32,
    pub retry_count: i32,
    pub max_retries: i32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AgentTaskDto {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub project_id: Option<Uuid>,
    pub action_id: Uuid,
    pub capability: String,
    pub status: String,
    pub agent_id: Option<String>,
    pub claimed_at: Option<String>,
    pub expires_at: Option<String>,
    pub result_payload: Option<serde_json::Value>,
    pub produced_asset_ids: Vec<Uuid>,
    pub lock_version: i64,
    pub idempotency_key: String,
    pub created_at: String,
    pub updated_at: String,
}

impl From<AgentTask> for AgentTaskDto {
    fn from(t: AgentTask) -> Self {
        Self {
            id: t.id.0,
            organization_id: t.organization_id.0,
            project_id: t.project_id.map(|p| p.0),
            action_id: t.action_id.0,
            capability: t.capability.0,
            status: t.status.to_string(),
            agent_id: t.agent_id,
            claimed_at: t.claimed_at.map(|d| d.to_rfc3339()),
            expires_at: t.expires_at.map(|d| d.to_rfc3339()),
            result_payload: t.result_payload,
            produced_asset_ids: t.produced_asset_ids,
            lock_version: t.lock_version,
            idempotency_key: t.idempotency_key,
            created_at: t.created_at.to_rfc3339(),
            updated_at: t.updated_at.to_rfc3339(),
        }
    }
}

impl From<adam_domain::workflow::action::WorkflowAction> for WorkflowActionDto {
    fn from(a: adam_domain::workflow::action::WorkflowAction) -> Self {
        Self {
            id: a.id.0,
            organization_id: a.organization_id.0,
            instance_id: a.instance_id.0,
            action_type: a.action_type.to_string(),
            target_asset_id: a.target_asset_id.map(|x| x.0),
            target_asset_type_id: a.target_asset_type_id.map(|x| x.0),
            status: a.status.to_string(),
            lock_version: a.lock_version,
            idempotency_key: a.idempotency_key,
            automation_level: a.automation_level.as_str().to_string(),
            is_required: a.is_required,
            order_index: a.order_index,
            retry_count: a.retry_count,
            max_retries: a.max_retries,
            created_at: a.created_at.to_rfc3339(),
            updated_at: a.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreatedActionsDto {
    pub event: WorkflowEventDto,
    pub created_action_ids: Vec<Uuid>,
    pub cascade_exceeded: Vec<CascadeViolationDto>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CascadeViolationDto {
    pub rule_id: Uuid,
    pub depth: i32,
    pub max: i32,
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn parse_event_type(s: &str) -> Result<EventType, ApiError> {
    match s {
        "asset_published" => Ok(EventType::AssetPublished),
        "dirty_resolved" => Ok(EventType::DirtyResolved),
        "pipeline_failed" => Ok(EventType::PipelineFailed),
        "action_succeeded" => Ok(EventType::ActionSucceeded),
        "action_failed" => Ok(EventType::ActionFailed),
        "approval_granted" => Ok(EventType::ApprovalGranted),
        "approval_rejected" => Ok(EventType::ApprovalRejected),
        other => Err(ApiError::BadRequest(format!("unknown event_type: {other}"))),
    }
}

fn parse_agent_task_status(s: &str) -> Result<AgentTaskStatus, ApiError> {
    match s {
        "queued" => Ok(AgentTaskStatus::Queued),
        "claimed" => Ok(AgentTaskStatus::Claimed),
        "running" => Ok(AgentTaskStatus::Running),
        "succeeded" => Ok(AgentTaskStatus::Succeeded),
        "failed" => Ok(AgentTaskStatus::Failed),
        "cancelled" => Ok(AgentTaskStatus::Cancelled),
        "expired" => Ok(AgentTaskStatus::Expired),
        other => Err(ApiError::BadRequest(format!(
            "unknown agent task status: {other}"
        ))),
    }
}

fn idempotency_key_header(headers: &HeaderMap) -> Result<String, ApiError> {
    headers
        .get("idempotency-key")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
        .ok_or_else(|| ApiError::BadRequest("missing Idempotency-Key header".to_string()))
}

fn map_agent_task_service_error(err: AgentTaskServiceError) -> ApiError {
    match err {
        AgentTaskServiceError::Repository(r) => ApiError::Repository(r),
        AgentTaskServiceError::Workflow(w) => ApiError::Conflict(w.to_string()),
        AgentTaskServiceError::ActionService(a) => ApiError::Conflict(a.to_string()),
        AgentTaskServiceError::TaskNotFound(_) | AgentTaskServiceError::ActionNotFound(_) => {
            ApiError::NotFound
        }
        AgentTaskServiceError::ActionNotReady(_) | AgentTaskServiceError::TaskNotClaimed(_) => {
            ApiError::Conflict(err.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// handlers
// ---------------------------------------------------------------------------

pub async fn list_events(
    State(state): State<AppState>,
    auth: ExtractAuth,
    Query(q): Query<ListEventsQuery>,
) -> Result<Json<Vec<WorkflowEventDto>>, ApiError> {
    let org_id = auth.0.principal.organization_id;
    let repo = state.workflow_event_repo.clone();
    let events = if let Some(cid) = q.correlation_id {
        repo.find_by_correlation_id(&CorrelationId::from_uuid(cid))
            .await?
    } else if let Some(aid) = q.asset_id {
        repo.find_by_asset(&AssetId::from_uuid(aid)).await?
    } else {
        // Without a filter, return an empty list rather than scanning the whole
        // event log; callers should scope by correlation_id or asset_id.
        Vec::new()
    };
    // Enforce organization boundary: drop any event not owned by the caller's
    // org. Cross-org UUID guessing must not leak another org's workflow state.
    Ok(Json(
        events
            .into_iter()
            .filter(|e| e.organization_id == org_id)
            .map(WorkflowEventDto::from)
            .collect(),
    ))
}

pub async fn get_instance(
    State(state): State<AppState>,
    auth: ExtractAuth,
    Path(instance_id): Path<Uuid>,
) -> Result<Json<WorkflowInstanceDto>, ApiError> {
    let org_id = auth.0.principal.organization_id;
    let repo = state.workflow_instance_repo.clone();
    let instance = repo
        .find_by_id(&WorkflowInstanceId::from_uuid(instance_id))
        .await?
        .ok_or(ApiError::NotFound)?;
    // Enforce organization boundary.
    if instance.organization_id != org_id {
        return Err(ApiError::NotFound);
    }
    Ok(Json(instance.into()))
}

pub async fn list_actions(
    State(state): State<AppState>,
    auth: ExtractAuth,
    Query(q): Query<ListActionsQuery>,
) -> Result<Json<Vec<WorkflowActionDto>>, ApiError> {
    let org_id = auth.0.principal.organization_id;
    let repo = state.workflow_action_repo.clone();
    // The repo only exposes an active-by-target lookup, so a target_asset_id
    // is required to scope the scan (a bare project-wide enumeration is not
    // supported by the Slice 1 repository contract).
    let Some(target) = q.target_asset_id else {
        return Ok(Json(Vec::new()));
    };
    let mut actions = repo
        .find_active_by_target(&AssetId::from_uuid(target))
        .await?;

    // Enforce organization boundary and optional status filter. project_id
    // cannot be applied precisely without an instance join (an action's project
    // lives on its parent instance); Slice 2 adds instance-scoped lookups.
    actions.retain(|a| a.organization_id == org_id);
    if let Some(status) = q.status {
        actions.retain(|a| a.status.to_string() == status);
    }
    Ok(Json(
        actions.into_iter().map(WorkflowActionDto::from).collect(),
    ))
}

pub async fn post_event(
    State(state): State<AppState>,
    auth: ExtractAuth,
    headers: HeaderMap,
    Json(req): Json<CreateEventRequest>,
) -> Result<(StatusCode, Json<CreatedActionsDto>), ApiError> {
    let request_token = idempotency_key_header(&headers)?;
    let event_type = parse_event_type(&req.event_type)?;
    let org_id = auth.0.principal.organization_id;

    let event_svc = WorkflowEventService::new(state.workflow_event_repo.clone());
    let event = event_svc
        .append_event(&AppendEventRequest {
            organization_id: org_id,
            event_type,
            source_asset_id: AssetId::from_uuid(req.source_asset_id),
            source_asset_type_id: AssetTypeId::from_uuid(req.source_asset_type_id),
            project_id: req.project_id.map(ProjectId::from_uuid),
            correlation_id: req
                .correlation_id
                .map(CorrelationId::from_uuid)
                .unwrap_or_default(),
            payload: req.payload,
            cascade_depth: req.cascade_depth.unwrap_or(0),
            triggering_action_id: None,
            request_token,
        })
        .await
        .map_err(|e| match e {
            WorkflowEventServiceError::Repository(r) => ApiError::Repository(r),
            WorkflowEventServiceError::ExistingNotFound(k) => {
                ApiError::Conflict(format!("duplicate idempotency key unresolved: {k}"))
            }
        })?;

    // Evaluate rules against the appended event and create actions.
    let evaluator = PromotionRuleEvaluator::new(
        state.workflow_rule_repo.clone(),
        state.workflow_instance_repo.clone(),
        state.workflow_action_repo.clone(),
    );
    let outcome = evaluator.evaluate(&event).await.map_err(|e| match e {
        RuleEvaluatorError::Repository(r) => ApiError::Repository(r),
        RuleEvaluatorError::Workflow(w) => ApiError::Conflict(w.to_string()),
    })?;

    let dto = CreatedActionsDto {
        event: event.clone().into(),
        created_action_ids: outcome.created.iter().map(|c| c.action_id).collect(),
        cascade_exceeded: outcome
            .cascade_exceeded
            .into_iter()
            .map(|v| CascadeViolationDto {
                rule_id: v.rule_id,
                depth: v.depth,
                max: v.max,
            })
            .collect(),
    };
    Ok((StatusCode::CREATED, Json(dto)))
}

pub async fn list_agent_tasks(
    State(state): State<AppState>,
    auth: ExtractAuth,
    Query(q): Query<ListAgentTasksQuery>,
) -> Result<Json<Vec<AgentTaskDto>>, ApiError> {
    let org_id = auth.0.principal.organization_id;
    let project_id = q.project_id.map(ProjectId::from_uuid);

    let mut tasks = if let Some(status) = q.status.as_deref() {
        state
            .agent_task_repo
            .find_by_status(&org_id, parse_agent_task_status(status)?)
            .await?
    } else if let Some(capability) = q.capability.as_deref() {
        state
            .agent_task_repo
            .list_queued(&org_id, &Capability::new(capability), project_id.as_ref())
            .await?
    } else {
        state
            .agent_task_repo
            .find_by_status(&org_id, AgentTaskStatus::Queued)
            .await?
    };

    if let Some(project_id) = project_id {
        tasks.retain(|task| task.project_id == Some(project_id));
    }
    if let Some(capability) = q.capability {
        tasks.retain(|task| task.capability.0 == capability);
    }

    Ok(Json(tasks.into_iter().map(AgentTaskDto::from).collect()))
}

pub async fn claim_agent_task(
    State(state): State<AppState>,
    auth: ExtractAuth,
    Path(task_id): Path<Uuid>,
    Json(req): Json<ClaimAgentTaskRequest>,
) -> Result<Json<Option<AgentTaskDto>>, ApiError> {
    let task_id = AgentTaskId::from_uuid(task_id);
    let task = state
        .agent_task_repo
        .find_by_id(&task_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if task.organization_id != auth.0.principal.organization_id {
        return Err(ApiError::NotFound);
    }

    let service = AgentTaskService::new(
        state.agent_task_repo.clone(),
        state.workflow_action_repo.clone(),
        state.workflow_event_repo.clone(),
        state.workflow_instance_repo.clone(),
    );
    let claimed = service
        .claim_task(
            task_id,
            &req.agent_id,
            chrono::Duration::seconds(
                req.lease_seconds
                    .unwrap_or(DEFAULT_AGENT_TASK_LEASE_SECONDS),
            ),
        )
        .await
        .map_err(map_agent_task_service_error)?;

    Ok(Json(claimed.map(AgentTaskDto::from)))
}

pub async fn submit_agent_task_result(
    State(state): State<AppState>,
    auth: ExtractAuth,
    Path(task_id): Path<Uuid>,
    Json(req): Json<SubmitAgentTaskResultRequest>,
) -> Result<Json<AgentTaskDto>, ApiError> {
    let task_id = AgentTaskId::from_uuid(task_id);
    let task = state
        .agent_task_repo
        .find_by_id(&task_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if task.organization_id != auth.0.principal.organization_id {
        return Err(ApiError::NotFound);
    }

    let service = AgentTaskService::new(
        state.agent_task_repo.clone(),
        state.workflow_action_repo.clone(),
        state.workflow_event_repo.clone(),
        state.workflow_instance_repo.clone(),
    );
    let completed = service
        .submit_result(task_id, req.result_payload, req.produced_asset_ids)
        .await
        .map_err(map_agent_task_service_error)?;

    Ok(Json(completed.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Method, Request};
    use std::sync::Arc;
    use tower::ServiceExt;

    use adam_application::services::workflow::AgentTaskService;
    use adam_domain::workflow::action::CreateActionCommand;
    use adam_domain::workflow::in_memory::{
        InMemoryAgentTaskRepository, InMemoryPromotionRuleRepository,
        InMemoryWorkflowActionRepository, InMemoryWorkflowEventRepository,
        InMemoryWorkflowInstanceRepository,
    };
    use adam_domain::workflow::instance::{CreateInstanceCommand, WorkflowTemplate};
    use adam_domain::workflow::repository::{WorkflowActionRepository, WorkflowInstanceRepository};
    use adam_domain::workflow::rule::{ActionType, AutomationLevel};
    use adam_domain::workflow::state_machine::{ActionStatus, InstanceStatus};
    use adam_domain::workflow::{Capability, CompensationPolicy, CorrelationId};
    use adam_domain::{AssetId, AssetTypeId, OrganizationId};

    fn test_state() -> AppState {
        AppState {
            asset_repo: Arc::new(adam_domain::InMemoryAssetRepository::new()),
            asset_type_repo: Arc::new(adam_domain::InMemoryAssetTypeRepository::new()),
            dependency_repo: Arc::new(adam_domain::InMemoryDependencyRepository::new()),
            dependency_rule_repo: Arc::new(adam_domain::InMemoryDependencyRuleRepository::new()),
            dirty_repo: Arc::new(adam_domain::InMemoryDirtyQueueRepository::new()),
            version_repo: Arc::new(adam_domain::InMemoryAssetVersionRepository::new()),
            dirty_log_repo: Arc::new(adam_domain::InMemoryDirtyResolutionLogRepository::new()),
            workflow_event_repo: Arc::new(InMemoryWorkflowEventRepository::default()),
            workflow_rule_repo: Arc::new(InMemoryPromotionRuleRepository::default()),
            workflow_instance_repo: Arc::new(InMemoryWorkflowInstanceRepository::default()),
            workflow_action_repo: Arc::new(InMemoryWorkflowActionRepository::default()),
            agent_task_repo: Arc::new(InMemoryAgentTaskRepository::default()),
        }
    }

    fn auth_header(org_id: uuid::Uuid) -> String {
        format!("Bearer {org_id}:agent-1:AiAgent:")
    }

    async fn seed_ready_task(state: &AppState, org_id: OrganizationId) -> AgentTaskId {
        let instance = state
            .workflow_instance_repo
            .create(&CreateInstanceCommand {
                organization_id: org_id,
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
                organization_id: org_id,
                instance_id: instance.id,
                action_type: ActionType::UpsertWorkItem,
                target_asset_id: Some(AssetId::from_uuid(uuid::Uuid::new_v4())),
                target_asset_type_id: Some(AssetTypeId::from_uuid(uuid::Uuid::new_v4())),
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
    async fn agent_task_rest_claim_and_result_complete_task() {
        let state = test_state();
        let org_id = OrganizationId::new();
        let task_id = seed_ready_task(&state, org_id).await;
        let app = super::super::create_router(state);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/agent-tasks?status=queued&capability=create_virtual_asset_context")
                    .header("authorization", auth_header(org_id.0))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let tasks: Vec<AgentTaskDto> = serde_json::from_slice(&body).unwrap();
        assert_eq!(tasks.len(), 1);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(format!("/api/agent-tasks/{}/claim", task_id.0))
                    .header("authorization", auth_header(org_id.0))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"agent_id": "agent-1", "lease_seconds": 60}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(format!("/api/agent-tasks/{}/result", task_id.0))
                    .header("authorization", auth_header(org_id.0))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "result_payload": {"ok": true},
                            "produced_asset_ids": [uuid::Uuid::new_v4()]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let completed: AgentTaskDto = serde_json::from_slice(&body).unwrap();
        assert_eq!(completed.status, "succeeded");
        assert_eq!(completed.agent_id.as_deref(), Some("agent-1"));
    }
}
