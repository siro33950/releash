mod command_preparation_tests {
    use super::super::*;
    use crate::domain::workflow::entities::workflow_execution::{
        ExecutionTree, ExecutionTreeRestore,
    };
    use crate::domain::workflow::{NodeDefinition, NodeKindName, WorkflowDefinition};

    fn input_for(node_execution_id: &str) -> CommandExecutionInput {
        CommandExecutionInput {
            execution_id: "execution-1".to_string(),
            node_execution_id: node_execution_id.to_string(),
            node_name: "check".to_string(),
            attempt: 1,
            worktree_path: "/repo".to_string(),
            raw_command: Some("true".to_string()),
            definition_env: Vec::new(),
            contract: None,
            schemas: Default::default(),
            session_id: None,
        }
    }

    fn execution_with_running_command() -> (ExecutionTree, String) {
        let mut execution = ExecutionTree::restore_runtime(ExecutionTreeRestore {
            id: "execution-1".to_string(),
            workflow: WorkflowDefinition {
                name: "wf".to_string(),
                entry: "check".to_string(),
                nodes: vec![NodeDefinition {
                    name: "check".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..ExecutionTreeRestore::default()
        });
        let node_execution_id = execution
            .begin_node_attempt(
                "check".to_string(),
                NodeKindName::Command,
                1,
                None,
                "command-1".to_string(),
                1.0,
            )
            .unwrap();
        (execution, node_execution_id)
    }

    #[test]
    fn command_input_is_current_only_while_the_node_execution_is_running() {
        let (mut execution, node_execution_id) = execution_with_running_command();
        let input = input_for(&node_execution_id);
        assert!(command_execution_input_is_current(&execution, &input));

        // Abort 後の command は起動しない。
        assert_eq!(
            execution.abort_node_execution(&node_execution_id, 2.0),
            crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
        );
        assert!(!command_execution_input_is_current(&execution, &input));
    }
}
