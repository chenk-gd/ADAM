//! Asset type definition with metadata schema

use crate::asset::instance::AssetTypeId;
use serde::{Deserialize, Serialize};

/// Asset type with JSON Schema metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetType {
    pub id: AssetTypeId,
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub metadata_schema: serde_json::Value, // JSON Schema
    pub retention_policy: Option<serde_json::Value>,
    pub icon: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl AssetType {
    pub fn new(
        name: impl Into<String>,
        display_name: impl Into<String>,
        description: impl Into<String>,
        metadata_schema: serde_json::Value,
    ) -> Self {
        let now = chrono::Utc::now();
        Self {
            id: AssetTypeId::new(),
            name: name.into(),
            display_name: display_name.into(),
            description: description.into(),
            metadata_schema,
            retention_policy: None,
            icon: None,
            created_at: now,
            updated_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_asset_type_creation() {
        let schema = json!({
            "type": "object",
            "properties": {
                "title": { "type": "string" }
            }
        });

        let asset_type = AssetType::new(
            "requirement",
            "需求",
            "功能需求文档",
            schema,
        );

        assert_eq!(asset_type.name, "requirement");
        assert_eq!(asset_type.display_name, "需求");
        assert!(!asset_type.id.0.to_string().is_empty());
    }
}
