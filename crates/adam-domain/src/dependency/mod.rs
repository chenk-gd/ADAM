//! Dependency domain module

pub mod boundary;
pub mod compiled;
pub mod dag;
pub mod rule;

pub use compiled::{CompilationError, CompiledDependency, CompiledDependencyCache};
pub use rule::{
    DependencyRule, DependencyRuleId, DependencyRuleRepository, PropagationPolicy,
    RelationshipType, UnknownPropagationPolicy, UnknownRelationshipType,
};
