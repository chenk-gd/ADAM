use adam_domain::dependency::dag::{DAGError, DAGValidator};

#[test]
fn detects_simple_cycle() {
    // A -> B, B -> C, C -> A (cycle)
    let edges = vec![("A", "B"), ("B", "C"), ("C", "A")];
    let result = DAGValidator::validate_no_cycle(&edges);
    assert!(matches!(result, Err(DAGError::CycleDetected(_))));
}

#[test]
fn valid_dag_passes() {
    // A -> B, A -> C, B -> D (no cycle)
    let edges = vec![("A", "B"), ("A", "C"), ("B", "D")];
    assert!(DAGValidator::validate_no_cycle(&edges).is_ok());
}

#[test]
fn empty_graph_passes() {
    let edges: Vec<(&str, &str)> = vec![];
    assert!(DAGValidator::validate_no_cycle(&edges).is_ok());
}

#[test]
fn single_edge_passes() {
    let edges = vec![("A", "B")];
    assert!(DAGValidator::validate_no_cycle(&edges).is_ok());
}

#[test]
fn self_loop_detected() {
    // A -> A (self-loop is a cycle)
    let edges = vec![("A", "A")];
    let result = DAGValidator::validate_no_cycle(&edges);
    assert!(matches!(result, Err(DAGError::CycleDetected(_))));
}

#[test]
fn complex_dag_passes() {
    // Complex DAG with multiple paths but no cycles
    // A -> B, A -> C, B -> D, C -> D, D -> E
    let edges = vec![("A", "B"), ("A", "C"), ("B", "D"), ("C", "D"), ("D", "E")];
    assert!(DAGValidator::validate_no_cycle(&edges).is_ok());
}

#[test]
fn complex_cycle_detected() {
    // A -> B -> C -> D -> B (cycle: B -> C -> D -> B)
    let edges = vec![("A", "B"), ("B", "C"), ("C", "D"), ("D", "B")];
    let result = DAGValidator::validate_no_cycle(&edges);
    assert!(matches!(result, Err(DAGError::CycleDetected(_))));
}

#[test]
fn diamond_shape_passes() {
    // Diamond: A -> B, A -> C, B -> D, C -> D
    let edges = vec![("A", "B"), ("A", "C"), ("B", "D"), ("C", "D")];
    assert!(DAGValidator::validate_no_cycle(&edges).is_ok());
}
