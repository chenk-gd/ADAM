//! ADAM Domain - Core domain layer for the Asset Management System

pub mod asset;
pub mod dependency;
pub mod repository;

pub use asset::instance::{AssetId, AssetInstance, AssetTypeId, OrganizationId, ProjectId};
pub use asset::state::AssetState;
pub use dependency::boundary::{AssetLevel, DependencyBoundaryContext, DependencyError};
pub use dependency::dag::{DAGError, DAGValidator};
pub use repository::in_memory::{InMemoryAssetRepository, InMemoryDirtyQueueRepository};
pub use repository::{
    AssetRepository, CreateAssetCommand, DependencyRepository, DirtyQueueEntry,
    DirtyQueueRepository, RepositoryError,
};
