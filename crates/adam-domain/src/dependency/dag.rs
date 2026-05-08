//! DAG (Directed Acyclic Graph) validation module
//!
//! Provides cycle detection for dependency graphs to enforce BR-006.

use petgraph::algo::is_cyclic_directed;
use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::HashMap;
use std::hash::Hash;
use thiserror::Error;

/// Errors that can occur during DAG validation
#[derive(Debug, Error, PartialEq, Clone)]
pub enum DAGError {
    /// A cycle was detected in the dependency graph
    #[error("Cycle detected in dependency graph: {0}")]
    CycleDetected(String),
}

/// Validator for Directed Acyclic Graph constraints
pub struct DAGValidator;

impl DAGValidator {
    /// Validate that the given edges form a DAG (no cycles)
    ///
    /// # Arguments
    /// * `edges` - A slice of tuples representing directed edges (source, target)
    ///
    /// # Returns
    /// * `Ok(())` if the graph is acyclic
    /// * `Err(DAGError::CycleDetected)` if a cycle is found
    ///
    /// # Examples
    /// ```
    /// use adam_domain::dependency::dag::{DAGValidator, DAGError};
    ///
    /// // Valid DAG
    /// let edges = vec![("A", "B"), ("B", "C")];
    /// assert!(DAGValidator::validate_no_cycle(&edges).is_ok());
    ///
    /// // Cycle detected
    /// let cyclic = vec![("A", "B"), ("B", "C"), ("C", "A")];
    /// assert!(DAGValidator::validate_no_cycle(&cyclic).is_err());
    /// ```
    pub fn validate_no_cycle<T>(edges: &[(T, T)]) -> Result<(), DAGError>
    where
        T: Clone + Eq + Hash + std::fmt::Debug,
    {
        if edges.is_empty() {
            return Ok(());
        }

        let mut graph = DiGraph::<T, ()>::new();
        let mut node_indices: HashMap<T, NodeIndex> = HashMap::new();

        // Build the graph
        for (source, target) in edges {
            let source_idx = *node_indices
                .entry(source.clone())
                .or_insert_with(|| graph.add_node(source.clone()));
            let target_idx = *node_indices
                .entry(target.clone())
                .or_insert_with(|| graph.add_node(target.clone()));

            graph.add_edge(source_idx, target_idx, ());
        }

        // Check for cycles using petgraph
        if is_cyclic_directed(&graph) {
            // Find and report the cycle
            let cycle_description = Self::find_cycle_description(&graph);
            return Err(DAGError::CycleDetected(cycle_description));
        }

        Ok(())
    }

    /// Find a cycle in the graph for error reporting
    fn find_cycle_description<T>(graph: &DiGraph<T, ()>) -> String
    where
        T: std::fmt::Debug,
    {
        // Use DFS to find a cycle
        let mut visited = std::collections::HashSet::new();
        let mut recursion_stack = std::collections::HashSet::new();
        let mut path = Vec::new();

        for node in graph.node_indices() {
            if !visited.contains(&node) {
                if let Some(cycle) =
                    Self::dfs_find_cycle(graph, node, &mut visited, &mut recursion_stack, &mut path)
                {
                    return format!("{cycle:?}");
                }
            }
        }

        "unknown cycle".to_string()
    }

    /// DFS helper to find a cycle
    fn dfs_find_cycle<T>(
        graph: &DiGraph<T, ()>,
        node: NodeIndex,
        visited: &mut std::collections::HashSet<NodeIndex>,
        recursion_stack: &mut std::collections::HashSet<NodeIndex>,
        path: &mut Vec<NodeIndex>,
    ) -> Option<Vec<String>>
    where
        T: std::fmt::Debug,
    {
        visited.insert(node);
        recursion_stack.insert(node);
        path.push(node);

        for neighbor in graph.neighbors(node) {
            if !visited.contains(&neighbor) {
                if let Some(cycle) =
                    Self::dfs_find_cycle(graph, neighbor, visited, recursion_stack, path)
                {
                    return Some(cycle);
                }
            } else if recursion_stack.contains(&neighbor) {
                // Found a cycle - extract the cycle nodes from path
                let cycle_start = path.iter().position(|&n| n == neighbor).unwrap_or(0);
                let cycle_nodes: Vec<String> = path[cycle_start..]
                    .iter()
                    .map(|&idx| format!("{node:?}", node = &graph[idx]))
                    .collect();
                return Some(cycle_nodes);
            }
        }

        path.pop();
        recursion_stack.remove(&node);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_graph_is_valid() {
        let edges: Vec<(&str, &str)> = vec![];
        assert!(DAGValidator::validate_no_cycle(&edges).is_ok());
    }

    #[test]
    fn test_single_edge_is_valid() {
        let edges = vec![("A", "B")];
        assert!(DAGValidator::validate_no_cycle(&edges).is_ok());
    }

    #[test]
    fn test_simple_cycle_detected() {
        let edges = vec![("A", "B"), ("B", "C"), ("C", "A")];
        let result = DAGValidator::validate_no_cycle(&edges);
        assert!(result.is_err());
        if let Err(DAGError::CycleDetected(desc)) = result {
            assert!(!desc.is_empty());
        }
    }

    #[test]
    fn test_self_loop_detected() {
        let edges = vec![("A", "A")];
        let result = DAGValidator::validate_no_cycle(&edges);
        assert!(result.is_err());
    }

    #[test]
    fn test_valid_dag_passes() {
        let edges = vec![("A", "B"), ("A", "C"), ("B", "D")];
        assert!(DAGValidator::validate_no_cycle(&edges).is_ok());
    }

    #[test]
    fn test_diamond_shape_is_valid() {
        let edges = vec![("A", "B"), ("A", "C"), ("B", "D"), ("C", "D")];
        assert!(DAGValidator::validate_no_cycle(&edges).is_ok());
    }

    #[test]
    fn test_complex_cycle_detected() {
        let edges = vec![("A", "B"), ("B", "C"), ("C", "D"), ("D", "B")];
        let result = DAGValidator::validate_no_cycle(&edges);
        assert!(result.is_err());
    }

    #[test]
    fn test_with_uuids() {
        use uuid::Uuid;

        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();

        // Valid: 1 -> 2 -> 3
        let valid = vec![(id1, id2), (id2, id3)];
        assert!(DAGValidator::validate_no_cycle(&valid).is_ok());

        // Cycle: 1 -> 2 -> 3 -> 1
        let cyclic = vec![(id1, id2), (id2, id3), (id3, id1)];
        assert!(DAGValidator::validate_no_cycle(&cyclic).is_err());
    }
}
