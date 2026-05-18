//! ADAM Domain - Core domain layer for the Asset Management System

pub mod asset;
pub mod auth;
pub mod dependency;
pub mod repository;
pub mod version;
pub mod virtual_instance;

pub use asset::asset_type::{AssetType, VersionStrategy};
pub use asset::instance::{AssetId, AssetInstance, AssetTypeId, OrganizationId, ProjectId};
pub use asset::state::{AssetState, StateError};
pub use asset::version::{AssetVersion, AssetVersionId, AssetVersionRepository};
pub use auth::{AuthPrincipal, AuthorizationError, AuthorizationService, Permission, Role};
pub use dependency::boundary::{AssetLevel, DependencyBoundaryContext, DependencyError};
pub use dependency::dag::{DAGError, DAGValidator};
pub use dependency::{
    DependencyRule, DependencyRuleId, DependencyRuleRepository, RelationshipType,
};
pub use repository::in_memory::{
    InMemoryAssetRepository, InMemoryAssetTypeRepository, InMemoryAssetVersionRepository,
    InMemoryDependencyRepository, InMemoryDirtyQueueRepository,
    InMemoryDirtyResolutionLogRepository, InMemoryVirtualInstanceRepository,
};
pub use repository::{
    AssetDependencyRecord, AssetRepository, AssetTypeRepository, CreateAssetCommand,
    DependencyRepository, DirtyQueueEntry, DirtyQueueRepository, DirtyResolutionLog,
    DirtyResolutionLogRepository, EffectiveUpdateReason, RepositoryError, UpdateAssetCommand,
};
pub use version::SemVer;
pub use virtual_instance::{VirtualInstance, VirtualInstanceId, VirtualInstanceRepository};
