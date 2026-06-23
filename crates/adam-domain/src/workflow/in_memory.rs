//! In-memory implementations of the workflow repository traits.
//!
//! Used for unit tests and as a reference for the Postgres implementation.
//! All write operations enforce optimistic locking via `lock_version` CAS and
//! unique-key idempotency guards, mirroring the production repository.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::asset::instance::{AssetId, OrganizationId, ProjectId};
use crate::repository::RepositoryError;
use crate::workflow::action::{CreateActionCommand, WorkflowAction, WorkflowActionId};
use crate::workflow::agent_task::{AgentTask, AgentTaskId, Capability, CreateAgentTaskCommand};
use crate::workflow::approval_gate::{ApprovalGate, ApprovalGateId, CreateApprovalGateCommand};
use crate::workflow::dead_letter::{DeadLetter, DeadLetterId, DeadLetterStatus};
use crate::workflow::event::{CorrelationId, EventType, WorkflowEvent, WorkflowEventId};
use crate::workflow::instance::{CreateInstanceCommand, WorkflowInstance, WorkflowInstanceId};
use crate::workflow::repository::{
    AgentTaskRepository, ApprovalGateRepository, DeadLetterRepository, PromotionRuleRepository,
    WorkflowActionRepository, WorkflowEventRepository, WorkflowInstanceRepository,
};
use crate::workflow::rule::PromotionRule;
use crate::workflow::rule::RuleScope;
use crate::workflow::state_machine::{ActionStatus, AgentTaskStatus, InstanceStatus};

fn now() -> DateTime<Utc> {
    Utc::now()
}

/// In-memory workflow event repository.
#[derive(Default)]
pub struct InMemoryWorkflowEventRepository {
    events: Mutex<HashMap<WorkflowEventId, WorkflowEvent>>,
    idempotency: Mutex<HashMap<String, WorkflowEventId>>,
}

#[async_trait]
impl WorkflowEventRepository for InMemoryWorkflowEventRepository {
    async fn append(&self, event: &WorkflowEvent) -> Result<WorkflowEvent, RepositoryError> {
        let mut idem = self.idempotency.lock().expect("idempotency lock poisoned");
        if idem.contains_key(&event.idempotency_key) {
            return Err(RepositoryError::DuplicateIdempotencyKey(
                event.idempotency_key.clone(),
            ));
        }
        idem.insert(event.idempotency_key.clone(), event.id);
        drop(idem);

        self.events
            .lock()
            .expect("events lock poisoned")
            .insert(event.id, event.clone());
        Ok(event.clone())
    }

    async fn find_by_id(
        &self,
        id: &WorkflowEventId,
    ) -> Result<Option<WorkflowEvent>, RepositoryError> {
        Ok(self.events.lock().expect("events lock").get(id).cloned())
    }

    async fn find_by_idempotency_key(
        &self,
        organization_id: &OrganizationId,
        idempotency_key: &str,
    ) -> Result<Option<WorkflowEvent>, RepositoryError> {
        let event_id = self
            .idempotency
            .lock()
            .expect("idempotency lock")
            .get(idempotency_key)
            .copied();
        let Some(event_id) = event_id else {
            return Ok(None);
        };

        Ok(self
            .events
            .lock()
            .expect("events lock")
            .get(&event_id)
            .filter(|event| event.organization_id == *organization_id)
            .cloned())
    }

    async fn find_by_correlation_id(
        &self,
        correlation_id: &CorrelationId,
    ) -> Result<Vec<WorkflowEvent>, RepositoryError> {
        Ok(self
            .events
            .lock()
            .expect("events lock")
            .values()
            .filter(|e| e.correlation_id == *correlation_id)
            .cloned()
            .collect())
    }

    async fn find_by_asset(
        &self,
        asset_id: &AssetId,
    ) -> Result<Vec<WorkflowEvent>, RepositoryError> {
        Ok(self
            .events
            .lock()
            .expect("events lock")
            .values()
            .filter(|e| e.source_asset_id == *asset_id)
            .cloned()
            .collect())
    }
}

/// In-memory promotion rule repository.
#[derive(Default)]
pub struct InMemoryPromotionRuleRepository {
    rules: Mutex<HashMap<uuid::Uuid, PromotionRule>>,
}

