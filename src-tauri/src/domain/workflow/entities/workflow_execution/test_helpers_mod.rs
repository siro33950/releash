use super::*;
#[cfg(test)]
pub fn execution(yaml: &str) -> ExecutionTree {
    ExecutionTree::restore_runtime(ExecutionTreeRestore {
        id: "execution".to_string(),
        workflow: serde_saphyr::from_str(yaml).unwrap(),
        ..Default::default()
    })
}
pub fn id_source() -> impl FnMut() -> String {
    let mut counter = 0;
    move || {
        counter += 1;
        format!("node-{counter}")
    }
}
pub fn started_names(events: &[WorkflowEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            WorkflowEvent::NodeStarted { node_name, .. } => Some(node_name.clone()),
            _ => None,
        })
        .collect()
}
pub fn execution_id_of(execution: &ExecutionTree, node_name: &str) -> String {
    execution
        .node_executions()
        .iter()
        .find(|node| node.node_name == node_name)
        .unwrap_or_else(|| panic!("node execution '{node_name}' must exist"))
        .id
        .clone()
}
