//! ADAM Infrastructure Repositories

// PostgreSQL implementations
pub mod postgres;

// Re-export PostgreSQL repository constructors
pub use postgres::{
    PostgresAssetRepository, PostgresAssetTypeRepository, PostgresAssetVersionRepository,
    PostgresDependencyRepository, PostgresDependencyRuleRepository, PostgresDirtyQueueRepository,
    PostgresDirtyResolutionLogRepository, PostgresVirtualInstanceRepository,
};
