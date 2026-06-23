//! Promotion rule evaluator — event → rule match → idempotent action creation.
//!
//! Implements design §8/§9/§10 for Slice 1. Given an appended
//! [`WorkflowEvent`], the evaluator:
//! 1. Loads enabled rules for the event type (consistency snapshot).
//! 2. Filters rules by source asset type and top-level payload filters.
//! 3. Evaluates enumerable preconditions.
//! 4. Resolves overlapping rules deterministically via [`resolve_conflicts`].
//! 5. For each winning rule, creates a [`WorkflowInstance`] (Saga skeleton)
//!    and a [`WorkflowAction`] idempotently — a duplicate idempotency key
//!    reloads the existing action.
//! 6. Enforces `cascade_depth < rule.max_cascade_depth`, recording
//!    [`WorkflowError::CascadeDepthExceeded`] otherwise (no action created).
//!
//! Dry-run / audit-only rules are logged via [`EvaluationOutcome::observed`]
//! but never produce actions (handled inside conflict resolution).

use std::sync::Arc;

use adam_domain::RepositoryError;
use adam_domain::workflow::WorkflowError;
use adam_domain::workflow::conflict::{ConflictResolution, resolve as resolve_conflicts};
use adam_domain::workflow::event::WorkflowEvent;
use adam_domain::workflow::idempotency::action_idempotency_key;
use adam_domain::workflow::instance::CreateInstanceCommand;
use adam_domain::workflow::repository::{
    PromotionRuleRepository, WorkflowActionRepository, WorkflowInstanceRepository,
};
use adam_domain::workflow::rule::PromotionRule;
use adam_domain::workflow::state_machine::InstanceStatus;

use super::rule_matching::{
    build_action_command, payload_filters_match, resolve_target_asset, rollout_bucket_for,
    source_type_matches, template_for,
};
use super::{ClockRef, SystemClock};

/// One action the evaluator decided to create (or reload) for an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedAction {
    pub rule_id: uuid::Uuid,
    pub action_id: uuid::Uuid,
    /// `true` when an existing action for this idempotency key was reloaded
    /// instead of a new one being created.
    pub reused: bool,
}

/// The outcome of evaluating rules for a single event.
#[derive(Debug, Clone, Default)]
pub struct EvaluationOutcome {
    /// Actions created or reloaded (winning active rules).
    pub created: Vec<CreatedAction>,
    /// All rules evaluated (active + dry-run/audit-only), for logging.
    pub evaluated: Vec<PromotionRule>,
    /// Cascade-depth violations recorded without creating an action.
    pub cascade_exceeded: Vec<CascadeViolation>,
}

/// A cascade-depth limit that was exceeded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CascadeViolation {
    pub rule_id: uuid::Uuid,
    pub depth: i32,
    pub max: i32,
}

/// Errors raised by [`PromotionRuleEvaluator`].
#[derive(Debug, thiserror::Error)]
pub enum RuleEvaluatorError {
    #[error("repository error: {0}")]
    Repository(RepositoryError),
    #[error("workflow error: {0}")]
    Workflow(WorkflowError),
}

impl From<RepositoryError> for RuleEvaluatorError {
    fn from(err: RepositoryError) -> Self {
        RuleEvaluatorError::Repository(err)
    }
}

impl From<WorkflowError> for RuleEvaluatorError {
    fn from(err: WorkflowError) -> Self {
        RuleEvaluatorError::Workflow(err)
    }
}

/// Evaluates promotion rules and idempotently creates workflow actions.
///
/// Generic over the three repositories it touches; the Postgres wiring shares
/// a single transaction so that event append + rule load + action create are
/// atomic (design §10). In the in-memory/test wiring each call is sequential.
#[derive(Clone)]
pub struct PromotionRuleEvaluator<RR, IR, AR>
where
    RR: PromotionRuleRepository + ?Sized,
    IR: WorkflowInstanceRepository + ?Sized,
    AR: WorkflowActionRepository + ?Sized,
{
    rule_repo: Arc<RR>,
    instance_repo: Arc<IR>,
    action_repo: Arc<AR>,
    clock: ClockRef,
}

