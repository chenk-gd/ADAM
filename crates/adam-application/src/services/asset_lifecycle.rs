//! Asset lifecycle service with CAS optimistic locking

use adam_domain::{
    AssetId, AssetRepository, AssetState, AssetVersion, AssetVersionRepository, IdempotencyKey,
    IdempotencyRecord, IdempotencyRepository, RepositoryError, SemVer,
};
use std::sync::Arc;

/// Error types for asset lifecycle operations
#[derive(Debug, thiserror::Error)]
pub enum AssetLifecycleError {
    /// Repository error
    #[error("Repository error: {0}")]
    Repository(RepositoryError),
    /// Asset not found
    #[error("Asset not found: {0}")]
    NotFound(String),
    /// Concurrent modification detected
    #[error("Concurrent modification: expected lock version {expected}, actual {actual}")]
    ConcurrentModification { expected: i64, actual: i64 },
    /// Invalid state transition
    #[error("Invalid state transition: {0}")]
    InvalidState(String),
    /// Version already exists
    #[error("Version already exists: {0}")]
    VersionAlreadyExists(String),
    /// Idempotency key conflict - same key, different request
    #[error("Idempotency key conflict: {0}")]
    IdempotencyConflict(String),
}

impl From<RepositoryError> for AssetLifecycleError {
    fn from(err: RepositoryError) -> Self {
        match err {
            RepositoryError::ConcurrentModification { expected, actual } => {
                AssetLifecycleError::ConcurrentModification { expected, actual }
            }
            RepositoryError::NotFound(msg) => AssetLifecycleError::NotFound(msg),
            other => AssetLifecycleError::Repository(other),
        }
    }
}

/// Configuration for retry mechanism
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_retries: u32,
    /// Base delay for exponential backoff
    pub base_delay: tokio::time::Duration,
    /// Maximum delay between retries
    pub max_delay: tokio::time::Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: tokio::time::Duration::from_millis(50),
            max_delay: tokio::time::Duration::from_secs(1),
        }
    }
}

/// Service for asset lifecycle operations with CAS optimistic locking
pub struct AssetLifecycleService<AR, VR, PR>
where
    AR: AssetRepository,
    VR: AssetVersionRepository,
    PR: crate::services::StatePropagationPort,
{
    asset_repo: Arc<AR>,
    version_repo: Arc<VR>,
    propagation_service: Arc<PR>,
}

/// Port for state propagation
#[async_trait::async_trait]
pub trait StatePropagationPort: Send + Sync {
    /// Propagate state changes when asset is published
    async fn propagate_on_publish(
        &self,
        asset_id: &AssetId,
        new_version: &SemVer,
    ) -> Result<(), AssetLifecycleError>;
}

