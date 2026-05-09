//! Dependency domain module

pub mod boundary;
pub mod dag;
pub mod rule;

pub use rule::{DependencyRule, DependencyRuleId, DependencyRuleRepository, RelationshipType};
