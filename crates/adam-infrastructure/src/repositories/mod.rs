//! ADAM Infrastructure Repositories

use std::collections::HashMap;
use std::sync::Mutex;

use adam_domain::{AssetId, DependencyRepository, RepositoryError};
use async_trait::async_trait;

// PostgreSQL implementations
pub mod postgres;

/// In-memory implementation of DependencyRepository for testing and development
pub struct InMemoryDependencyRepository {
    /// Stores upstream dependencies: asset_id -> list of assets it depends on
    upstream: Mutex<HashMap<AssetId, Vec<AssetId>>>,
    /// Stores downstream dependencies: asset_id -> list of assets that depend on it
    downstream: Mutex<HashMap<AssetId, Vec<AssetId>>>,
}

impl InMemoryDependencyRepository {
    /// Create a new empty dependency repository
    pub fn new() -> Self {
        Self {
            upstream: Mutex::new(HashMap::new()),
            downstream: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryDependencyRepository {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DependencyRepository for InMemoryDependencyRepository {
    async fn find_downstream(&self, asset_id: &AssetId) -> Result<Vec<AssetId>, RepositoryError> {
        let downstream = self.downstream.lock().map_err(|_| {
            RepositoryError::DatabaseError("Failed to lock downstream map".to_string())
        })?;
        Ok(downstream.get(asset_id).cloned().unwrap_or_default())
    }

    async fn find_upstream(&self, asset_id: &AssetId) -> Result<Vec<AssetId>, RepositoryError> {
        let upstream = self.upstream.lock().map_err(|_| {
            RepositoryError::DatabaseError("Failed to lock upstream map".to_string())
        })?;
        Ok(upstream.get(asset_id).cloned().unwrap_or_default())
    }

    async fn create_dependency(
        &self,
        source_id: &AssetId,
        target_id: &AssetId,
    ) -> Result<(), RepositoryError> {
        // source depends on target (source -> target)
        let mut upstream = self.upstream.lock().map_err(|_| {
            RepositoryError::DatabaseError("Failed to lock upstream map".to_string())
        })?;
        upstream
            .entry(*source_id)
            .or_default()
            .push(*target_id);

        let mut downstream = self.downstream.lock().map_err(|_| {
            RepositoryError::DatabaseError("Failed to lock downstream map".to_string())
        })?;
        downstream
            .entry(*target_id)
            .or_default()
            .push(*source_id);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_and_find_dependencies() {
        let repo = InMemoryDependencyRepository::new();
        let asset_a = AssetId::new();
        let asset_b = AssetId::new();
        let asset_c = AssetId::new();

        // Create A -> B dependency (A depends on B)
        repo.create_dependency(&asset_a, &asset_b).await.unwrap();
        // Create A -> C dependency (A depends on C)
        repo.create_dependency(&asset_a, &asset_c).await.unwrap();

        // A's upstream should be [B, C]
        let upstream = repo.find_upstream(&asset_a).await.unwrap();
        assert_eq!(upstream.len(), 2);
        assert!(upstream.contains(&asset_b));
        assert!(upstream.contains(&asset_c));

        // B's downstream should be [A]
        let downstream = repo.find_downstream(&asset_b).await.unwrap();
        assert_eq!(downstream.len(), 1);
        assert!(downstream.contains(&asset_a));

        // C's downstream should be [A]
        let downstream = repo.find_downstream(&asset_c).await.unwrap();
        assert_eq!(downstream.len(), 1);
        assert!(downstream.contains(&asset_a));
    }

    #[tokio::test]
    async fn find_downstream_empty_when_no_dependencies() {
        let repo = InMemoryDependencyRepository::new();
        let asset = AssetId::new();

        let downstream = repo.find_downstream(&asset).await.unwrap();
        assert!(downstream.is_empty());
    }

    #[tokio::test]
    async fn find_upstream_empty_when_no_dependencies() {
        let repo = InMemoryDependencyRepository::new();
        let asset = AssetId::new();

        let upstream = repo.find_upstream(&asset).await.unwrap();
        assert!(upstream.is_empty());
    }
}