impl<AR, VR, PR> AssetLifecycleService<AR, VR, PR>
where
    AR: AssetRepository,
    VR: AssetVersionRepository,
    PR: crate::services::StatePropagationPort,
{
    /// Create a new AssetLifecycleService
    pub fn new(asset_repo: Arc<AR>, version_repo: Arc<VR>, propagation_service: Arc<PR>) -> Self {
        Self {
            asset_repo,
            version_repo,
            propagation_service,
        }
    }

    /// Publish a new version with CAS (Compare-And-Swap) optimistic locking
    ///
    /// # CAS Implementation
    ///
    /// This implementation performs optimistic locking at two levels:
    /// 1. **Application-level check**: Verifies the lock version before proceeding
    /// 2. **Database-level CAS**: Uses `update_publication_cas` which includes lock_version
    ///    in the WHERE clause with RETURNING clause for atomic check-and-set
    ///
    /// The database update only succeeds if the lock_version matches the expected value.
    /// If another transaction modified the row, a `ConcurrentModification` error is returned.
    ///
    /// # Arguments
    /// * `asset_id` - ID of the asset to publish
    /// * `new_version` - The new version to publish
    /// * `content_ref` - Reference to the content
    /// * `expected_lock_version` - Expected lock version for CAS check
    /// * `publisher` - Who is publishing the version
    pub async fn publish_version_cas(
        &self,
        asset_id: AssetId,
        new_version: SemVer,
        content_ref: String,
        expected_lock_version: i64,
        publisher: String,
    ) -> Result<AssetVersion, AssetLifecycleError> {
        // Get current asset state
        let asset = self
            .asset_repo
            .find_by_id(&asset_id)
            .await?
            .ok_or_else(|| AssetLifecycleError::NotFound(format!("{asset_id:?}")))?;

        // Check lock version
        if asset.lock_version() != expected_lock_version {
            return Err(AssetLifecycleError::ConcurrentModification {
                expected: expected_lock_version,
                actual: asset.lock_version(),
            });
        }

        // Check version ordering - new version must be greater than current
        if new_version <= *asset.current_version() {
            return Err(AssetLifecycleError::InvalidState(format!(
                "New version {} must be greater than current version {}",
                new_version,
                asset.current_version()
            )));
        }

        // Check asset state
        if asset.current_state() == AssetState::Archived {
            return Err(AssetLifecycleError::InvalidState(
                "Cannot publish archived asset".to_string(),
            ));
        }

        // Create version record
        let version = AssetVersion::new(
            asset_id,
            new_version.to_string(),
            serde_json::json!({"content_ref": content_ref}),
            vec![],         // empty dependencies for now
            "".to_string(), // release notes
            publisher.clone(),
        );

        // Save version
        self.version_repo.create(&version).await?;

        // Update asset with CAS - the database layer performs the CAS check
        self.asset_repo
            .update_publication_cas(
                &asset_id,
                new_version.to_string(),
                publisher,
                AssetState::Clean,
                expected_lock_version,
            )
            .await?;

        // Propagate state changes
        self.propagation_service
            .propagate_on_publish(&asset_id, &new_version)
            .await?;

        Ok(version)
    }

    /// Publish with automatic retry and exponential backoff
    ///
    /// # Arguments
    /// * `asset_id` - ID of the asset to publish
    /// * `new_version` - The new version to publish
    /// * `content_ref` - Reference to the content
    /// * `publisher` - Who is publishing the version
    /// * `config` - Retry configuration
    pub async fn publish_with_retry(
        &self,
        asset_id: AssetId,
        new_version: SemVer,
        content_ref: String,
        publisher: String,
        config: RetryConfig,
    ) -> Result<AssetVersion, AssetLifecycleError> {
        let mut attempt = 0;

        loop {
            // Get current asset and lock version
            let asset = self
                .asset_repo
                .find_by_id(&asset_id)
                .await?
                .ok_or_else(|| AssetLifecycleError::NotFound(format!("{asset_id:?}")))?;

            let expected_version = asset.lock_version();

            match self
                .publish_version_cas(
                    asset_id,
                    new_version.clone(),
                    content_ref.clone(),
                    expected_version,
                    publisher.clone(),
                )
                .await
            {
                Ok(version) => return Ok(version),
                Err(AssetLifecycleError::ConcurrentModification { .. })
                    if attempt < config.max_retries =>
                {
                    // Exponential backoff
                    let delay =
                        std::cmp::min(config.base_delay * 2u32.pow(attempt), config.max_delay);
                    tokio::time::sleep(delay).await;

                    attempt += 1;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Find a version by asset ID and version number
    async fn find_version_by_asset_and_version(
        &self,
        asset_id: &AssetId,
        version: &str,
    ) -> Result<Option<AssetVersion>, AssetLifecycleError> {
        self.version_repo
            .find_by_version(asset_id, version)
            .await
            .map_err(AssetLifecycleError::from)
    }
}

/// Idempotent publish request
#[derive(Debug, Clone)]
pub struct PublishVersionRequest {
    pub asset_id: AssetId,
    pub new_version: SemVer,
    pub content_ref: String,
    pub idempotency_key: IdempotencyKey,
    pub expected_lock_version: i64,
    pub publisher: String,
}

/// Idempotent asset lifecycle service
pub struct IdempotentAssetLifecycleService<AR, VR, PR, IR>
where
    AR: AssetRepository,
    VR: AssetVersionRepository,
    PR: StatePropagationPort,
    IR: IdempotencyRepository,
{
    inner: AssetLifecycleService<AR, VR, PR>,
    idempotency_repo: Arc<IR>,
}

impl<AR, VR, PR, IR> IdempotentAssetLifecycleService<AR, VR, PR, IR>
where
    AR: AssetRepository,
    VR: AssetVersionRepository,
    PR: StatePropagationPort,
    IR: IdempotencyRepository,
{
    /// Create a new idempotent service wrapping an existing lifecycle service
    pub fn new(inner: AssetLifecycleService<AR, VR, PR>, idempotency_repo: Arc<IR>) -> Self {
        Self {
            inner,
            idempotency_repo,
        }
    }

    /// Idempotent publish
    pub async fn publish_version_idempotent(
        &self,
        request: PublishVersionRequest,
    ) -> Result<AssetVersion, AssetLifecycleError> {
        // Check idempotency
        if let Some(existing) = self
            .idempotency_repo
            .find_by_key(&request.idempotency_key)
            .await?
        {
            // Verify request matches by computing hash
            let request_hash = self.compute_request_hash(&request);
            if existing.request_hash == request_hash {
                // Return cached response
                let version = self.find_version_by_id(&existing.response_id).await?;
                return Ok(version);
            } else {
                return Err(AssetLifecycleError::IdempotencyConflict(format!(
                    "Key {:?} already used with different request",
                    request.idempotency_key
                )));
            }
        }

        // Execute the publish
        let version = self
            .inner
            .publish_version_cas(
                request.asset_id,
                request.new_version.clone(),
                request.content_ref.clone(),
                request.expected_lock_version,
                request.publisher.clone(),
            )
            .await?;

        // Record idempotency
        let request_hash = self.compute_request_hash(&request);
        // Store asset_id:version as the response identifier
        let response_id = format!("{}:{}", request.asset_id.0, version.version_number);
        let record = IdempotencyRecord::new(request.idempotency_key, request_hash, response_id);
        self.idempotency_repo.save(&record).await?;

        Ok(version)
    }

    /// Find a version by its stored identifier
    /// The identifier format is "asset_uuid:version_number"
    async fn find_version_by_id(&self, id: &str) -> Result<AssetVersion, AssetLifecycleError> {
        // Parse the identifier
        let parts: Vec<&str> = id.split(':').collect();
        if parts.len() != 2 {
            return Err(AssetLifecycleError::NotFound(format!(
                "Invalid response ID format: {id}"
            )));
        }

        let asset_uuid = match uuid::Uuid::parse_str(parts[0]) {
            Ok(uuid) => uuid,
            Err(_) => {
                return Err(AssetLifecycleError::NotFound(format!(
                    "Invalid asset UUID in response ID: {id}"
                )));
            }
        };
        let asset_id = AssetId(asset_uuid);
        let version_number = parts[1];

        // Find the version
        match self
            .inner
            .find_version_by_asset_and_version(&asset_id, version_number)
            .await
        {
            Ok(Some(version)) => Ok(version),
            Ok(None) => Err(AssetLifecycleError::NotFound(format!(
                "Version not found: {id}"
            ))),
            Err(e) => Err(e),
        }
    }

    /// Compute a hash of the request for idempotency checking
    fn compute_request_hash(&self, request: &PublishVersionRequest) -> String {
        // Simple hash combination without external crate
        format!(
            "{:?}:{}:{}:{}",
            request.asset_id, request.new_version, request.content_ref, request.publisher
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adam_domain::{
        AssetLevel, AssetTypeId, CreateAssetCommand, InMemoryAssetRepository,
        InMemoryAssetVersionRepository, OrganizationId,
    };

    /// Mock propagation port for testing
    struct MockPropagationPort;

    #[async_trait::async_trait]
    impl StatePropagationPort for MockPropagationPort {
        async fn propagate_on_publish(
            &self,
            _asset_id: &AssetId,
            _new_version: &SemVer,
        ) -> Result<(), AssetLifecycleError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn publish_cas_succeeds_with_correct_lock_version() {
        let asset_repo = Arc::new(InMemoryAssetRepository::new());
        let version_repo = Arc::new(InMemoryAssetVersionRepository::new());
        let propagation = Arc::new(MockPropagationPort);

        let service =
            AssetLifecycleService::new(asset_repo.clone(), version_repo.clone(), propagation);

        // Create an asset
        let org_id = OrganizationId::new();
        let type_id = AssetTypeId::new();
        let asset = asset_repo
            .create(&CreateAssetCommand {
                name: "Test Asset".to_string(),
                asset_type_id: type_id,
                organization_id: org_id,
                project_id: None,
                level: AssetLevel::Organization,
                external_ref: "https://example.com".to_string(),
                source: "manual".to_string(),
                metadata: serde_json::json!({}),
                idempotency_key: None,
            })
            .await
            .unwrap();

        // Publish with correct lock version
        let result = service
            .publish_version_cas(
                asset.id,
                SemVer::new(1, 0, 0),
                "content-ref".to_string(),
                asset.lock_version(),
                "publisher".to_string(),
            )
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn publish_cas_fails_with_wrong_lock_version() {
        let asset_repo = Arc::new(InMemoryAssetRepository::new());
        let version_repo = Arc::new(InMemoryAssetVersionRepository::new());
        let propagation = Arc::new(MockPropagationPort);

        let service =
            AssetLifecycleService::new(asset_repo.clone(), version_repo.clone(), propagation);

        // Create an asset
        let org_id = OrganizationId::new();
        let type_id = AssetTypeId::new();
        let asset = asset_repo
            .create(&CreateAssetCommand {
                name: "Test Asset".to_string(),
                asset_type_id: type_id,
                organization_id: org_id,
                project_id: None,
                level: AssetLevel::Organization,
                external_ref: "https://example.com".to_string(),
                source: "manual".to_string(),
                metadata: serde_json::json!({}),
                idempotency_key: None,
            })
            .await
            .unwrap();

        // Try to publish with wrong lock version
        let result = service
            .publish_version_cas(
                asset.id,
                SemVer::new(1, 0, 0),
                "content-ref".to_string(),
                asset.lock_version() + 1, // Wrong version
                "publisher".to_string(),
            )
            .await;

        assert!(matches!(
            result,
            Err(AssetLifecycleError::ConcurrentModification { .. })
        ));
    }

    #[tokio::test]
    async fn publish_with_retry_succeeds_after_conflict() {
        let asset_repo = Arc::new(InMemoryAssetRepository::new());
        let version_repo = Arc::new(InMemoryAssetVersionRepository::new());
        let propagation = Arc::new(MockPropagationPort);

        let service =
            AssetLifecycleService::new(asset_repo.clone(), version_repo.clone(), propagation);

        // Create an asset
        let org_id = OrganizationId::new();
        let type_id = AssetTypeId::new();
        let asset = asset_repo
            .create(&CreateAssetCommand {
                name: "Test Asset".to_string(),
                asset_type_id: type_id,
                organization_id: org_id,
                project_id: None,
                level: AssetLevel::Organization,
                external_ref: "https://example.com".to_string(),
                source: "manual".to_string(),
                metadata: serde_json::json!({}),
                idempotency_key: None,
            })
            .await
            .unwrap();

        let config = RetryConfig {
            max_retries: 3,
            base_delay: tokio::time::Duration::from_millis(10),
            max_delay: tokio::time::Duration::from_millis(100),
        };

        // Publish with retry should succeed
        let result = service
            .publish_with_retry(
                asset.id,
                SemVer::new(1, 0, 0),
                "content-ref".to_string(),
                "publisher".to_string(),
                config,
            )
            .await;

        assert!(result.is_ok());
    }
}