#[async_trait]
impl PromotionRuleRepository for InMemoryPromotionRuleRepository {
    async fn create(&self, rule: &PromotionRule) -> Result<PromotionRule, RepositoryError> {
        self.rules
            .lock()
            .expect("rules lock")
            .insert(rule.id.0, rule.clone());
        Ok(rule.clone())
    }

    async fn find_by_id(&self, id: &uuid::Uuid) -> Result<Option<PromotionRule>, RepositoryError> {
        Ok(self.rules.lock().expect("rules lock").get(id).cloned())
    }

    async fn find_enabled_for(
        &self,
        organization_id: &OrganizationId,
        event_type: EventType,
        scope: Option<RuleScope>,
        now: DateTime<Utc>,
    ) -> Result<Vec<PromotionRule>, RepositoryError> {
        Ok(self
            .rules
            .lock()
            .expect("rules lock")
            .values()
            .filter(|r| {
                r.organization_id == *organization_id && r.event_type == event_type && r.enabled
            })
            .filter(|r| scope.is_none_or(|scope| r.scope == scope))
            .filter(|r| r.is_effective_at(now))
            .cloned()
            .collect())
    }
}

/// In-memory workflow instance repository.
#[derive(Default)]
pub struct InMemoryWorkflowInstanceRepository {
    instances: Mutex<HashMap<WorkflowInstanceId, WorkflowInstance>>,
}

#[async_trait]
impl WorkflowInstanceRepository for InMemoryWorkflowInstanceRepository {
    async fn create(
        &self,
        cmd: &CreateInstanceCommand,
    ) -> Result<WorkflowInstance, RepositoryError> {
        let ts = now();
        let instance = WorkflowInstance {
            id: WorkflowInstanceId::new(),
            organization_id: cmd.organization_id,
            project_id: cmd.project_id,
            correlation_id: cmd.correlation_id,
            template: cmd.template,
            status: InstanceStatus::Pending,
            cascade_depth: cmd.cascade_depth,
            lock_version: 1,
            created_at: ts,
            updated_at: ts,
        };
        self.instances
            .lock()
            .expect("instances lock")
            .insert(instance.id, instance.clone());
        Ok(instance)
    }

    async fn find_by_id(
        &self,
        id: &WorkflowInstanceId,
    ) -> Result<Option<WorkflowInstance>, RepositoryError> {
        Ok(self
            .instances
            .lock()
            .expect("instances lock")
            .get(id)
            .cloned())
    }

    async fn update_cas(
        &self,
        id: &WorkflowInstanceId,
        expected_lock_version: i64,
        new_status: InstanceStatus,
    ) -> Result<i64, RepositoryError> {
        let mut instances = self.instances.lock().expect("instances lock");
        let instance = instances
            .get_mut(id)
            .ok_or_else(|| RepositoryError::NotFound(format!("workflow_instance {id}")))?;
        if instance.lock_version != expected_lock_version {
            return Err(RepositoryError::ConcurrentModification {
                expected: expected_lock_version,
                actual: instance.lock_version,
            });
        }
        instance.lock_version += 1;
        instance.status = new_status;
        instance.updated_at = now();
        Ok(instance.lock_version)
    }

    async fn find_non_terminal(&self) -> Result<Vec<WorkflowInstance>, RepositoryError> {
        Ok(self
            .instances
            .lock()
            .expect("instances lock")
            .values()
            .filter(|i| !i.is_terminal())
            .cloned()
            .collect())
    }
}

/// In-memory workflow action repository.
#[derive(Default)]
pub struct InMemoryWorkflowActionRepository {
    actions: Mutex<HashMap<WorkflowActionId, WorkflowAction>>,
    idempotency: Mutex<HashMap<String, WorkflowActionId>>,
}

