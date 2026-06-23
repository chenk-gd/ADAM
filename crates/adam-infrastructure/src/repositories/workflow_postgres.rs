//! PostgreSQL implementations of the workflow automation repository traits.
//!
//! Mirrors the in-memory reference implementation. Write operations use
//! `lock_version` CAS and return [`RepositoryError::ConcurrentModification`] on
//! mismatch; the unique idempotency-key indexes are the final idempotency guard
//! (a 23505 unique violation is mapped to
//! [`RepositoryError::DuplicateIdempotencyKey`]).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use adam_domain::asset::instance::{AssetId, AssetTypeId, OrganizationId, ProjectId};
use adam_domain::repository::RepositoryError;
use adam_domain::workflow::action::{
    BlockedReason, CompensationPolicy, CreateActionCommand, WorkflowAction, WorkflowActionId,
};
use adam_domain::workflow::agent_task::{
    AgentTask, AgentTaskId, Capability, CreateAgentTaskCommand,
};
use adam_domain::workflow::event::{CorrelationId, EventType, WorkflowEvent, WorkflowEventId};
use adam_domain::workflow::instance::{
    CreateInstanceCommand, WorkflowInstance, WorkflowInstanceId, WorkflowTemplate,
};
use adam_domain::workflow::repository::{
    AgentTaskRepository, PromotionRuleRepository, WorkflowActionRepository,
    WorkflowEventRepository, WorkflowInstanceRepository,
};
use adam_domain::workflow::rule::ActionType as DomainActionType;
use adam_domain::workflow::rule::{
    ActionTemplate, AutomationLevel, MutexGroup, PromotionRule, PromotionRuleId, RuleScope,
};
use adam_domain::workflow::state_machine::{ActionStatus, AgentTaskStatus, InstanceStatus};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Map a sqlx error to a repository error. A Postgres unique violation (SQLSTATE
/// 23505) becomes [`RepositoryError::DuplicateIdempotencyKey`].
fn map_db_err(err: sqlx::Error) -> RepositoryError {
    if let Some(db) = err.as_database_error() {
        if db.code().as_deref() == Some("23505") {
            return RepositoryError::DuplicateIdempotencyKey(
                db.constraint().unwrap_or("unique").to_string(),
            );
        }
    }
    RepositoryError::DatabaseError(err.to_string())
}

/// Parse a stored status string into an [`InstanceStatus`].
fn parse_instance_status(s: &str) -> Result<InstanceStatus, RepositoryError> {
    match s {
        "pending" => Ok(InstanceStatus::Pending),
        "ready" => Ok(InstanceStatus::Ready),
        "in_progress" => Ok(InstanceStatus::InProgress),
        "blocked" => Ok(InstanceStatus::Blocked),
        "waiting_review" => Ok(InstanceStatus::WaitingReview),
        "waiting_validation" => Ok(InstanceStatus::WaitingValidation),
        "completed" => Ok(InstanceStatus::Completed),
        "failed" => Ok(InstanceStatus::Failed),
        "cancelled" => Ok(InstanceStatus::Cancelled),
        other => Err(RepositoryError::DatabaseError(format!(
            "unknown instance status: {other}"
        ))),
    }
}

fn parse_action_status(s: &str) -> Result<ActionStatus, RepositoryError> {
    match s {
        "pending" => Ok(ActionStatus::Pending),
        "ready" => Ok(ActionStatus::Ready),
        "in_progress" => Ok(ActionStatus::InProgress),
        "waiting_approval" => Ok(ActionStatus::WaitingApproval),
        "blocked" => Ok(ActionStatus::Blocked),
        "succeeded" => Ok(ActionStatus::Succeeded),
        "failed" => Ok(ActionStatus::Failed),
        "cancelled" => Ok(ActionStatus::Cancelled),
        "skipped" => Ok(ActionStatus::Skipped),
        other => Err(RepositoryError::DatabaseError(format!(
            "unknown action status: {other}"
        ))),
    }
}

fn parse_agent_task_status(s: &str) -> Result<AgentTaskStatus, RepositoryError> {
    match s {
        "queued" => Ok(AgentTaskStatus::Queued),
        "claimed" => Ok(AgentTaskStatus::Claimed),
        "running" => Ok(AgentTaskStatus::Running),
        "succeeded" => Ok(AgentTaskStatus::Succeeded),
        "failed" => Ok(AgentTaskStatus::Failed),
        "cancelled" => Ok(AgentTaskStatus::Cancelled),
        "expired" => Ok(AgentTaskStatus::Expired),
        other => Err(RepositoryError::DatabaseError(format!(
            "unknown agent task status: {other}"
        ))),
    }
}

fn parse_blocked_reason(s: &str) -> Option<BlockedReason> {
    Some(match s {
        "missing_dependency" => BlockedReason::MissingDependency,
        "dirty_dependency" => BlockedReason::DirtyDependency,
        "waiting_approval" => BlockedReason::WaitingApproval,
        "pipeline_failed" => BlockedReason::PipelineFailed,
        "policy_denied" => BlockedReason::PolicyDenied,
        "executor_unavailable" => BlockedReason::ExecutorUnavailable,
        "external_system_unavailable" => BlockedReason::ExternalSystemUnavailable,
        "waiting_manual_intervention" => BlockedReason::WaitingManualIntervention,
        _ => return None,
    })
}

fn parse_compensation_policy(s: &str) -> Result<CompensationPolicy, RepositoryError> {
    match s {
        "none" => Ok(CompensationPolicy::None),
        "best_effort" => Ok(CompensationPolicy::BestEffort),
        "required_before_fail" => Ok(CompensationPolicy::RequiredBeforeFail),
        "manual_only" => Ok(CompensationPolicy::ManualOnly),
        other => Err(RepositoryError::DatabaseError(format!(
            "unknown compensation policy: {other}"
        ))),
    }
}

fn parse_automation_level(s: &str) -> Result<AutomationLevel, RepositoryError> {
    match s {
        "automatic" => Ok(AutomationLevel::Automatic),
        "agent_suggested" => Ok(AutomationLevel::AgentSuggested),
        "human_approval_required" => Ok(AutomationLevel::HumanApprovalRequired),
        "human_only" => Ok(AutomationLevel::HumanOnly),
        other => Err(RepositoryError::DatabaseError(format!(
            "unknown automation level: {other}"
        ))),
    }
}

fn parse_rule_scope(s: &str) -> Result<RuleScope, RepositoryError> {
    match s {
        "asset_type" => Ok(RuleScope::AssetType),
        "project" => Ok(RuleScope::Project),
        "organization" => Ok(RuleScope::Organization),
        other => Err(RepositoryError::DatabaseError(format!(
            "unknown rule scope: {other}"
        ))),
    }
}

fn parse_action_type(s: &str) -> Result<DomainActionType, RepositoryError> {
    use std::str::FromStr;
    DomainActionType::from_str(s)
        .map_err(|e| RepositoryError::DatabaseError(format!("unknown action type: {e}")))
}

fn parse_event_type(s: &str) -> Result<EventType, RepositoryError> {
    match s {
        "asset_published" => Ok(EventType::AssetPublished),
        "dirty_resolved" => Ok(EventType::DirtyResolved),
        "pipeline_failed" => Ok(EventType::PipelineFailed),
        "action_succeeded" => Ok(EventType::ActionSucceeded),
        "action_failed" => Ok(EventType::ActionFailed),
        "approval_granted" => Ok(EventType::ApprovalGranted),
        "approval_rejected" => Ok(EventType::ApprovalRejected),
        other => Err(RepositoryError::DatabaseError(format!(
            "unknown event type: {other}"
        ))),
    }
}

fn parse_template(s: &str) -> Result<WorkflowTemplate, RepositoryError> {
    match s {
        "feature" => Ok(WorkflowTemplate::Feature),
        "bugfix" => Ok(WorkflowTemplate::Bugfix),
        "test_execution" => Ok(WorkflowTemplate::TestExecution),
        other => Err(RepositoryError::DatabaseError(format!(
            "unknown workflow template: {other}"
        ))),
    }
}

/// Reconstruct an [`ActionTemplate`] from the stored `action_type` column and
/// the JSONB `action_template` payload. The payload carries
/// `payload`/`is_required`/`order_index`; `action_type` is the source of truth.
fn action_template_from_row(
    action_type: DomainActionType,
    template_json: serde_json::Value,
) -> ActionTemplate {
    let payload = template_json
        .get("payload")
        .cloned()
        .unwrap_or_else(|| template_json.clone());
    let is_required = template_json
        .get("is_required")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let order_index = template_json
        .get("order_index")
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as i32;
    ActionTemplate {
        action_type,
        payload,
        is_required,
        order_index,
    }
}

// ---------------------------------------------------------------------------
// WorkflowEventRepository
// ---------------------------------------------------------------------------

pub struct PostgresWorkflowEventRepository {
    pool: PgPool,
}

impl PostgresWorkflowEventRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl WorkflowEventRepository for PostgresWorkflowEventRepository {
    async fn append(&self, event: &WorkflowEvent) -> Result<WorkflowEvent, RepositoryError> {
        let row = sqlx::query(
            r#"
            INSERT INTO workflow_events
                (id, organization_id, project_id, correlation_id, event_type,
                 source_asset_id, source_asset_type_id, payload, idempotency_key,
                 cascade_depth, triggering_action_id, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            RETURNING id, organization_id, project_id, correlation_id, event_type,
                      source_asset_id, source_asset_type_id, payload, idempotency_key,
                      cascade_depth, triggering_action_id, created_at
            "#,
        )
        .bind(event.id.0)
        .bind(event.organization_id.0)
        .bind(event.project_id.map(|p| p.0))
        .bind(event.correlation_id.0)
        .bind(event.event_type.as_str())
        .bind(event.source_asset_id.0)
        .bind(event.source_asset_type_id.0)
        .bind(&event.payload)
        .bind(&event.idempotency_key)
        .bind(event.cascade_depth)
        .bind(event.triggering_action_id)
        .bind(event.created_at)
        .fetch_one(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(event_from_row(&row))
    }

    async fn find_by_id(
        &self,
        id: &WorkflowEventId,
    ) -> Result<Option<WorkflowEvent>, RepositoryError> {
        let row = sqlx::query(
            r#"SELECT id, organization_id, project_id, correlation_id, event_type,
                      source_asset_id, source_asset_type_id, payload, idempotency_key,
                      cascade_depth, triggering_action_id, created_at
               FROM workflow_events WHERE id = $1"#,
        )
        .bind(id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(row.as_ref().map(event_from_row))
    }

    async fn find_by_idempotency_key(
        &self,
        organization_id: &OrganizationId,
        idempotency_key: &str,
    ) -> Result<Option<WorkflowEvent>, RepositoryError> {
        let row = sqlx::query(
            r#"SELECT id, organization_id, project_id, correlation_id, event_type,
                      source_asset_id, source_asset_type_id, payload, idempotency_key,
                      cascade_depth, triggering_action_id, created_at
               FROM workflow_events
               WHERE organization_id = $1 AND idempotency_key = $2"#,
        )
        .bind(organization_id.0)
        .bind(idempotency_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(row.as_ref().map(event_from_row))
    }

    async fn find_by_correlation_id(
        &self,
        correlation_id: &CorrelationId,
    ) -> Result<Vec<WorkflowEvent>, RepositoryError> {
        let rows = sqlx::query(
            r#"SELECT id, organization_id, project_id, correlation_id, event_type,
                      source_asset_id, source_asset_type_id, payload, idempotency_key,
                      cascade_depth, triggering_action_id, created_at
               FROM workflow_events WHERE correlation_id = $1 ORDER BY created_at"#,
        )
        .bind(correlation_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(rows.iter().map(event_from_row).collect())
    }

    async fn find_by_asset(
        &self,
        asset_id: &AssetId,
    ) -> Result<Vec<WorkflowEvent>, RepositoryError> {
        let rows = sqlx::query(
            r#"SELECT id, organization_id, project_id, correlation_id, event_type,
                      source_asset_id, source_asset_type_id, payload, idempotency_key,
                      cascade_depth, triggering_action_id, created_at
               FROM workflow_events WHERE source_asset_id = $1 ORDER BY created_at"#,
        )
        .bind(asset_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(rows.iter().map(event_from_row).collect())
    }
}

fn event_from_row(row: &sqlx::postgres::PgRow) -> WorkflowEvent {
    WorkflowEvent {
        id: WorkflowEventId::from_uuid(row.get("id")),
        organization_id: OrganizationId::from_uuid(row.get("organization_id")),
        project_id: row
            .get::<Option<Uuid>, _>("project_id")
            .map(ProjectId::from_uuid),
        correlation_id: CorrelationId::from_uuid(row.get("correlation_id")),
        event_type: parse_event_type(row.get("event_type")).expect("stored event_type valid"),
        source_asset_id: AssetId::from_uuid(row.get("source_asset_id")),
        source_asset_type_id: AssetTypeId::from_uuid(row.get("source_asset_type_id")),
        payload: row.get("payload"),
        cascade_depth: row.get("cascade_depth"),
        triggering_action_id: row.get("triggering_action_id"),
        idempotency_key: row.get("idempotency_key"),
        created_at: row.get("created_at"),
    }
}

// ---------------------------------------------------------------------------
// PromotionRuleRepository
// ---------------------------------------------------------------------------

pub struct PostgresPromotionRuleRepository {
    pool: PgPool,
}

impl PostgresPromotionRuleRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PromotionRuleRepository for PostgresPromotionRuleRepository {
    async fn create(&self, rule: &PromotionRule) -> Result<PromotionRule, RepositoryError> {
        let action_type_str = rule.action_template.action_type.as_str();
        let template_json = serde_json::json!({
            "payload": rule.action_template.payload,
            "is_required": rule.action_template.is_required,
            "order_index": rule.action_template.order_index,
        });
        let row = sqlx::query(
            r#"
            INSERT INTO promotion_rules
                (id, organization_id, scope, scope_ref, event_type, source_asset_type_id,
                 mutex_group, rule_version, priority, automation_level, filters, preconditions,
                 action_type, action_template, max_cascade_depth, effective_from, effective_to,
                 rollout_segment, enabled, dry_run, audit_only, created_at, updated_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23)
            RETURNING id, organization_id, scope, scope_ref, event_type, source_asset_type_id,
                      mutex_group, rule_version, priority, automation_level, filters, preconditions,
                      action_type, action_template, max_cascade_depth, effective_from, effective_to,
                      rollout_segment, enabled, dry_run, audit_only, created_at, updated_at
            "#,
        )
        .bind(rule.id.0)
        .bind(rule.organization_id.0)
        .bind(rule.scope.as_str())
        .bind(rule.scope_ref)
        .bind(rule.event_type.as_str())
        .bind(rule.source_asset_type_id.map(|t| t.0))
        .bind(rule.mutex_group.as_ref().map(|g| g.0.as_str()))
        .bind(rule.rule_version)
        .bind(rule.priority)
        .bind(rule.automation_level.as_str())
        .bind(&rule.filters)
        .bind(&rule.preconditions)
        .bind(action_type_str)
        .bind(&template_json)
        .bind(rule.max_cascade_depth)
        .bind(rule.effective_from)
        .bind(rule.effective_to)
        .bind(rule.rollout_segment)
        .bind(rule.enabled)
        .bind(rule.dry_run)
        .bind(rule.audit_only)
        .bind(rule.created_at)
        .bind(rule.updated_at)
        .fetch_one(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(rule_from_row(&row))
    }

    async fn find_by_id(&self, id: &Uuid) -> Result<Option<PromotionRule>, RepositoryError> {
        let row = sqlx::query(
            r#"SELECT id, organization_id, scope, scope_ref, event_type, source_asset_type_id,
                      mutex_group, rule_version, priority, automation_level, filters, preconditions,
                      action_type, action_template, max_cascade_depth, effective_from, effective_to,
                      rollout_segment, enabled, dry_run, audit_only, created_at, updated_at
               FROM promotion_rules WHERE id = $1"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(row.as_ref().map(rule_from_row))
    }

    async fn find_enabled_for(
        &self,
        organization_id: &OrganizationId,
        event_type: EventType,
        _scope: Option<RuleScope>,
        now: DateTime<Utc>,
    ) -> Result<Vec<PromotionRule>, RepositoryError> {
        let rows = sqlx::query(
            r#"SELECT id, organization_id, scope, scope_ref, event_type, source_asset_type_id,
                      mutex_group, rule_version, priority, automation_level, filters, preconditions,
                      action_type, action_template, max_cascade_depth, effective_from, effective_to,
                      rollout_segment, enabled, dry_run, audit_only, created_at, updated_at
               FROM promotion_rules
               WHERE organization_id = $1 AND event_type = $2 AND enabled = TRUE
                 AND (effective_from IS NULL OR effective_from <= $3)
                 AND (effective_to IS NULL OR effective_to >= $3)
               ORDER BY scope, rule_version DESC, priority DESC, created_at"#,
        )
        .bind(organization_id.0)
        .bind(event_type.as_str())
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(rows.iter().map(rule_from_row).collect())
    }
}

fn rule_from_row(row: &sqlx::postgres::PgRow) -> PromotionRule {
    let action_type = parse_action_type(row.get("action_type")).expect("stored action_type valid");
    let template_json: serde_json::Value = row.get("action_template");
    PromotionRule {
        id: PromotionRuleId::from_uuid(row.get("id")),
        organization_id: OrganizationId::from_uuid(row.get("organization_id")),
        scope: parse_rule_scope(row.get("scope")).expect("stored scope valid"),
        scope_ref: row.get("scope_ref"),
        event_type: parse_event_type(row.get("event_type")).expect("stored event_type valid"),
        source_asset_type_id: row
            .get::<Option<Uuid>, _>("source_asset_type_id")
            .map(AssetTypeId::from_uuid),
        mutex_group: row
            .get::<Option<String>, _>("mutex_group")
            .map(MutexGroup::new),
        rule_version: row.get("rule_version"),
        priority: row.get("priority"),
        automation_level: parse_automation_level(row.get("automation_level"))
            .expect("stored automation_level valid"),
        filters: row.get("filters"),
        preconditions: row.get("preconditions"),
        action_template: action_template_from_row(action_type, template_json),
        max_cascade_depth: row.get("max_cascade_depth"),
        effective_from: row.get("effective_from"),
        effective_to: row.get("effective_to"),
        rollout_segment: row.get("rollout_segment"),
        enabled: row.get("enabled"),
        dry_run: row.get("dry_run"),
        audit_only: row.get("audit_only"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

// ---------------------------------------------------------------------------
// WorkflowInstanceRepository
// ---------------------------------------------------------------------------

pub struct PostgresWorkflowInstanceRepository {
    pool: PgPool,
}

impl PostgresWorkflowInstanceRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl WorkflowInstanceRepository for PostgresWorkflowInstanceRepository {
    async fn create(
        &self,
        cmd: &CreateInstanceCommand,
    ) -> Result<WorkflowInstance, RepositoryError> {
        let row = sqlx::query(
            r#"
            INSERT INTO workflow_instances
                (organization_id, project_id, correlation_id, template, status, cascade_depth)
            VALUES ($1, $2, $3, $4, 'pending', $5)
            RETURNING id, organization_id, project_id, correlation_id, template, status,
                      cascade_depth, lock_version, created_at, updated_at
            "#,
        )
        .bind(cmd.organization_id.0)
        .bind(cmd.project_id.map(|p| p.0))
        .bind(cmd.correlation_id.0)
        .bind(cmd.template.as_str())
        .bind(cmd.cascade_depth)
        .fetch_one(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(instance_from_row(&row))
    }

    async fn find_by_id(
        &self,
        id: &WorkflowInstanceId,
    ) -> Result<Option<WorkflowInstance>, RepositoryError> {
        let row = sqlx::query(
            r#"SELECT id, organization_id, project_id, correlation_id, template, status,
                      cascade_depth, lock_version, created_at, updated_at
               FROM workflow_instances WHERE id = $1"#,
        )
        .bind(id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(row.as_ref().map(instance_from_row))
    }

    async fn update_cas(
        &self,
        id: &WorkflowInstanceId,
        expected_lock_version: i64,
        new_status: InstanceStatus,
    ) -> Result<i64, RepositoryError> {
        let row = sqlx::query(
            r#"UPDATE workflow_instances
               SET status = $1, lock_version = lock_version + 1, updated_at = NOW()
               WHERE id = $2 AND lock_version = $3
               RETURNING lock_version"#,
        )
        .bind(new_status.to_string())
        .bind(id.0)
        .bind(expected_lock_version)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_db_err)?;

        match row {
            Some(r) => Ok(r.get("lock_version")),
            None => {
                // Either not found or lock version mismatch. Distinguish by
                // checking existence.
                let exists: Option<Uuid> =
                    sqlx::query_scalar("SELECT id FROM workflow_instances WHERE id = $1")
                        .bind(id.0)
                        .fetch_optional(&self.pool)
                        .await
                        .map_err(map_db_err)?;
                if exists.is_some() {
                    Err(RepositoryError::ConcurrentModification {
                        expected: expected_lock_version,
                        actual: -1,
                    })
                } else {
                    Err(RepositoryError::NotFound(format!("workflow_instance {id}")))
                }
            }
        }
    }

    async fn find_non_terminal(&self) -> Result<Vec<WorkflowInstance>, RepositoryError> {
        let rows = sqlx::query(
            r#"SELECT id, organization_id, project_id, correlation_id, template, status,
                      cascade_depth, lock_version, created_at, updated_at
               FROM workflow_instances
               WHERE status NOT IN ('completed', 'failed', 'cancelled')
               ORDER BY created_at"#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(rows.iter().map(instance_from_row).collect())
    }
}

fn instance_from_row(row: &sqlx::postgres::PgRow) -> WorkflowInstance {
    WorkflowInstance {
        id: WorkflowInstanceId::from_uuid(row.get("id")),
        organization_id: OrganizationId::from_uuid(row.get("organization_id")),
        project_id: row
            .get::<Option<Uuid>, _>("project_id")
            .map(ProjectId::from_uuid),
        correlation_id: CorrelationId::from_uuid(row.get("correlation_id")),
        template: parse_template(row.get("template")).expect("stored template valid"),
        status: parse_instance_status(row.get("status")).expect("stored status valid"),
        cascade_depth: row.get("cascade_depth"),
        lock_version: row.get("lock_version"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

// ---------------------------------------------------------------------------
// WorkflowActionRepository
// ---------------------------------------------------------------------------

pub struct PostgresWorkflowActionRepository {
    pool: PgPool,
}

impl PostgresWorkflowActionRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl WorkflowActionRepository for PostgresWorkflowActionRepository {
    async fn create(&self, cmd: &CreateActionCommand) -> Result<WorkflowAction, RepositoryError> {
        let row = sqlx::query(
            r#"
            INSERT INTO workflow_actions
                (organization_id, instance_id, action_type, target_asset_id, target_asset_type_id,
                 status, lock_version, idempotency_key, preconditions, postconditions,
                 automation_level, is_required, order_index, compensation_action_type,
                 compensation_payload, compensation_policy, retry_count, max_retries)
            VALUES ($1,$2,$3,$4,$5,'pending',1,$6,$7,$8,$9,$10,$11,$12,$13,$14,0,$15)
            RETURNING id, organization_id, instance_id, action_type, target_asset_id,
                      target_asset_type_id, status, lock_version, idempotency_key, preconditions,
                      postconditions, automation_level, is_required, order_index,
                      compensation_action_type, compensation_payload, compensation_policy,
                      retry_count, max_retries, next_retry_at, blocked_reason, result_payload,
                      created_at, updated_at
            "#,
        )
        .bind(cmd.organization_id.0)
        .bind(cmd.instance_id.0)
        .bind(cmd.action_type.as_str())
        .bind(cmd.target_asset_id.map(|a| a.0))
        .bind(cmd.target_asset_type_id.map(|t| t.0))
        .bind(&cmd.idempotency_key)
        .bind(&cmd.preconditions)
        .bind(&cmd.postconditions)
        .bind(cmd.automation_level.as_str())
        .bind(cmd.is_required)
        .bind(cmd.order_index)
        .bind(cmd.compensation_action_type.map(|t| t.as_str()))
        .bind(&cmd.compensation_payload)
        .bind(cmd.compensation_policy.as_str())
        .bind(cmd.max_retries)
        .fetch_one(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(action_from_row(&row))
    }

    async fn find_by_id(
        &self,
        id: &WorkflowActionId,
    ) -> Result<Option<WorkflowAction>, RepositoryError> {
        let row = sqlx::query(&(action_select_sql().to_owned() + " WHERE id = $1"))
            .bind(id.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_db_err)?;

        Ok(row.as_ref().map(action_from_row))
    }

    async fn find_by_idempotency_key(
        &self,
        organization_id: &OrganizationId,
        idempotency_key: &str,
    ) -> Result<Option<WorkflowAction>, RepositoryError> {
        let row = sqlx::query(
            &(action_select_sql().to_owned()
                + " WHERE organization_id = $1 AND idempotency_key = $2"),
        )
        .bind(organization_id.0)
        .bind(idempotency_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(row.as_ref().map(action_from_row))
    }

    async fn find_by_instance(
        &self,
        instance_id: &WorkflowInstanceId,
    ) -> Result<Vec<WorkflowAction>, RepositoryError> {
        let rows = sqlx::query(
            &(action_select_sql().to_owned()
                + " WHERE instance_id = $1 ORDER BY order_index, created_at"),
        )
        .bind(instance_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(rows.iter().map(action_from_row).collect())
    }

    async fn find_active_by_target(
        &self,
        target_asset_id: &AssetId,
    ) -> Result<Vec<WorkflowAction>, RepositoryError> {
        let rows = sqlx::query(
            &(action_select_sql().to_owned()
                + " WHERE target_asset_id = $1 AND status NOT IN ('succeeded','cancelled','skipped') ORDER BY created_at"),
        )
        .bind(target_asset_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(rows.iter().map(action_from_row).collect())
    }

    async fn update_cas(
        &self,
        action: &WorkflowAction,
        expected_lock_version: i64,
    ) -> Result<WorkflowAction, RepositoryError> {
        let row = sqlx::query(
            r#"
            UPDATE workflow_actions SET
                status = $1,
                lock_version = lock_version + 1,
                preconditions = $2,
                postconditions = $3,
                automation_level = $4,
                is_required = $5,
                order_index = $6,
                compensation_action_type = $7,
                compensation_payload = $8,
                compensation_policy = $9,
                retry_count = $10,
                max_retries = $11,
                next_retry_at = $12,
                blocked_reason = $13,
                result_payload = $14,
                updated_at = NOW()
            WHERE id = $15 AND lock_version = $16
            RETURNING id, organization_id, instance_id, action_type, target_asset_id,
                      target_asset_type_id, status, lock_version, idempotency_key, preconditions,
                      postconditions, automation_level, is_required, order_index,
                      compensation_action_type, compensation_payload, compensation_policy,
                      retry_count, max_retries, next_retry_at, blocked_reason, result_payload,
                      created_at, updated_at
            "#,
        )
        .bind(action.status.to_string())
        .bind(&action.preconditions)
        .bind(&action.postconditions)
        .bind(action.automation_level.as_str())
        .bind(action.is_required)
        .bind(action.order_index)
        .bind(action.compensation_action_type.map(|t| t.as_str()))
        .bind(&action.compensation_payload)
        .bind(action.compensation_policy.as_str())
        .bind(action.retry_count)
        .bind(action.max_retries)
        .bind(action.next_retry_at)
        .bind(action.blocked_reason.map(blocked_reason_str))
        .bind(&action.result_payload)
        .bind(action.id.0)
        .bind(expected_lock_version)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_db_err)?;

        match row {
            Some(r) => Ok(action_from_row(&r)),
            None => {
                let exists: Option<Uuid> =
                    sqlx::query_scalar("SELECT id FROM workflow_actions WHERE id = $1")
                        .bind(action.id.0)
                        .fetch_optional(&self.pool)
                        .await
                        .map_err(map_db_err)?;
                if exists.is_some() {
                    Err(RepositoryError::ConcurrentModification {
                        expected: expected_lock_version,
                        actual: -1,
                    })
                } else {
                    Err(RepositoryError::NotFound(format!(
                        "workflow_action {}",
                        action.id
                    )))
                }
            }
        }
    }
}

fn action_select_sql() -> &'static str {
    "SELECT id, organization_id, instance_id, action_type, target_asset_id,
            target_asset_type_id, status, lock_version, idempotency_key, preconditions,
            postconditions, automation_level, is_required, order_index,
            compensation_action_type, compensation_payload, compensation_policy,
            retry_count, max_retries, next_retry_at, blocked_reason, result_payload,
            created_at, updated_at FROM workflow_actions"
}

fn blocked_reason_str(r: BlockedReason) -> &'static str {
    r.as_str()
}

fn action_from_row(row: &sqlx::postgres::PgRow) -> WorkflowAction {
    let blocked: Option<String> = row.get("blocked_reason");
    WorkflowAction {
        id: WorkflowActionId::from_uuid(row.get("id")),
        organization_id: OrganizationId::from_uuid(row.get("organization_id")),
        instance_id: WorkflowInstanceId::from_uuid(row.get("instance_id")),
        action_type: parse_action_type(row.get("action_type")).expect("stored action_type valid"),
        target_asset_id: row
            .get::<Option<Uuid>, _>("target_asset_id")
            .map(AssetId::from_uuid),
        target_asset_type_id: row
            .get::<Option<Uuid>, _>("target_asset_type_id")
            .map(AssetTypeId::from_uuid),
        status: parse_action_status(row.get("status")).expect("stored status valid"),
        lock_version: row.get("lock_version"),
        idempotency_key: row.get("idempotency_key"),
        preconditions: row.get("preconditions"),
        postconditions: row.get("postconditions"),
        automation_level: parse_automation_level(row.get("automation_level"))
            .expect("stored automation_level valid"),
        is_required: row.get("is_required"),
        order_index: row.get("order_index"),
        compensation_action_type: row
            .get::<Option<String>, _>("compensation_action_type")
            .and_then(|s| parse_action_type(&s).ok()),
        compensation_payload: row.get("compensation_payload"),
        compensation_policy: parse_compensation_policy(row.get("compensation_policy"))
            .expect("stored compensation_policy valid"),
        retry_count: row.get("retry_count"),
        max_retries: row.get("max_retries"),
        next_retry_at: row.get("next_retry_at"),
        blocked_reason: blocked.as_deref().and_then(parse_blocked_reason),
        result_payload: row.get("result_payload"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

// ---------------------------------------------------------------------------
// AgentTaskRepository
// ---------------------------------------------------------------------------

pub struct PostgresAgentTaskRepository {
    pool: PgPool,
}

impl PostgresAgentTaskRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AgentTaskRepository for PostgresAgentTaskRepository {
    async fn create(&self, cmd: &CreateAgentTaskCommand) -> Result<AgentTask, RepositoryError> {
        let row = sqlx::query(
            r#"
            INSERT INTO agent_tasks
                (organization_id, project_id, action_id, capability, status,
                 expires_at, produced_asset_ids, idempotency_key)
            VALUES ($1, $2, $3, $4, 'queued', $5, '[]'::jsonb, $6)
            RETURNING id, organization_id, project_id, action_id, capability, status,
                      agent_id, claimed_at, expires_at, result_payload, produced_asset_ids,
                      lock_version, idempotency_key, created_at, updated_at
            "#,
        )
        .bind(cmd.organization_id.0)
        .bind(cmd.project_id.map(|p| p.0))
        .bind(cmd.action_id.0)
        .bind(&cmd.capability.0)
        .bind(cmd.expires_at)
        .bind(&cmd.idempotency_key)
        .fetch_one(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(agent_task_from_row(&row))
    }

    async fn find_by_id(&self, id: &AgentTaskId) -> Result<Option<AgentTask>, RepositoryError> {
        let row = sqlx::query(&(agent_task_select_sql().to_owned() + " WHERE id = $1"))
            .bind(id.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_db_err)?;

        Ok(row.as_ref().map(agent_task_from_row))
    }

    async fn find_by_action(
        &self,
        action_id: &WorkflowActionId,
    ) -> Result<Vec<AgentTask>, RepositoryError> {
        let rows = sqlx::query(
            &(agent_task_select_sql().to_owned() + " WHERE action_id = $1 ORDER BY created_at"),
        )
        .bind(action_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(rows.iter().map(agent_task_from_row).collect())
    }

    async fn list_queued(
        &self,
        organization_id: &OrganizationId,
        capability: &Capability,
        project_id: Option<&ProjectId>,
    ) -> Result<Vec<AgentTask>, RepositoryError> {
        let rows = sqlx::query(
            &(agent_task_select_sql().to_owned()
                + " WHERE organization_id = $1 AND capability = $2 AND status = 'queued'
                    AND ($3::uuid IS NULL OR project_id = $3)
                    ORDER BY created_at"),
        )
        .bind(organization_id.0)
        .bind(&capability.0)
        .bind(project_id.map(|p| p.0))
        .fetch_all(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(rows.iter().map(agent_task_from_row).collect())
    }

    async fn claim_cas(
        &self,
        id: &AgentTaskId,
        agent_id: &str,
        claimed_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> Result<Option<AgentTask>, RepositoryError> {
        let row = sqlx::query(
            r#"
            UPDATE agent_tasks SET
                status = 'claimed',
                agent_id = $1,
                claimed_at = $2,
                expires_at = $3,
                lock_version = lock_version + 1,
                updated_at = NOW()
            WHERE id = $4 AND status = 'queued'
            RETURNING id, organization_id, project_id, action_id, capability, status,
                      agent_id, claimed_at, expires_at, result_payload, produced_asset_ids,
                      lock_version, idempotency_key, created_at, updated_at
            "#,
        )
        .bind(agent_id)
        .bind(claimed_at)
        .bind(expires_at)
        .bind(id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(row.as_ref().map(agent_task_from_row))
    }

    async fn update_cas(
        &self,
        task: &AgentTask,
        expected_lock_version: i64,
    ) -> Result<AgentTask, RepositoryError> {
        let produced_asset_ids = serde_json::json!(task.produced_asset_ids);
        let row = sqlx::query(
            r#"
            UPDATE agent_tasks SET
                status = $1,
                agent_id = $2,
                claimed_at = $3,
                expires_at = $4,
                result_payload = $5,
                produced_asset_ids = $6,
                lock_version = lock_version + 1,
                updated_at = NOW()
            WHERE id = $7 AND lock_version = $8
            RETURNING id, organization_id, project_id, action_id, capability, status,
                      agent_id, claimed_at, expires_at, result_payload, produced_asset_ids,
                      lock_version, idempotency_key, created_at, updated_at
            "#,
        )
        .bind(task.status.to_string())
        .bind(&task.agent_id)
        .bind(task.claimed_at)
        .bind(task.expires_at)
        .bind(&task.result_payload)
        .bind(&produced_asset_ids)
        .bind(task.id.0)
        .bind(expected_lock_version)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_db_err)?;

        match row {
            Some(row) => Ok(agent_task_from_row(&row)),
            None => Err(RepositoryError::ConcurrentModification {
                expected: expected_lock_version,
                actual: -1,
            }),
        }
    }

    async fn find_expired(&self, now: DateTime<Utc>) -> Result<Vec<AgentTask>, RepositoryError> {
        let rows = sqlx::query(
            &(agent_task_select_sql().to_owned()
                + " WHERE expires_at IS NOT NULL
                    AND expires_at < $1
                    AND status NOT IN ('succeeded', 'failed', 'cancelled', 'expired')
                    ORDER BY expires_at"),
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(rows.iter().map(agent_task_from_row).collect())
    }

    async fn find_by_status(
        &self,
        organization_id: &OrganizationId,
        status: AgentTaskStatus,
    ) -> Result<Vec<AgentTask>, RepositoryError> {
        let rows = sqlx::query(
            &(agent_task_select_sql().to_owned()
                + " WHERE organization_id = $1 AND status = $2 ORDER BY created_at"),
        )
        .bind(organization_id.0)
        .bind(status.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(map_db_err)?;

        Ok(rows.iter().map(agent_task_from_row).collect())
    }
}

fn agent_task_select_sql() -> &'static str {
    "SELECT id, organization_id, project_id, action_id, capability, status,
            agent_id, claimed_at, expires_at, result_payload, produced_asset_ids,
            lock_version, idempotency_key, created_at, updated_at FROM agent_tasks"
}

fn agent_task_from_row(row: &sqlx::postgres::PgRow) -> AgentTask {
    let produced: serde_json::Value = row.get("produced_asset_ids");
    let produced_asset_ids = produced
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .filter_map(|id| Uuid::parse_str(id).ok())
                .collect()
        })
        .unwrap_or_default();

    AgentTask {
        id: AgentTaskId::from_uuid(row.get("id")),
        organization_id: OrganizationId::from_uuid(row.get("organization_id")),
        project_id: row
            .get::<Option<Uuid>, _>("project_id")
            .map(ProjectId::from_uuid),
        action_id: WorkflowActionId::from_uuid(row.get("action_id")),
        capability: Capability(row.get("capability")),
        status: parse_agent_task_status(row.get("status")).expect("stored task status valid"),
        agent_id: row.get("agent_id"),
        claimed_at: row.get("claimed_at"),
        expires_at: row.get("expires_at"),
        result_payload: row.get("result_payload"),
        produced_asset_ids,
        lock_version: row.get("lock_version"),
        idempotency_key: row.get("idempotency_key"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}
