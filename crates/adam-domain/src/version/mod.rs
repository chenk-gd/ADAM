//! Version module
pub mod constraint;
pub mod semver;
pub use constraint::{Bound, VersionConstraint};
pub use semver::SemVer;