#[async_trait]
impl WorkflowActionRepository for InMemoryWorkflowActionRepository {
    async fn create(&self, cmd: &CreateActionCommand) -> Result<WorkflowAction, RepositoryError> {
        let mut idem = self.idempotency.lock().expect("idem lock");
        if idem.contains_key(&cmd.idempotency_key) {
            return Err(RepositoryError::DuplicateIdempotencyKey(
                cmd.idempotency_key.clone(),
            ));
        }
        let ts = now();
        let action = WorkflowAction {
            id: WorkflowActionId::new(),
            organization_id: cmd.organization_id,
            instance_id: cmd.instance_id,
            action_type: cmd.action_type,
            target_asset_id: cmd.target_asset_id,
            target_asset_type_id: cmd.target_asset_type_id,
            status: ActionStatus::Pending,
            lock_version: 1,
            idempotency_key: cmd.idempotency_key.clone(),
            preconditions: cmd.preconditions.clone(),
            postconditions: cmd.postconditions.clone(),
            automation_level: cmd.automation_level,
            is_required: cmd.is_required,
            order_index: cmd.order_index,
            compensation_action_type: cmd.compensation_action_type,
            compensation_payload: cmd.compensation_payload.clone(),
            compensation_policy: cmd.compensation_policy,
            retry_count: 0,
            max_retries: cmd.max_retries,
            next_retry_at: None,
            blocked_reason: None,
            result_payload: None,
            created_at: ts,
            updated_at: ts,
        };
        idem.insert(cmd.idempotency_key.clone(), action.id);
        drop(idem);
        self.actions
            .lock()
            .expect("actions lock")
            .insert(action.id, action.clone());
        Ok(action)
    }

    async fn find_by_id(
        &self,
        id: &WorkflowActionId,
    ) -> Result<Option<WorkflowAction>, RepositoryError> {
        Ok(self.actions.lock().expect("actions lock").get(id).cloned())
    }

    async fn find_by_idempotency_key(
        &self,
        organization_id: &OrganizationId,
        idempotency_key: &str,
    ) -> Result<Option<WorkflowAction>, RepositoryError> {
        let action_id = self
            .idempotency
            .lock()
            .expect("idem lock")
            .get(idempotency_key)
            .copied();
        let Some(action_id) = action_id else {
            return Ok(None);
        };

        Ok(self
            .actions
            .lock()
            .expect("actions lock")
            .get(&action_id)
            .filter(|action| action.organization_id == *organization_id)
            .cloned())
    }

    async fn find_by_instance(
        &self,
        instance_id: &WorkflowInstanceId,
    ) -> Result<Vec<WorkflowAction>, RepositoryError> {
        Ok(self
            .actions
            .lock()
            .expect("actions lock")
            .values()
            .filter(|a| a.instance_id == *instance_id)
            .cloned()
            .collect())
    }

    async fn find_active_by_target(
        &self,
        target_asset_id: &AssetId,
    ) -> Result<Vec<WorkflowAction>, RepositoryError> {
        Ok(self
            .actions
            .lock()
            .expect("actions lock")
            .values()
            .filter(|a| a.target_asset_id == Some(*target_asset_id) && !a.is_terminal())
            .cloned()
            .collect())
    }

    async fn update_cas(
        &self,
        action: &WorkflowAction,
        expected_lock_version: i64,
    ) -> Result<WorkflowAction, RepositoryError> {
        let mut actions = self.actions.lock().expect("actions lock");
        let existing = actions
            .get(&action.id)
            .ok_or_else(|| RepositoryError::NotFound(format!("workflow_action {}", action.id)))?;
        if existing.lock_version != expected_lock_version {
            return Err(RepositoryError::ConcurrentModification {
                expected: expected_lock_version,
                actual: existing.lock_version,
            });
        }
        let mut updated = action.clone();
        updated.lock_version = expected_lock_version + 1;
        updated.updated_at = now();
        actions.insert(action.id, updated.clone());
        Ok(updated)
    }
}

/// In-memory agent task repository.
#[derive(Default)]
pub struct InMemoryAgentTaskRepository {
    tasks: Mutex<HashMap<AgentTaskId, AgentTask>>,
    idempotency: Mutex<HashMap<String, AgentTaskId>>,
}

#[async_trait]
impl AgentTaskRepository for InMemoryAgentTaskRepository {
    async fn create(&self, cmd: &CreateAgentTaskCommand) -> Result<AgentTask, RepositoryError> {
        let mut idem = self.idempotency.lock().expect("idem lock");
        if idem.contains_key(&cmd.idempotency_key) {
            return Err(RepositoryError::DuplicateIdempotencyKey(
                cmd.idempotency_key.clone(),
            ));
        }
        let ts = now();
        let task = AgentTask {
            id: AgentTaskId::new(),
            organization_id: cmd.organization_id,
            project_id: cmd.project_id,
            action_id: cmd.action_id,
            capability: cmd.capability.clone(),
            status: AgentTaskStatus::Queued,
            agent_id: None,
            claimed_at: None,
            expires_at: cmd.expires_at,
            result_payload: None,
            produced_asset_ids: Vec::new(),
            lock_version: 1,
            idempotency_key: cmd.idempotency_key.clone(),
            created_at: ts,
            updated_at: ts,
        };
        idem.insert(cmd.idempotency_key.clone(), task.id);
        drop(idem);
        self.tasks
            .lock()
            .expect("tasks lock")
            .insert(task.id, task.clone());
        Ok(task)
    }