impl<RR, IR, AR> PromotionRuleEvaluator<RR, IR, AR>
where
    RR: PromotionRuleRepository + ?Sized,
    IR: WorkflowInstanceRepository + ?Sized,
    AR: WorkflowActionRepository + ?Sized,
{
    /// Create a new evaluator backed by the given repositories.
    pub fn new(rule_repo: Arc<RR>, instance_repo: Arc<IR>, action_repo: Arc<AR>) -> Self {
        Self {
            rule_repo,
            instance_repo,
            action_repo,
            clock: Arc::new(SystemClock),
        }
    }

    /// Create a new evaluator with a custom clock (for tests).
    pub fn with_clock(
        rule_repo: Arc<RR>,
        instance_repo: Arc<IR>,
        action_repo: Arc<AR>,
        clock: ClockRef,
    ) -> Self {
        Self {
            rule_repo,
            instance_repo,
            action_repo,
            clock,
        }
    }

    /// Evaluate enabled rules for `event` and create/reload actions.
    pub async fn evaluate(
        &self,
        event: &WorkflowEvent,
    ) -> Result<EvaluationOutcome, RuleEvaluatorError> {
        let now = self.clock.now();
        let rules = self
            .rule_repo
            .find_enabled_for(&event.organization_id, event.event_type, None, now)
            .await?;

        // Source-asset-type and payload-filter matching.
        let matched: Vec<PromotionRule> = rules
            .into_iter()
            .filter(|r| source_type_matches(r, event))
            .filter(|r| payload_filters_match(r, event))
            .collect();

        let rollout_bucket = rollout_bucket_for(event.source_asset_id.0);
        let ConflictResolution { winners, evaluated } =
            resolve_conflicts(matched, now, rollout_bucket);

        let mut outcome = EvaluationOutcome {
            created: Vec::new(),
            evaluated,
            cascade_exceeded: Vec::new(),
        };

        for rule in winners {
            // Cascade depth guard (design §8): the event's cascade depth must
            // stay below the rule's limit. Violations are recorded and skip
            // action creation.
            if event.cascade_depth >= rule.max_cascade_depth {
                outcome.cascade_exceeded.push(CascadeViolation {
                    rule_id: rule.id.0,
                    depth: event.cascade_depth,
                    max: rule.max_cascade_depth,
                });
                continue;
            }

            let created = self.apply_winning_rule(rule, event).await?;
            outcome.created.push(created);
        }

        Ok(outcome)
    }

    async fn apply_winning_rule(
        &self,
        rule: PromotionRule,
        event: &WorkflowEvent,
    ) -> Result<CreatedAction, RuleEvaluatorError> {
        let template = rule.action_template.clone();
        let target_asset_id = resolve_target_asset(&template, event);

        let idempotency_key = action_idempotency_key(rule.id.0, event.id.0, target_asset_id);

        // Fast path: an action for this (rule, event, target) already exists.
        if let Some(existing) = self
            .action_repo
            .find_by_idempotency_key(&event.organization_id, &idempotency_key)
            .await?
        {
            return Ok(CreatedAction {
                rule_id: rule.id.0,
                action_id: existing.id.0,
                reused: true,
            });
        }

        // Create the Saga-coordinator instance for this action chain. The
        // instance starts in Pending and is advanced by the instance service.
        let instance = self
            .instance_repo
            .create(&CreateInstanceCommand {
                organization_id: event.organization_id,
                project_id: event.project_id,
                correlation_id: event.correlation_id,
                template: template_for(&rule),
                cascade_depth: event.cascade_depth + 1,
            })
            .await?;

        // Move the instance to Ready so the action can be picked up.
        self.instance_repo
            .update_cas(&instance.id, instance.lock_version, InstanceStatus::Ready)
            .await?;

        let cmd = build_action_command(&rule, &template, event, instance.id, idempotency_key);
        match self.action_repo.create(&cmd).await {
            Ok(action) => Ok(CreatedAction {
                rule_id: rule.id.0,
                action_id: action.id.0,
                reused: false,
            }),
            Err(RepositoryError::DuplicateIdempotencyKey(_)) => {
                // A concurrent worker won the action insert. The action's
                // unique idempotency key is the decisive guard; this worker
                // lost the race, so cancel the orphan instance we just created
                // and marked Ready (it owns no actions). Reloading the current
                // instance lock version keeps the CAS correct.
                let fresh = self.instance_repo.find_by_id(&instance.id).await?;
                if let Some(current) = fresh {
                    let _ = self
                        .instance_repo
                        .update_cas(
                            &instance.id,
                            current.lock_version,
                            InstanceStatus::Cancelled,
                        )
                        .await;
                }
                let existing = self
                    .action_repo
                    .find_by_idempotency_key(&event.organization_id, &cmd.idempotency_key)
                    .await?
                    .ok_or_else(|| {
                        RuleEvaluatorError::Workflow(WorkflowError::DuplicateIdempotencyKey(
                            cmd.idempotency_key.clone(),
                        ))
                    })?;
                Ok(CreatedAction {
                    rule_id: rule.id.0,
                    action_id: existing.id.0,
                    reused: true,
                })
            }
            Err(other) => Err(other.into()),
        }
    }
}

#[cfg(test)]
#[path = "rule_evaluator_tests.rs"]
mod tests;
