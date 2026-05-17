//! ADAM Infrastructure - Infrastructure layer (database, external services)

pub mod repositories;

// Re-export PostgreSQL repositories
#[cfg(feature = "postgres")]
pub use repositories::postgres::{
    PostgresAssetRepository, PostgresAssetTypeRepository, PostgresAssetVersionRepository,
    PostgresDependencyRepository, PostgresDirtyQueueRepository,
    PostgresDirtyResolutionLogRepository, PostgresVirtualInstanceRepository,
};
