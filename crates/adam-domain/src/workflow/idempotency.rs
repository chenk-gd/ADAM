//! Idempotency key construction for workflow entities.
//!
//! Key shapes follow design §9:
//! - event:          `{event_type}:{source_asset_id}:{request_token}`
//! - rule -> action: `{rule_id}:{event_id}:{target_asset_id}`
//! - action:         `{workflow_instance_id}:{action_type}:{target_asset_id}`
//! - agent task:     `{action_id}:{capability}`

use uuid::Uuid;

use crate::asset::instance::AssetId;
use crate::workflow::event::EventType;

/// Build the idempotency key for a workflow event.
pub fn event_idempotency_key(
    event_type: EventType,
    source_asset_id: AssetId,
    request_token: &str,
) -> String {
    format!(
        "{}:{}:{}",
        event_type.as_str(),
        source_asset_id.0,
        request_token
    )
}

/// Build the idempotency key for a rule-created action.
pub fn action_idempotency_key(
    rule_id: Uuid,
    event_id: Uuid,
    target_asset_id: Option<AssetId>,
) -> String {
    let target = target_asset_id
        .map(|a| a.0.to_string())
        .unwrap_or_else(|| "none".to_string());
    format!("{rule_id}:{event_id}:{target}")
}

/// Build the idempotency key for a workflow action within an instance.
pub fn instance_action_idempotency_key(
    instance_id: Uuid,
    action_type: &str,
    target_asset_id: Option<AssetId>,
) -> String {
    let target = target_asset_id
        .map(|a| a.0.to_string())
        .unwrap_or_else(|| "none".to_string());
    format!("{instance_id}:{action_type}:{target}")
}

/// Build the idempotency key for an agent task.
pub fn agent_task_idempotency_key(action_id: Uuid, capability: &str) -> String {
    format!("{action_id}:{capability}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn event_key_is_deterministic() {
        let asset = AssetId::from_uuid(Uuid::nil());
        let a = event_idempotency_key(EventType::AssetPublished, asset, "req-1");
        let b = event_idempotency_key(EventType::AssetPublished, asset, "req-1");
        assert_eq!(a, b);
        assert!(a.contains("asset_published"));
    }

    #[test]
    fn event_key_differs_by_token() {
        let asset = AssetId::from_uuid(Uuid::nil());
        assert_ne!(
            event_idempotency_key(EventType::AssetPublished, asset, "req-1"),
            event_idempotency_key(EventType::AssetPublished, asset, "req-2")
        );
    }

    #[test]
    fn action_key_handles_missing_target() {
        let key = instance_action_idempotency_key(Uuid::nil(), "upsert_work_item", None);
        assert!(key.ends_with(":none"));
    }
}
