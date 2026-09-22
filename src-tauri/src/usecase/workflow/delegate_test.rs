use super::*;
use crate::domain::workflow::entities::workflow_execution::ExecutionTreeRestore;
use crate::domain::workflow::{NodeCompletionSignal, WorkflowDefinition};
use std::sync::Mutex;

struct Gateway {
    execution: Mutex<ExecutionTree>,
    calls: Mutex<Vec<&'static str>>,
    fail: Option<&'static str>,
}

#[async_trait::async_trait]
impl DelegateContinuationGateway for Gateway {
    fn current_timestamp(&self) -> f64 {
        9.0
    }
    async fn load_execution(&self, _: &str) -> Result<ExecutionTree, WorkflowRuntimeError> {
        Ok(self.execution.lock().unwrap().clone())
    }
    async fn restore_provider(&self, session: &str, _: &str) -> Result<(), WorkflowRuntimeError> {
        assert_eq!(session, "agent");
        self.calls.lock().unwrap().push("restore");
        if self.fail == Some("restore") {
            return Err(WorkflowRuntimeError::AgentSession("restore failed".into()));
        }
        Ok(())
    }
    async fn send_instruction(
        &self,
        session: &str,
        child_execution_id: &str,
        instruction: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        assert_eq!(session, "agent");
        assert_eq!(child_execution_id, "node-2");
        let value: serde_json::Value =
            serde_json::from_str(instruction.split_once("\n\n").unwrap().1).unwrap();
        assert_eq!(value["child"]["ok"], false);
        self.calls.lock().unwrap().push("send");
        if self.fail == Some("send") {
            return Err(WorkflowRuntimeError::AgentSession("send failed".into()));
        }
        Ok(())
    }
    async fn commit(
        &self,
        commit: WorkflowControlPlaneCommit,
    ) -> Result<RuntimeCommitSnapshot, WorkflowRuntimeError> {
        self.calls.lock().unwrap().push("commit");
        if self.fail == Some("commit") {
            return Err(WorkflowRuntimeError::SessionStore("commit failed".into()));
        }
        assert!(matches!(
            commit.workflow_events.as_slice(),
            [WorkflowEvent::DelegateResultInjected { .. }]
        ));
        let snapshot = RuntimeCommitSnapshot::from_execution(&commit.after)?;
        *self.execution.lock().unwrap() = commit.after;
        Ok(snapshot)
    }
}

fn fixture(fail: Option<&'static str>) -> (std::sync::Arc<Gateway>, DelegateInjection) {
    let workflow: WorkflowDefinition = serde_saphyr::from_str("name: test\ndescription: test\nnodes:\n  main: {session: {provider: codex}, artifact: result, completion: {delegate: {child: check, when: child.ok, max_iterations: 2}}}\n  check: {command: check}").unwrap();
    let mut execution = ExecutionTree::restore_runtime(ExecutionTreeRestore {
        id: "tree".into(),
        workflow,
        ..Default::default()
    });
    let mut index = 0;
    let mut ids = || {
        index += 1;
        format!("node-{index}")
    };
    execution.start_root(&mut ids, 1.0).unwrap();
    let parent = execution.node_executions[0].id.clone();
    execution.attach_node_session(&parent, "agent".into(), 1.0);
    execution.record_node_completion_signal(&parent, NodeCompletionSignal::Submit, 2.0);
    execution.apply_submitted_output(
        "main".into(),
        &parent,
        1,
        Some("agent".into()),
        "result".into(),
        serde_json::json!({}),
        None,
        2.0,
    );
    execution
        .apply_node_completion_handshake(&parent, &mut ids, 2.0)
        .unwrap();
    execution.record_node_completion_signal(&parent, NodeCompletionSignal::Stop, 3.0);
    let child = execution.node_executions.last().unwrap().id.clone();
    execution.record_pending_result(
        &child,
        None,
        Some(serde_json::json!({"ok": false})),
        None,
        None,
        4.0,
    );
    execution
        .complete_leaf_and_advance(&child, &mut ids, 4.0)
        .unwrap();
    let injection = execution.pending_delegate_injection(&parent).unwrap();
    (
        std::sync::Arc::new(Gateway {
            execution: Mutex::new(execution),
            calls: Mutex::new(Vec::new()),
            fail,
        }),
        injection,
    )
}

#[tokio::test]
async fn test_delegate_復元と注入が成功した後に事実化し注入済みなら何もしない() {
    // Given
    let (gateway, injection) = fixture(None);
    let usecase = DelegateContinuationUsecase {
        gateway: gateway.clone(),
    };
    // When
    assert!(usecase.execute("tree", &injection).await.unwrap().is_some());
    assert!(usecase.execute("tree", &injection).await.unwrap().is_none());
    // Then
    assert_eq!(
        *gateway.calls.lock().unwrap(),
        ["restore", "send", "commit"]
    );
    assert!(gateway
        .execution
        .lock()
        .unwrap()
        .pending_delegate_injections()
        .is_empty());
}

#[tokio::test]
async fn test_delegate_復元と注入と保存の失敗を呼び出し元へ返す() {
    for (fail, expected) in [
        ("restore", vec!["restore"]),
        ("send", vec!["restore", "send"]),
        ("commit", vec!["restore", "send", "commit"]),
    ] {
        // Given
        let (gateway, injection) = fixture(Some(fail));
        // When
        let result = (DelegateContinuationUsecase {
            gateway: gateway.clone(),
        })
        .execute("tree", &injection)
        .await;
        // Then
        assert!(result.is_err());
        assert_eq!(*gateway.calls.lock().unwrap(), expected);
        assert_eq!(
            gateway
                .execution
                .lock()
                .unwrap()
                .pending_delegate_injections(),
            [injection]
        );
    }
}