    async fn find_by_id(&self, id: &AgentTaskId) -> Result<Option<AgentTask>, RepositoryError> {
        Ok(self.tasks.lock().expect("tasks lock").get(id).cloned())
    }

    async fn find_by_action(
        &self,
        action_id: &WorkflowActionId,
    ) -> Result<Vec<AgentTask>, RepositoryError> {
        Ok(self
            .tasks
            .lock()
            .expect("tasks lock")
            .values()
            .filter(|t| t.action_id == *action_id)
            .cloned()
            .collect())
    }

    async fn list_queued(
        &self,
        organization_id: &OrganizationId,
        capability: &Capability,
        project_id: Option<&ProjectId>,
    ) -> Result<Vec<AgentTask>, RepositoryError> {
        Ok(self
            .tasks
            .lock()
            .expect("tasks lock")
            .values()
            .filter(|t| {
                t.organization_id == *organization_id
                    && t.capability == *capability
                    && t.status == AgentTaskStatus::Queued
                    && project_id.is_none_or(|p| t.project_id == Some(*p))
            })
            .cloned()
            .collect())
    }

    async fn claim_cas(
        &self,
        id: &AgentTaskId,
        agent_id: &str,
        claimed_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> Result<Option<AgentTask>, RepositoryError> {
        let mut tasks = self.tasks.lock().expect("tasks lock");
        let task = match tasks.get_mut(id) {
            Some(t) => t,
            None => return Ok(None),
        };
        if task.status != AgentTaskStatus::Queued {
            return Ok(None);
        }
        task.status = AgentTaskStatus::Claimed;
        task.agent_id = Some(agent_id.to_string());
        task.claimed_at = Some(claimed_at);
        task.expires_at = Some(expires_at);
        task.lock_version += 1;
        task.updated_at = now();
        Ok(Some(task.clone()))
    }

    async fn update_cas(
        &self,
        task: &AgentTask,
        expected_lock_version: i64,
    ) -> Result<AgentTask, RepositoryError> {
        let mut tasks = self.tasks.lock().expect("tasks lock");
        let existing = tasks
            .get(&task.id)
            .ok_or_else(|| RepositoryError::NotFound(format!("agent_task {}", task.id)))?;
        if existing.lock_version != expected_lock_version {
            return Err(RepositoryError::ConcurrentModification {
                expected: expected_lock_version,
                actual: existing.lock_version,
            });
        }
        let mut updated = task.clone();
        updated.lock_version = expected_lock_version + 1;
        updated.updated_at = now();
        tasks.insert(task.id, updated.clone());
        Ok(updated)
    }

    async fn find_expired(&self, now: DateTime<Utc>) -> Result<Vec<AgentTask>, RepositoryError> {
        Ok(self
            .tasks
            .lock()
            .expect("tasks lock")
            .values()
            .filter(|t| !t.is_terminal() && t.is_expired_at(now))
            .cloned()
            .collect())
    }

    async fn find_by_status(
        &self,
        organization_id: &OrganizationId,
        status: AgentTaskStatus,
    ) -> Result<Vec<AgentTask>, RepositoryError> {
        Ok(self
            .tasks
            .lock()
            .expect("tasks lock")
            .values()
            .filter(|t| t.organization_id == *organization_id && t.status == status)
            .cloned()
            .collect())
    }
}

/// In-memory approval gate repository.
#[derive(Default)]
pub struct InMemoryApprovalGateRepository {
    gates: Mutex<HashMap<ApprovalGateId, ApprovalGate>>,
}

#[async_trait]
impl ApprovalGateRepository for InMemoryApprovalGateRepository {
    async fn create(
        &self,
        cmd: &CreateApprovalGateCommand,
    ) -> Result<ApprovalGate, RepositoryError> {
        let ts = now();
        let gate = ApprovalGate {
            id: ApprovalGateId::new(),
            organization_id: cmd.organization_id,
            action_id: cmd.action_id,
            approver_type: cmd.approver_type,
            approver_ref: cmd.approver_ref.clone(),
            status: crate::workflow::state_machine::GateStatus::Pending,
            decision_payload: None,
            deadline: cmd.deadline,
            decided_by: None,
            decided_at: None,
            lock_version: 1,
            created_at: ts,
        };
        self.gates
            .lock()
            .expect("gates lock")
            .insert(gate.id, gate.clone());
        Ok(gate)
    }

