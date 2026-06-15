//! Idempotency support for ADAM
//!
//! Provides idempotent operation tracking to prevent duplicate processing
//! of the same request.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::RepositoryError;

/// Unique identifier for idempotency records
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IdempotencyKey(pub String);

impl IdempotencyKey {
    /// Create a new idempotency key from a string
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// Generate a unique idempotency key based on organization and request details
    pub fn generate(org_id: Uuid, resource_type: &str, unique_id: &str) -> Self {
        Self(format!("{org_id}:{resource_type}:{unique_id}"))
    }
}

impl std::fmt::Display for IdempotencyKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Record of an idempotent operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdempotencyRecord {
    /// The idempotency key
    pub key: IdempotencyKey,
    /// Hash of the request for verification
    pub request_hash: String,
    /// ID of the response (e.g., AssetVersionId)
    pub response_id: String,
    /// When the record was created
    pub created_at: DateTime<Utc>,
    /// When the record expires (for cleanup)
    pub expires_at: DateTime<Utc>,
}

impl IdempotencyRecord {
    /// Create a new idempotency record
    pub fn new(
        key: IdempotencyKey,
        request_hash: impl Into<String>,
        response_id: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            key,
            request_hash: request_hash.into(),
            response_id: response_id.into(),
            created_at: now,
            expires_at: now + chrono::Duration::hours(24),
        }
    }

    /// Check if the record has expired
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }
}

/// Repository trait for idempotency records
#[async_trait::async_trait]
pub trait IdempotencyRepository: Send + Sync {
    /// Find a record by its key
    async fn find_by_key(
        &self,
        key: &IdempotencyKey,
    ) -> Result<Option<IdempotencyRecord>, RepositoryError>;

    /// Save a new idempotency record
    async fn save(&self, record: &IdempotencyRecord) -> Result<(), RepositoryError>;

    /// Delete expired records
    async fn delete_expired(&self) -> Result<u64, RepositoryError>;
}

/// In-memory implementation of IdempotencyRepository
#[derive(Default)]
pub struct InMemoryIdempotencyRepository {
    records: std::sync::Mutex<std::collections::HashMap<String, IdempotencyRecord>>,
}

impl InMemoryIdempotencyRepository {
    /// Create a new in-memory repository
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl IdempotencyRepository for InMemoryIdempotencyRepository {
    async fn find_by_key(
        &self,
        key: &IdempotencyKey,
    ) -> Result<Option<IdempotencyRecord>, RepositoryError> {
        let records = self.records.lock().unwrap();
        Ok(records.get(&key.0).cloned())
    }

    async fn save(&self, record: &IdempotencyRecord) -> Result<(), RepositoryError> {
        let mut records = self.records.lock().unwrap();
        records.insert(record.key.0.clone(), record.clone());
        Ok(())
    }

    async fn delete_expired(&self) -> Result<u64, RepositoryError> {
        let mut records = self.records.lock().unwrap();
        let expired_keys: Vec<String> = records
            .values()
            .filter(|r| r.is_expired())
            .map(|r| r.key.0.clone())
            .collect();

        let count = expired_keys.len();
        for key in expired_keys {
            records.remove(&key);
        }

        Ok(count as u64)
    }
}

/// Error types for idempotency operations
#[derive(Debug, thiserror::Error)]
pub enum IdempotencyError {
    /// Idempotency key conflict - same key, different request
    #[error("Idempotency key conflict: {0}")]
    Conflict(String),
    /// Repository error
    #[error("Repository error: {0}")]
    Repository(#[from] RepositoryError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idempotency_key_display() {
        let key = IdempotencyKey::new("test-key");
        assert_eq!(key.to_string(), "test-key");
    }

    #[test]
    fn idempotency_key_generate() {
        let org_id = Uuid::new_v4();
        let key = IdempotencyKey::generate(org_id, "publish", "abc123");
        assert!(key.0.contains("publish"));
        assert!(key.0.contains("abc123"));
    }

    #[test]
    fn record_is_expired() {
        let record = IdempotencyRecord {
            key: IdempotencyKey::new("test"),
            request_hash: "hash".to_string(),
            response_id: "response".to_string(),
            created_at: Utc::now() - chrono::Duration::hours(25),
            expires_at: Utc::now() - chrono::Duration::hours(1),
        };
        assert!(record.is_expired());
    }

    #[test]
    fn record_is_not_expired() {
        let record = IdempotencyRecord::new(IdempotencyKey::new("test"), "hash", "response");
        assert!(!record.is_expired());
    }

    #[tokio::test]
    async fn memory_repo_saves_and_finds() {
        let repo = InMemoryIdempotencyRepository::new();
        let key = IdempotencyKey::new("test-key");
        let record = IdempotencyRecord::new(key.clone(), "hash123", "response456");

        repo.save(&record).await.unwrap();

        let found = repo.find_by_key(&key).await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().request_hash, "hash123");
    }

    #[tokio::test]
    async fn memory_repo_returns_none_for_missing() {
        let repo = InMemoryIdempotencyRepository::new();
        let key = IdempotencyKey::new("missing");

        let found = repo.find_by_key(&key).await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn memory_repo_deletes_expired() {
        let repo = InMemoryIdempotencyRepository::new();

        // Create expired record
        let expired = IdempotencyRecord {
            key: IdempotencyKey::new("expired"),
            request_hash: "hash".to_string(),
            response_id: "response".to_string(),
            created_at: Utc::now() - chrono::Duration::hours(25),
            expires_at: Utc::now() - chrono::Duration::hours(1),
        };

        // Create valid record
        let valid = IdempotencyRecord::new(IdempotencyKey::new("valid"), "hash", "response");

        repo.save(&expired).await.unwrap();
        repo.save(&valid).await.unwrap();

        let deleted = repo.delete_expired().await.unwrap();
        assert_eq!(deleted, 1);

        let found_expired = repo
            .find_by_key(&IdempotencyKey::new("expired"))
            .await
            .unwrap();
        assert!(found_expired.is_none());

        let found_valid = repo
            .find_by_key(&IdempotencyKey::new("valid"))
            .await
            .unwrap();
        assert!(found_valid.is_some());
    }
}
