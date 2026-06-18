//! Structured output submission rules.

use crate::domain::workflow::value_objects::{NodeType, WorkflowDefinition};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionParallelChildState {
    Running,
    Completed,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, Copy)]
pub struct SubmissionParallelChild<'a> {
    pub step_name: &'a str,
    pub state: SubmissionParallelChildState,
}

#[derive(Debug, Clone, Copy)]
pub struct SubmissionParallelRun<'a> {
    pub parent_step_name: &'a str,
    pub children: &'a [SubmissionParallelChild<'a>],
}

pub fn is_accepting_submission_target(
    workflow: &WorkflowDefinition,
    current_step_index: usize,
    parallel_run: Option<SubmissionParallelRun<'_>>,
    step_name: &str,
) -> bool {
    let Some(current) = workflow.nodes.get(current_step_index) else {
        return false;
    };
    if current.name == step_name && current.node_type != NodeType::Parallel {
        return true;
    }
    if let Some(parallel_run) = parallel_run {
        if parallel_run.parent_step_name == current.name {
            return parallel_run.children.iter().any(|child| {
                child.step_name == step_name && child.state == SubmissionParallelChildState::Running
            });
        }
    }
    false
}

pub fn step_output_keys_to_clear_for_new_execution(
    workflow: &WorkflowDefinition,
    step_index: usize,
) -> Vec<String> {
    let Some(step) = workflow.nodes.get(step_index) else {
        return Vec::new();
    };
    let mut keys = vec![step.name.clone()];
    if let Some(children) = step.parallel_children.as_ref() {
        keys.extend(children.iter().map(|child| child.name.clone()));
    }
    keys
}

#[cfg(test)]
mod submission_tests {
    use super::*;
    use crate::domain::workflow::value_objects::{NodeDefinition, WorkflowDefinition};

    fn node(name: &str, node_type: NodeType) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            node_type,
            ..Default::default()
        }
    }

    fn workflow(nodes: Vec<NodeDefinition>) -> WorkflowDefinition {
        WorkflowDefinition {
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            variables: Default::default(),
            nodes,
        }
    }

    #[test]
    fn accepts_current_non_parallel_node() {
        let workflow = workflow(vec![
            node("draft", NodeType::Agent),
            node("review", NodeType::Approval),
        ]);

        assert!(is_accepting_submission_target(&workflow, 0, None, "draft"));
        assert!(!is_accepting_submission_target(
            &workflow, 0, None, "review"
        ));
    }

    #[test]
    fn accepts_running_child_of_current_parallel_parent_only() {
        let workflow = workflow(vec![node("parallel-review", NodeType::Parallel)]);
        let children = [
            SubmissionParallelChild {
                step_name: "quality",
                state: SubmissionParallelChildState::Running,
            },
            SubmissionParallelChild {
                step_name: "security",
                state: SubmissionParallelChildState::Completed,
            },
        ];
        let parallel_run = SubmissionParallelRun {
            parent_step_name: "parallel-review",
            children: &children,
        };

        assert!(is_accepting_submission_target(
            &workflow,
            0,
            Some(parallel_run),
            "quality",
        ));
        assert!(!is_accepting_submission_target(
            &workflow,
            0,
            Some(parallel_run),
            "security",
        ));
        assert!(!is_accepting_submission_target(
            &workflow,
            0,
            Some(parallel_run),
            "parallel-review",
        ));
    }

    #[test]
    fn rejects_out_of_range_current_step() {
        let workflow = workflow(vec![node("draft", NodeType::Agent)]);

        assert!(!is_accepting_submission_target(&workflow, 1, None, "draft"));
    }

    #[test]
    fn output_keys_to_clear_include_parallel_parent_and_children() {
        let mut parallel = node("parallel-review", NodeType::Parallel);
        parallel.parallel_children = Some(vec![
            crate::domain::workflow::value_objects::ChildNodeDefinition {
                name: "quality".to_string(),
                ..Default::default()
            },
            crate::domain::workflow::value_objects::ChildNodeDefinition {
                name: "security".to_string(),
                ..Default::default()
            },
        ]);
        let workflow = workflow(vec![node("draft", NodeType::Agent), parallel]);

        assert_eq!(
            step_output_keys_to_clear_for_new_execution(&workflow, 1),
            vec!["parallel-review", "quality", "security"]
        );
        assert_eq!(
            step_output_keys_to_clear_for_new_execution(&workflow, 0),
            vec!["draft"]
        );
        assert!(step_output_keys_to_clear_for_new_execution(&workflow, 99).is_empty());
    }
}