    async fn find_by_id(
        &self,
        id: &ApprovalGateId,
    ) -> Result<Option<ApprovalGate>, RepositoryError> {
        Ok(self.gates.lock().expect("gates lock").get(id).cloned())
    }

    async fn find_by_action(
        &self,
        action_id: &crate::workflow::action::WorkflowActionId,
    ) -> Result<Vec<ApprovalGate>, RepositoryError> {
        Ok(self
            .gates
            .lock()
            .expect("gates lock")
            .values()
            .filter(|g| g.action_id == *action_id)
            .cloned()
            .collect())
    }

    async fn find_pending(
        &self,
        organization_id: &OrganizationId,
    ) -> Result<Vec<ApprovalGate>, RepositoryError> {
        Ok(self
            .gates
            .lock()
            .expect("gates lock")
            .values()
            .filter(|g| {
                g.organization_id == *organization_id
                    && g.status == crate::workflow::state_machine::GateStatus::Pending
            })
            .cloned()
            .collect())
    }

    async fn update_cas(
        &self,
        gate: &ApprovalGate,
        expected_lock_version: i64,
    ) -> Result<ApprovalGate, RepositoryError> {
        let mut gates = self.gates.lock().expect("gates lock");
        let existing = gates
            .get(&gate.id)
            .ok_or_else(|| RepositoryError::NotFound(format!("approval_gate {}", gate.id)))?;
        if existing.lock_version != expected_lock_version {
            return Err(RepositoryError::ConcurrentModification {
                expected: expected_lock_version,
                actual: existing.lock_version,
            });
        }
        let mut updated = gate.clone();
        updated.lock_version = expected_lock_version + 1;
        gates.insert(gate.id, updated.clone());
        Ok(updated)
    }
}

/// In-memory dead letter repository.
#[derive(Default)]
pub struct InMemoryDeadLetterRepository {
    entries: Mutex<HashMap<DeadLetterId, DeadLetter>>,
}

#[async_trait]
impl DeadLetterRepository for InMemoryDeadLetterRepository {
    async fn enqueue(&self, entry: &DeadLetter) -> Result<DeadLetter, RepositoryError> {
        self.entries
            .lock()
            .expect("entries lock")
            .insert(entry.id, entry.clone());
        Ok(entry.clone())
    }

    async fn find_by_id(&self, id: &DeadLetterId) -> Result<Option<DeadLetter>, RepositoryError> {
        Ok(self.entries.lock().expect("entries lock").get(id).cloned())
    }

    async fn find_by_status(
        &self,
        organization_id: &OrganizationId,
        status: DeadLetterStatus,
    ) -> Result<Vec<DeadLetter>, RepositoryError> {
        Ok(self
            .entries
            .lock()
            .expect("entries lock")
            .values()
            .filter(|d| d.organization_id == *organization_id && d.status == status)
            .cloned()
            .collect())
    }

