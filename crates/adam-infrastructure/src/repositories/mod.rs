//! ADAM Infrastructure Repositories

// PostgreSQL implementations
pub mod postgres;
pub mod workflow_postgres;

// Re-export PostgreSQL repository constructors
pub use postgres::{
    PostgresAssetRepository, PostgresAssetTypeRepository, PostgresAssetVersionRepository,
    PostgresDependencyRepository, PostgresDependencyRuleRepository, PostgresDirtyQueueRepository,
    PostgresDirtyResolutionLogRepository, PostgresVirtualInstanceRepository,
};
pub use workflow_postgres::{
    PostgresAgentTaskRepository, PostgresPromotionRuleRepository, PostgresWorkflowActionRepository,
    PostgresWorkflowEventRepository, PostgresWorkflowInstanceRepository,
};
