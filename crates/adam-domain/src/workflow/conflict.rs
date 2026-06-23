//! Promotion rule conflict resolution.
//!
//! Resolves overlapping rules deterministically per design §8 (lines 516-524):
//! 1. Rule scope: most specific first (`AssetType` > `Project` > `Organization`).
//! 2. Only rules inside their effective time window and rollout segment.
//! 3. Rules in the same `mutex_group` are mutually exclusive for the same
//!    event and target.
//! 4. For the same `mutex_group`, higher `rule_version` wins, then higher
//!    `priority` wins.
//! 5. Same priority and target: safer automation level wins
//!    (`HumanOnly` > `HumanApprovalRequired` > `AgentSuggested` > `Automatic`).
//! 6. Same scope, version, priority, and automation level: first-created
//!    (lower `rule_id`) wins.
//! 7. Dry-run and audit-only rules never suppress active rules, but all
//!    evaluated rules are logged.

use chrono::{DateTime, Utc};

use crate::workflow::rule::{PromotionRule, RuleScope};

/// Outcome of conflict resolution for one event/target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictResolution {
    /// The winning active rules (dry-run/audit-only excluded from winners).
    pub winners: Vec<PromotionRule>,
    /// All rules evaluated, including dry-run/audit-only, for logging.
    pub evaluated: Vec<PromotionRule>,
}

/// Resolve overlapping promotion rules for a single event and target.
///
/// `rules` should be the full set of enabled rules matching the event type.
/// `rollout_bucket` is the caller's resolved bucket in `0..=100`; a rule is
/// eligible only when `rollout_segment >= rollout_bucket`.
pub fn resolve(
    rules: Vec<PromotionRule>,
    now: DateTime<Utc>,
    rollout_bucket: u8,
) -> ConflictResolution {
    let evaluated = rules.clone();

    // Split active rules from observe-only (dry-run / audit-only). Observe-only
    // rules are logged but never participate in mutex exclusion and never win
    // (design §8 rule 7: they never suppress active rules).
    let (active_candidates, _observe_only): (Vec<PromotionRule>, Vec<PromotionRule>) = rules
        .into_iter()
        .filter(|r| r.is_effective_at(now))
        .filter(|r| r.rollout_segment as u8 >= rollout_bucket)
        .partition(|r| !r.dry_run && !r.audit_only);

    // Sort so that the "best" rule per mutex group lands first.
    let mut eligible = active_candidates;
    eligible.sort_by(compare_for_conflict);

    // Deduplicate by mutex_group: keep the first (best) per group. Rules with
    // no mutex_group are independent and all win.
    let mut seen_groups: Vec<Option<String>> = Vec::new();
    let mut winners: Vec<PromotionRule> = Vec::new();
    for rule in eligible {
        let group_key = rule.mutex_group.as_ref().map(|g| g.0.clone());
        if let Some(g) = &group_key {
            if seen_groups
                .iter()
                .any(|seen| seen.as_deref() == Some(g.as_str()))
            {
                continue;
            }
        }
        seen_groups.push(group_key);
        winners.push(rule);
    }

    ConflictResolution { winners, evaluated }
}

/// Comparator implementing the tie-break ladder.
///
/// Returns `Ordering::Less` when `a` should win over `b`.
fn compare_for_conflict(a: &PromotionRule, b: &PromotionRule) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    // 1. Most specific scope first.
    b.scope
        .specificity()
        .cmp(&a.scope.specificity())
        .then_with(|| {
            // 4. Higher rule_version wins.
            b.rule_version.cmp(&a.rule_version)
        })
        .then_with(|| {
            // 4b. Higher priority wins.
            b.priority.cmp(&a.priority)
        })
        .then_with(|| {
            // 5. Safer automation level wins.
            b.automation_level
                .safety_rank()
                .cmp(&a.automation_level.safety_rank())
        })
        .then_with(|| {
            // 6. First-created (lower rule_id) wins.
            a.id.0.cmp(&b.id.0)
        })
        .then(Ordering::Equal)
}

/// Map a target asset id deterministically to a rollout bucket in `0..=100`.
///
/// Uses the high 64 bits of the UUID so the same target always lands in the
/// same bucket, enabling stable staged rollout.
pub fn rollout_bucket_for(target: uuid::Uuid) -> u8 {
    let bytes = target.as_bytes();
    let hi = u64::from_be_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]);
    (hi % 101) as u8
}

