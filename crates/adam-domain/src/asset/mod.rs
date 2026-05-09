//! Asset domain module

pub mod asset_type;
pub mod instance;
pub mod state;
pub mod version;

pub use asset_type::AssetType;
pub use instance::{AssetId, AssetInstance, AssetTypeId, OrganizationId, ProjectId};
pub use version::{AssetVersion, AssetVersionId, AssetVersionRepository, DependencySnapshot, SemVer};
