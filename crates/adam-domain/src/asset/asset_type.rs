//! Asset type definition with metadata schema

use crate::asset::instance::{AssetTypeId, OrganizationId};
use serde::{Deserialize, Serialize};

/// Version strategy for asset types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionStrategy {
    /// Semantic versioning (major.minor.patch)
    Semver,
    /// Use external reference as version
    ExternalRef,
    /// Composite versioning (combination of sources)
    Composite,
}

impl Default for VersionStrategy {
    fn default() -> Self {
        Self::Semver
    }
}

impl std::fmt::Display for VersionStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VersionStrategy::Semver => write!(f, "semver"),
            VersionStrategy::ExternalRef => write!(f, "external_ref"),
            VersionStrategy::Composite => write!(f, "composite"),
        }
    }
}

/// Asset type with JSON Schema metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetType {
    pub id: AssetTypeId,
    pub organization_id: OrganizationId,
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub metadata_schema: serde_json::Value, // JSON Schema
    pub version_strategy: VersionStrategy,
    pub(crate) retention_policy: Option<serde_json::Value>,
    pub(crate) icon: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub(crate) updated_at: chrono::DateTime<chrono::Utc>,
}

impl AssetType {
    pub fn new(
        organization_id: OrganizationId,
        name: impl Into<String>,
        display_name: impl Into<String>,
        description: impl Into<String>,
        metadata_schema: serde_json::Value,
    ) -> Self {
        let now = chrono::Utc::now();
        Self {
            id: AssetTypeId::new(),
            organization_id,
            name: name.into(),
            display_name: display_name.into(),
            description: description.into(),
            metadata_schema,
            version_strategy: VersionStrategy::default(),
            retention_policy: None,
            icon: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Recreate an asset type from persisted fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_fields(
        id: AssetTypeId,
        organization_id: OrganizationId,
        name: String,
        display_name: String,
        description: String,
        metadata_schema: serde_json::Value,
        version_strategy: VersionStrategy,
        retention_policy: Option<serde_json::Value>,
        icon: Option<String>,
        created_at: chrono::DateTime<chrono::Utc>,
        updated_at: chrono::DateTime<chrono::Utc>,
    ) -> Self {
        Self {
            id,
            organization_id,
            name,
            display_name,
            description,
            metadata_schema,
            version_strategy,
            retention_policy,
            icon,
            created_at,
            updated_at,
        }
    }

    /// Set version strategy
    pub fn with_version_strategy(mut self, strategy: VersionStrategy) -> Self {
        self.version_strategy = strategy;
        self
    }

    /// Set the retention policy
    pub fn set_retention_policy(&mut self, policy: impl Into<Option<serde_json::Value>>) {
        self.retention_policy = policy.into();
        self.updated_at = chrono::Utc::now();
    }

    /// Get the retention policy
    pub fn retention_policy(&self) -> Option<&serde_json::Value> {
        self.retention_policy.as_ref()
    }

    /// Set the icon
    pub fn set_icon(&mut self, icon: impl Into<Option<String>>) {
        self.icon = icon.into();
        self.updated_at = chrono::Utc::now();
    }

    /// Get the icon
    pub fn icon(&self) -> Option<&str> {
        self.icon.as_deref()
    }

    /// Update the display name
    pub fn update_display_name(&mut self, display_name: impl Into<String>) {
        self.display_name = display_name.into();
        self.updated_at = chrono::Utc::now();
    }

    /// Update the description
    pub fn update_description(&mut self, description: impl Into<String>) {
        self.description = description.into();
        self.updated_at = chrono::Utc::now();
    }

    /// Update the metadata schema
    pub fn update_metadata_schema(&mut self, schema: serde_json::Value) {
        self.metadata_schema = schema;
        self.updated_at = chrono::Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_asset_type_creation() {
        let org_id = OrganizationId::new();
        let schema = json!({
            "type": "object",
            "properties": {
                "title": { "type": "string" }
            }
        });

        let asset_type = AssetType::new(org_id, "requirement", "需求", "功能需求文档", schema);

        assert_eq!(asset_type.name, "requirement");
        assert_eq!(asset_type.display_name, "需求");
        assert_eq!(asset_type.organization_id, org_id);
        assert_eq!(asset_type.version_strategy, VersionStrategy::Semver);
        assert!(!asset_type.id.0.to_string().is_empty());
    }

    #[test]
    fn test_version_strategy_display() {
        assert_eq!(VersionStrategy::Semver.to_string(), "semver");
        assert_eq!(VersionStrategy::ExternalRef.to_string(), "external_ref");
        assert_eq!(VersionStrategy::Composite.to_string(), "composite");
    }

    #[test]
    fn test_asset_type_with_version_strategy() {
        let org_id = OrganizationId::new();
        let asset_type = AssetType::new(org_id, "code", "代码", "代码提交", json!({}))
            .with_version_strategy(VersionStrategy::ExternalRef);

        assert_eq!(asset_type.version_strategy, VersionStrategy::ExternalRef);
    }
}