/// Effective scope rank used for sorting convenience.
pub fn scope_rank(scope: RuleScope) -> u8 {
    scope.specificity()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::instance::OrganizationId;
    use crate::workflow::event::EventType;
    use crate::workflow::rule::{
        ActionTemplate, AutomationLevel, PromotionRule, PromotionRuleId, RuleScope,
    };
    use chrono::Utc;
    use uuid::Uuid;

    fn rule(
        id: u128,
        scope: RuleScope,
        version: i32,
        priority: i32,
        level: AutomationLevel,
        mutex: Option<&str>,
    ) -> PromotionRule {
        PromotionRule {
            id: PromotionRuleId::from_uuid(Uuid::from_u128(id)),
            organization_id: OrganizationId::from_uuid(Uuid::nil()),
            scope,
            scope_ref: None,
            event_type: EventType::AssetPublished,
            source_asset_type_id: None,
            mutex_group: mutex.map(crate::workflow::rule::MutexGroup::new),
            rule_version: version,
            priority,
            automation_level: level,
            filters: serde_json::json!({}),
            preconditions: serde_json::json!([]),
            action_template: ActionTemplate {
                action_type: crate::workflow::rule::ActionType::UpsertWorkItem,
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

    #[test]
    fn more_specific_scope_wins() {
        let org = rule(
            1,
            RuleScope::Organization,
            1,
            0,
            AutomationLevel::Automatic,
            Some("g"),
        );
        let at = rule(
            2,
            RuleScope::AssetType,
            1,
            0,
            AutomationLevel::Automatic,
            Some("g"),
        );
        let res = resolve(vec![org, at], Utc::now(), 0);
        assert_eq!(res.winners.len(), 1);
        assert_eq!(res.winners[0].scope, RuleScope::AssetType);
    }

    #[test]
    fn higher_version_wins_in_same_mutex_group() {
        let v1 = rule(
            1,
            RuleScope::Organization,
            1,
            0,
            AutomationLevel::Automatic,
            Some("g"),
        );
        let v2 = rule(
            2,
            RuleScope::Organization,
            2,
            0,
            AutomationLevel::Automatic,
            Some("g"),
        );
        let res = resolve(vec![v1, v2], Utc::now(), 0);
        assert_eq!(res.winners.len(), 1);
        assert_eq!(res.winners[0].rule_version, 2);
    }

    #[test]
    fn safer_automation_level_wins_on_tie() {
        let auto = rule(
            1,
            RuleScope::Organization,
            1,
            0,
            AutomationLevel::Automatic,
            Some("g"),
        );
        let human = rule(
            2,
            RuleScope::Organization,
            1,
            0,
            AutomationLevel::HumanOnly,
            Some("g"),
        );
        let res = resolve(vec![auto, human], Utc::now(), 0);
        assert_eq!(res.winners.len(), 1);
        assert_eq!(res.winners[0].automation_level, AutomationLevel::HumanOnly);
    }

    #[test]
    fn non_mutex_rules_all_win() {
        let a = rule(
            1,
            RuleScope::Organization,
            1,
            0,
            AutomationLevel::Automatic,
            None,
        );
        let b = rule(
            2,
            RuleScope::Organization,
            1,
            0,
            AutomationLevel::Automatic,
            None,
        );
        let res = resolve(vec![a, b], Utc::now(), 0);
        assert_eq!(res.winners.len(), 2);
    }

    #[test]
    fn dry_run_rule_never_wins_but_is_logged() {
        let mut active = rule(
            1,
            RuleScope::Organization,
            1,
            0,
            AutomationLevel::Automatic,
            Some("g"),
        );
        let mut dry = rule(
            2,
            RuleScope::Organization,
            9,
            9,
            AutomationLevel::Automatic,
            Some("g"),
        );
        dry.dry_run = true;
        dry.rule_version = 9;
        active.rule_version = 1;
        let res = resolve(vec![active, dry], Utc::now(), 0);
        assert_eq!(res.winners.len(), 1);
        assert!(!res.winners[0].dry_run);
        assert_eq!(res.evaluated.len(), 2);
    }

    #[test]
    fn rollout_segment_filters_rules() {
        let mut r = rule(
            1,
            RuleScope::Organization,
            1,
            0,
            AutomationLevel::Automatic,
            None,
        );
        r.rollout_segment = 50;
        // bucket 80 >= segment 50? no: rule eligible when segment >= bucket.
        // segment=50, bucket=80 -> 50 >= 80 false -> excluded.
        let res = resolve(vec![r], Utc::now(), 80);
        assert!(res.winners.is_empty());
    }

    #[test]
    fn disabled_rule_excluded() {
        let mut r = rule(
            1,
            RuleScope::Organization,
            1,
            0,
            AutomationLevel::Automatic,
            None,
        );
        r.enabled = false;
        let res = resolve(vec![r], Utc::now(), 0);
        assert!(res.winners.is_empty());
    }

    #[test]
    fn rollout_bucket_is_stable() {
        let id = Uuid::from_u128(42);
        assert_eq!(rollout_bucket_for(id), rollout_bucket_for(id));
        assert!(rollout_bucket_for(id) <= 100);
    }
}
