//! Agent-session activation procedure for workflow nodes.

use std::collections::{BTreeMap, HashMap};
use tokio::sync::Mutex;

use crate::adaptor::gateway::workflow::workflow_host::execution_registry::find_by_worktree;
use crate::adaptor::gateway::workflow::workflow_host::execution_state::DomainWorkflowExecution;
use crate::adaptor::gateway::workflow::workflow_host::fanout_runtime::{
    self as workflow_fanout_runtime, FanoutPromptInputs, FanoutStartContext,
};
use crate::adaptor::gateway::workflow::workflow_host::prompt_rendering as workflow_prompt;
use crate::domain::workflow::SchemaDef;
use crate::domain::workflow::WorkflowFacetContents;
use crate::usecase::workflow::runtime_error::WorkflowRuntimeError;
use crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot;

pub(crate) struct FanoutStartRuntimeInputs {
    pub(crate) fanout_start: FanoutStartContext,
    pub(crate) prompt_inputs: FanoutPromptInputs,
}

pub(crate) async fn load_fanout_start_runtime_inputs(
    executions: &Mutex<HashMap<String, DomainWorkflowExecution>>,
    worktree_path: &str,
) -> Result<FanoutStartRuntimeInputs, WorkflowRuntimeError> {
    let execs = executions.lock().await;
    let (_, exec) = find_by_worktree(&execs, worktree_path)
        .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(worktree_path.to_string()))?;
    Ok(FanoutStartRuntimeInputs {
        fanout_start: workflow_fanout_runtime::prepare_fanout_start_context(exec)?,
        prompt_inputs: workflow_fanout_runtime::fanout_prompt_inputs(exec),
    })
}

/// ワークフロー状態をブロードキャストする。
/// スナップショットは呼び出し元がロック内で確定したものを受け取る。
pub(crate) async fn broadcast_state<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    worktree_path: &str,
    commit_snapshot: RuntimeCommitSnapshot,
) {
    crate::adaptor::gateway::workflow::emit_workflow_execution_from_snapshot(
        app,
        worktree_path,
        crate::usecase::workflow::runtime_snapshot::runtime_commit_snapshot_to_domain_snapshot(
            commit_snapshot,
        ),
    )
    .await;
}

pub(crate) struct FanoutChildSessionPlan {
    pub(crate) node_execution_id: String,
    pub(crate) launch_config:
        crate::adaptor::gateway::workflow::node_session_boundary::WorkflowSessionLaunchConfig,
    pub(crate) initial_instruction: String,
}

pub(crate) fn prepare_fanout_child_session_plans(
    fanout_start: &FanoutStartContext,
    prompt_inputs: &FanoutPromptInputs,
    facet_contents: &WorkflowFacetContents,
    schemas: &BTreeMap<String, SchemaDef>,
) -> Result<Vec<FanoutChildSessionPlan>, WorkflowRuntimeError> {
    let prompt_plans =
        prepare_fanout_child_prompt_plans(fanout_start, prompt_inputs, facet_contents, schemas)?;
    prompt_plans
        .into_iter()
        .map(|prompt_plan| {
            let child = &fanout_start.children[prompt_plan.expansion_index];
            let launch_config = child
                .node
                .session()
                .map(
                    crate::adaptor::gateway::workflow::node_session_boundary::
                        WorkflowSessionLaunchConfig::from_session_spec,
                )
                .ok_or_else(|| {
                    WorkflowRuntimeError::InvalidState(format!(
                        "fanout child '{}' is not a Session Node",
                        child.node.name
                    ))
                })?;
            Ok(FanoutChildSessionPlan {
                node_execution_id: child.node_execution_id.clone(),
                launch_config,
                initial_instruction: crate::domain::workflow::services::prompt_composition::provider_tui_initial_instruction(
                    prompt_plan.system_prompt.as_deref(),
                    &prompt_plan.user_message,
                ),
            })
        })
        .collect()
}

struct FanoutChildPromptPlan {
    expansion_index: usize,
    system_prompt: Option<String>,
    user_message: String,
}