    async fn update_status(
        &self,
        id: &DeadLetterId,
        status: DeadLetterStatus,
        resolved_at: Option<DateTime<Utc>>,
    ) -> Result<DeadLetter, RepositoryError> {
        let mut entries = self.entries.lock().expect("entries lock");
        let entry = entries
            .get_mut(id)
            .ok_or_else(|| RepositoryError::NotFound(format!("dead_letter {id}")))?;
        entry.status = status;
        entry.resolved_at = resolved_at;
        Ok(entry.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::instance::OrganizationId;
    use crate::workflow::action::CreateActionCommand;
    use crate::workflow::agent_task::{Capability, CreateAgentTaskCommand};
    use crate::workflow::event::{CorrelationId, EventType, WorkflowEvent, WorkflowEventId};
    use crate::workflow::instance::{CreateInstanceCommand, WorkflowTemplate};
    use crate::workflow::rule::{ActionType, AutomationLevel};
    use crate::workflow::state_machine::InstanceStatus;
    use chrono::Duration;
    use uuid::Uuid;

    fn org() -> OrganizationId {
        OrganizationId::from_uuid(Uuid::nil())
    }

    fn make_event() -> WorkflowEvent {
        WorkflowEvent {
            id: WorkflowEventId::new(),
            organization_id: org(),
            project_id: None,
            correlation_id: CorrelationId::new(),
            event_type: EventType::AssetPublished,
            source_asset_id: AssetId::new(),
            source_asset_type_id: crate::asset::instance::AssetTypeId::new(),
            payload: serde_json::json!({}),
            cascade_depth: 0,
            triggering_action_id: None,
            idempotency_key: format!("asset_published:{}", Uuid::new_v4()),
            created_at: Utc::now(),
        }
    }

    fn make_promotion_rule(
        organization_id: OrganizationId,
        scope: crate::workflow::rule::RuleScope,
    ) -> crate::workflow::rule::PromotionRule {
        crate::workflow::rule::PromotionRule {
            id: crate::workflow::rule::PromotionRuleId::new(),
            organization_id,
            scope,
            scope_ref: None,
            event_type: EventType::AssetPublished,
            source_asset_type_id: None,
            mutex_group: None,
            rule_version: 1,
            priority: 0,
            automation_level: AutomationLevel::Automatic,
            filters: serde_json::json!({}),
            preconditions: serde_json::json!([]),
            action_template: crate::workflow::rule::ActionTemplate {
                action_type: ActionType::UpsertWorkItem,
                payload: serde_json::json!({"work_item_kind":"feature"}),
                is_required: true,
                order_index: 0,
            },
            max_cascade_depth: 5,
            effective_from: None,
            effective_to: None,
            rollout_segment: 100,
            enabled: true,
            dry_run: false,
            audit_only: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn event_append_and_idempotency() {
        let repo = InMemoryWorkflowEventRepository::default();
        let event = make_event();
        repo.append(&event).await.unwrap();
        // Same event id + type + source + correlation -> duplicate.
        let err = repo.append(&event).await.unwrap_err();
        assert!(matches!(err, RepositoryError::DuplicateIdempotencyKey(_)));
    }

    #[tokio::test]
    async fn event_can_be_reloaded_by_idempotency_key_after_duplicate() {
        let repo = InMemoryWorkflowEventRepository::default();
        let event = make_event();
        repo.append(&event).await.unwrap();

        let reloaded = repo
            .find_by_idempotency_key(&event.organization_id, &event.idempotency_key)
            .await
            .unwrap();

        assert_eq!(reloaded, Some(event));
    }

    #[tokio::test]
    async fn promotion_rules_load_only_effective_rules_for_requested_scope() {
        let repo = InMemoryPromotionRuleRepository::default();
        let now = Utc::now();
        let org_id = org();

        let mut org_rule =
            make_promotion_rule(org_id, crate::workflow::rule::RuleScope::Organization);
        org_rule.effective_from = Some(now - Duration::minutes(1));
        org_rule.effective_to = Some(now + Duration::minutes(1));
        let mut project_rule =
            make_promotion_rule(org_id, crate::workflow::rule::RuleScope::Project);
        project_rule.effective_from = Some(now + Duration::minutes(1));
        project_rule.effective_to = Some(now + Duration::minutes(2));

        repo.create(&org_rule).await.unwrap();
        repo.create(&project_rule).await.unwrap();

        let rules = repo
            .find_enabled_for(
                &org_id,
                EventType::AssetPublished,
                Some(crate::workflow::rule::RuleScope::Organization),
                now,
            )
            .await
            .unwrap();

        assert_eq!(rules, vec![org_rule]);
    }

    #[tokio::test]
    async fn instance_cas_detects_concurrent_modification() {
        let repo = InMemoryWorkflowInstanceRepository::default();
        let cmd = CreateInstanceCommand {
            organization_id: org(),
            project_id: None,
            correlation_id: CorrelationId::new(),
            template: WorkflowTemplate::Feature,
            cascade_depth: 0,
        };
        let instance = repo.create(&cmd).await.unwrap();
        repo.update_cas(&instance.id, 1, InstanceStatus::Ready)
            .await
            .unwrap();
        // Stale expected version 1 -> conflict.
        let err = repo
            .update_cas(&instance.id, 1, InstanceStatus::InProgress)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            RepositoryError::ConcurrentModification { .. }
        ));
    }

    #[tokio::test]
    async fn action_create_idempotency() {
        let inst_repo = InMemoryWorkflowInstanceRepository::default();
        let instance = inst_repo
            .create(&CreateInstanceCommand {
                organization_id: org(),
                project_id: None,
                correlation_id: CorrelationId::new(),
                template: WorkflowTemplate::Feature,
                cascade_depth: 0,
            })
            .await
            .unwrap();
        let repo = InMemoryWorkflowActionRepository::default();
        let cmd = CreateActionCommand {
            organization_id: org(),
            instance_id: instance.id,
            action_type: ActionType::UpsertWorkItem,
            target_asset_id: None,
            target_asset_type_id: None,
            idempotency_key: "k1".to_string(),
            preconditions: serde_json::json!([]),
            postconditions: serde_json::json!([]),
            automation_level: AutomationLevel::Automatic,
            is_required: true,
            order_index: 0,
            compensation_action_type: None,
            compensation_payload: None,
            compensation_policy: crate::workflow::action::CompensationPolicy::None,
            max_retries: 3,
        };
        let action = repo.create(&cmd).await.unwrap();
        let err = repo.create(&cmd).await.unwrap_err();
        assert!(matches!(err, RepositoryError::DuplicateIdempotencyKey(_)));

        let reloaded = repo
            .find_by_idempotency_key(&org(), &cmd.idempotency_key)
            .await
            .unwrap();
        assert_eq!(reloaded.map(|a| a.id), Some(action.id));
    }

    #[tokio::test]
    async fn agent_task_claim_is_atomic() {
        let inst_repo = InMemoryWorkflowInstanceRepository::default();
        let instance = inst_repo
            .create(&CreateInstanceCommand {
                organization_id: org(),
                project_id: None,
                correlation_id: CorrelationId::new(),
                template: WorkflowTemplate::Feature,
                cascade_depth: 0,
            })
            .await
            .unwrap();
        let action_repo = InMemoryWorkflowActionRepository::default();
        let action = action_repo
            .create(&CreateActionCommand {
                organization_id: org(),
                instance_id: instance.id,
                action_type: ActionType::CreateVirtualAssetContext,
                target_asset_id: None,
                target_asset_type_id: None,
                idempotency_key: "a1".to_string(),
                preconditions: serde_json::json!([]),
                postconditions: serde_json::json!([]),
                automation_level: AutomationLevel::AgentSuggested,
                is_required: true,
                order_index: 0,
                compensation_action_type: None,
                compensation_payload: None,
                compensation_policy: crate::workflow::action::CompensationPolicy::None,
                max_retries: 3,
            })
            .await
            .unwrap();
        let repo = InMemoryAgentTaskRepository::default();
        let task = repo
            .create(&CreateAgentTaskCommand {
                organization_id: org(),
                project_id: None,
                action_id: action.id,
                capability: Capability::create_virtual_asset_context(),
                idempotency_key: "t1".to_string(),
                expires_at: Some(Utc::now() + Duration::minutes(5)),
            })
            .await
            .unwrap();

        let now = Utc::now();
        let first = repo
            .claim_cas(&task.id, "agent-1", now, now + Duration::minutes(5))
            .await
            .unwrap();
        assert!(first.is_some());
        // Second claim on already-claimed task returns None (not an error).
        let second = repo
            .claim_cas(&task.id, "agent-2", now, now + Duration::minutes(5))
            .await
            .unwrap();
        assert!(second.is_none());
    }

    #[tokio::test]
    async fn dead_letter_status_update() {
        let repo = InMemoryDeadLetterRepository::default();
        let entry = DeadLetter {
            id: DeadLetterId::new(),
            organization_id: org(),
            project_id: None,
            source_type: crate::workflow::dead_letter::DeadLetterSource::Action,
            source_id: Uuid::nil(),
            reason: "exhausted".to_string(),
            context: serde_json::json!({}),
            status: DeadLetterStatus::Open,
            created_at: Utc::now(),
            resolved_at: None,
        };
        repo.enqueue(&entry).await.unwrap();
        let updated = repo
            .update_status(&entry.id, DeadLetterStatus::Resolved, Some(Utc::now()))
            .await
            .unwrap();
        assert_eq!(updated.status, DeadLetterStatus::Resolved);
    }
}
