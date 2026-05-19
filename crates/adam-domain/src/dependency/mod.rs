//! Dependency domain module

pub mod boundary;
pub mod compiled;
pub mod dag;
pub mod rule;

pub use compiled::{
    CompiledDependency, CompiledDependencyCache, CompilationError,
};
pub use rule::{DependencyRule, DependencyRuleId, DependencyRuleRepository, RelationshipType};