fn prepare_fanout_child_prompt_plans(
    fanout_start: &FanoutStartContext,
    prompt_inputs: &FanoutPromptInputs,
    facet_contents: &WorkflowFacetContents,
    schemas: &BTreeMap<String, SchemaDef>,
) -> Result<Vec<FanoutChildPromptPlan>, WorkflowRuntimeError> {
    fanout_start
        .children
        .iter()
        .enumerate()
        .filter(|(_, child)| child.reused.is_none() && child.node.session().is_some())
        .map(|(expansion_index, child)| {
            let (system_prompt, user_message) = workflow_prompt::build_fanout_child_prompt(
                &child.node,
                facet_contents.for_node(&child.node.name),
                fanout_start.request.as_deref(),
                &prompt_inputs.artifacts,
                workflow_prompt::FanoutChildPromptContext::new(
                    child.item.as_ref(),
                    &child.node_execution_id,
                ),
                schemas,
            )?;
            Ok(FanoutChildPromptPlan {
                expansion_index,
                system_prompt,
                user_message,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::workflow_host::node_settings::WorkflowDefaults;
    use crate::domain::workflow::{FanoutSpec, NodeDefinition, NodeKind, WorkflowDefinition};
    use crate::domain::workflow::{RuntimeArtifact, RuntimeExecutionState, TokenUsage};

    fn workflow_execution_fixture(
        execution_id: &str,
        worktree_path: &str,
    ) -> crate::adaptor::gateway::workflow::workflow_host::execution_state::DomainWorkflowExecution
    {
        let node_name = "plan".to_string();
        crate::adaptor::gateway::workflow::workflow_host::execution_state::domain_workflow_execution! {
            id: execution_id.to_string(),
            workflow: WorkflowDefinition {
                name: "test-workflow".to_string(),
                description: String::new(),
                builtin: false,
                schemas: Default::default(),
                nodes: vec![NodeDefinition {
                    name: node_name.clone(),
                    ..Default::default()
                }],
                entry: node_name.clone(),
            },
            lifecycle: DomainWorkflowExecution::lifecycle_from_state(RuntimeExecutionState::Running),
            current_node_index: 0,
            node_execution_counts: HashMap::from([(node_name, 1)]),
            loop_guard_reset_baselines: Default::default(),
            node_history: Vec::new(),
            workflow_defaults: WorkflowDefaults,
            worktree_path: worktree_path.to_string(),
            created_from: crate::domain::workflow::ExecutionOrigin::Cli,
            error_reason: None,
            started_at: 1.0,
            updated_at: 1.0,
            current_session_id: Some("session-1".to_string()),
            current_node_token_usage: TokenUsage::default(),
            artifacts: HashMap::new(),
            node_executions: Vec::new(),
            request: None,
            fanout_runtime: None,
            current_stall_observations: Vec::new(),
        }
    }

    #[tokio::test]
    async fn load_fanout_start_runtime_inputs_reads_context_and_prompt_inputs() {
        let mut exec = workflow_execution_fixture("execution-1", "/tmp/repo");
        exec.workflow.nodes[0] = NodeDefinition {
            name: "fanout-review".to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                child: vec!["review-a".to_string()],
                items: None,
            }),
            ..Default::default()
        };
        exec.workflow.nodes.push(NodeDefinition {
            name: "review-a".to_string(),
            ..Default::default()
        });
        exec.artifacts.insert(
            "plan".to_string(),
            RuntimeArtifact {
                node_name: "plan".to_string(),
                attempt: 1,
                session_id: Some("plan-session".to_string()),
                result: Some("DONE".to_string()),
                artifact: Some(serde_json::json!({ "status": "ok" })),
                contract: None,
                token_usage: None,
                completed_at: 2.0,
            },
        );
        let executions = Mutex::new(HashMap::from([("execution-1".to_string(), exec)]));

        let inputs = load_fanout_start_runtime_inputs(&executions, "/tmp/repo")
            .await
            .unwrap();

        assert_eq!(inputs.fanout_start.parent_node_name, "fanout-review");
        assert_eq!(
            inputs.fanout_start.child_node_names(),
            vec!["review-a".to_string()]
        );
        assert_eq!(
            inputs.prompt_inputs.artifacts["plan"].artifact,
            Some(serde_json::json!({ "status": "ok" }))
        );
    }

    #[test]
    fn fanout_resume_plans_prompts_and_session_creation_only_for_unconfirmed_children() {
        let mut exec = workflow_execution_fixture("execution-1", "/tmp/repo");
        exec.current_session_id = None;
        exec.workflow.nodes = vec![
            NodeDefinition {
                name: "fanout-review".to_string(),
                kind: NodeKind::Fanout(FanoutSpec {
                    child: vec!["review-reused".to_string(), "review-pending".to_string()],
                    items: None,
                }),
                ..Default::default()
            },
            NodeDefinition {
                name: "review-reused".to_string(),
                kind: NodeKind::Session(crate::domain::workflow::SessionSpec::default()),
                artifact: Some("review".to_string()),
                ..Default::default()
            },
            NodeDefinition {
                name: "review-pending".to_string(),
                kind: NodeKind::Session(crate::domain::workflow::SessionSpec::default()),
                artifact: Some("review".to_string()),
                ..Default::default()
            },
        ];

        let prompt_inputs = workflow_fanout_runtime::fanout_prompt_inputs(&exec);
        let mut fanout_start =
            workflow_fanout_runtime::prepare_fanout_start_context(&exec).unwrap();
        let reused_node_execution_id = fanout_start.children[0].node_execution_id.clone();
        let pending_node_execution_id = fanout_start.children[1].node_execution_id.clone();
        fanout_start.children[0].reused = Some(workflow_fanout_runtime::ReusableFanoutChild {
            result: Some("already confirmed".to_string()),
            display_command: None,
            artifact: Some(serde_json::json!({ "verdict": "pass" })),
            contract: Some("review".to_string()),
            token_usage: Some(TokenUsage {
                input_tokens: 3,
                output_tokens: 4,
            }),
            completed_at: 2.0,
        });

        let prompt_plans = prepare_fanout_child_prompt_plans(
            &fanout_start,
            &prompt_inputs,
            &WorkflowFacetContents::default(),
            &exec.workflow.schemas,
        )
        .unwrap();

        assert_eq!(prompt_plans.len(), 1);
        assert_eq!(prompt_plans[0].expansion_index, 1);
        assert!(prompt_plans[0]
            .user_message
            .contains(&pending_node_execution_id));
        assert!(!prompt_plans[0]
            .user_message
            .contains(&reused_node_execution_id));

        let session_plans = prepare_fanout_child_session_plans(
            &fanout_start,
            &prompt_inputs,
            &WorkflowFacetContents::default(),
            &exec.workflow.schemas,
        )
        .unwrap();

        assert_eq!(session_plans.len(), 1);
        assert_eq!(
            session_plans[0].node_execution_id,
            pending_node_execution_id
        );
    }
}
