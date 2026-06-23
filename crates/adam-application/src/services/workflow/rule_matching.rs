//! Pure helpers for the rule evaluator: rule/event matching and action
//! command construction. Kept separate from [`super::rule_evaluator`] so the
//! evaluator module stays under the file-size budget.

use adam_domain::workflow::action::CreateActionCommand;
use adam_domain::workflow::event::WorkflowEvent;
use adam_domain::workflow::instance::WorkflowInstanceId;
use adam_domain::workflow::rule::{ActionTemplate, PromotionRule};

/// Build the action creation command from a rule template and event.
pub(super) fn build_action_command(
    rule: &PromotionRule,
    template: &ActionTemplate,
    event: &WorkflowEvent,
    instance_id: WorkflowInstanceId,
    idempotency_key: String,
) -> CreateActionCommand {
    CreateActionCommand {
        organization_id: event.organization_id,
        instance_id,
        action_type: template.action_type,
        target_asset_id: resolve_target_asset(template, event),
        target_asset_type_id: rule.source_asset_type_id,
        idempotency_key,
        preconditions: rule.preconditions.clone(),
        postconditions: template.payload.clone(),
        automation_level: rule.automation_level,
        is_required: template.is_required,
        order_index: template.order_index,
        compensation_action_type: None,
        compensation_payload: None,
        compensation_policy: rule
            .action_template
            .payload
            .get("compensation_policy")
            .and_then(|v| v.as_str())
            .map(|_| adam_domain::workflow::action::CompensationPolicy::None)
            .unwrap_or_default(),
        max_retries: 3,
    }
}

/// A rule matches the event's source asset type when the rule declares no
/// restriction or declares the event's exact type.
pub(super) fn source_type_matches(rule: &PromotionRule, event: &WorkflowEvent) -> bool {
    match rule.source_asset_type_id {
        None => true,
        Some(t) => t == event.source_asset_type_id,
    }
}

/// Top-level exact-match filter: every key in `rule.filters` must equal the
/// same key in the event payload. An empty/null filter always matches.
pub(super) fn payload_filters_match(rule: &PromotionRule, event: &WorkflowEvent) -> bool {
    let Some(obj) = rule.filters.as_object() else {
        return true;
    };
    if obj.is_empty() {
        return true;
    }
    let Some(payload) = event.payload.as_object() else {
        return false;
    };
    obj.iter().all(|(k, v)| payload.get(k) == Some(v))
}

/// Resolve the target asset for the created action. For Slice 1 the seeded
/// rule targets the source requirement; later slices may derive other targets.
pub(super) fn resolve_target_asset(
    _template: &ActionTemplate,
    event: &WorkflowEvent,
) -> Option<adam_domain::AssetId> {
    // The seeded `requirement publish -> upsert work_item(feature)` rule
    // operates on the publishing requirement itself.
    Some(event.source_asset_id)
}

/// Map the rule to a workflow template. Slice 1 only seeds the feature path.
pub(super) fn template_for(
    rule: &PromotionRule,
) -> adam_domain::workflow::instance::WorkflowTemplate {
    use adam_domain::workflow::rule::ActionType;
    match rule.action_template.action_type {
        ActionType::UpsertWorkItem => adam_domain::workflow::instance::WorkflowTemplate::Feature,
        ActionType::CreateBugfixWork => adam_domain::workflow::instance::WorkflowTemplate::Bugfix,
        ActionType::CreateVirtualAssetContext
        | ActionType::RequestApproval
        | ActionType::ResolveDirty => adam_domain::workflow::instance::WorkflowTemplate::Feature,
    }
}

/// Deterministic rollout bucket for an asset id (mirrors the domain helper).
pub(super) fn rollout_bucket_for(target: uuid::Uuid) -> u8 {
    let bytes = target.as_bytes();
    let hi = u64::from_be_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]);
    (hi % 101) as u8
}
