use super::*;
use crate::domain::workflow::entities::workflow_execution::ExecutionTreeRestore;
use crate::domain::workflow::{NodeCompletionSignal, WorkflowDefinition};
use std::sync::Mutex;

struct Gateway {
    execution: Mutex<ExecutionTree>,
    calls: Mutex<Vec<&'static str>>,
    on_send: Mutex<Option<fn(&mut ExecutionTree)>>,
    conflicts: std::sync::atomic::AtomicUsize,
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
        if let Some(change) = self.on_send.lock().unwrap().take() {
            change(&mut self.execution.lock().unwrap());
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
        let mut execution = self.execution.lock().unwrap();
        if self
            .conflicts
            .fetch_update(
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
                |count| count.checked_sub(1),
            )
            .is_ok()
        {
            execution.record_node_display_command("node-1", "codex".into(), 8.0);
            return Err(WorkflowRuntimeError::Conflict(
                "changed before commit".into(),
            ));
        }
        assert_eq!(commit.before, *execution);
        assert!(matches!(
            commit.workflow_events.as_slice(),
            [WorkflowEvent::DelegateResultInjected { .. }]
        ));
        let snapshot = RuntimeCommitSnapshot::from_execution(&commit.after)?;
        *execution = commit.after;
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
            on_send: Mutex::new(None),
            conflicts: Default::default(),
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
        queue: crate::usecase::work_queue::WorkQueueUsecase::new(std::sync::Arc::new(
            crate::usecase::work_queue::ImmediateWorkQueueRuntime::default(),
        )),
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
            queue: crate::usecase::work_queue::WorkQueueUsecase::new(std::sync::Arc::new(
                crate::usecase::work_queue::ImmediateWorkQueueRuntime::default(),
            )),
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

fn continuation(gateway: std::sync::Arc<Gateway>) -> DelegateContinuationUsecase {
    DelegateContinuationUsecase {
        gateway,
        queue: crate::usecase::work_queue::WorkQueueUsecase::new(std::sync::Arc::new(
            crate::usecase::work_queue::ImmediateWorkQueueRuntime::default(),
        )),
    }
}

fn submit_again(execution: &mut ExecutionTree) {
    assert_eq!(
        execution.apply_submitted_output(
            "main".into(),
            "node-1",
            1,
            Some("agent".into()),
            "result".into(),
            serde_json::json!({}),
            None,
            10.0,
        ),
        TransitionOutcome::Applied,
    );
}

#[tokio::test]
async fn test_delegate_送付中の変更と末尾競合を保持し再送せず次の提出を受理する() {
    for during_send in [true, false] {
        // Given
        let (gateway, injection) = fixture(None);
        if during_send {
            *gateway.on_send.lock().unwrap() = Some(|execution| {
                execution.record_node_display_command("node-1", "codex".into(), 8.0);
            });
        } else {
            gateway
                .conflicts
                .store(2, std::sync::atomic::Ordering::SeqCst);
        }
        let usecase = continuation(gateway.clone());
        // When
        usecase.execute("tree", &injection).await.unwrap();
        // Then
        let mut execution = gateway.execution.lock().unwrap();
        assert_eq!(
            execution
                .node_execution("node-1")
                .unwrap()
                .display_command
                .as_deref(),
            Some("codex")
        );
        submit_again(&mut execution);
        let calls = gateway.calls.lock().unwrap();
        assert_eq!(calls.iter().filter(|call| **call == "send").count(), 1);
        assert_eq!(
            calls.iter().filter(|call| **call == "commit").count(),
            if during_send { 1 } else { 3 }
        );
    }
}

#[tokio::test]
async fn test_delegate_送付後に注入済みまたは対象変更または中止なら保存しない() {
    for change in [0, 1, 2] {
        // Given
        let (gateway, injection) = fixture(None);
        *gateway.on_send.lock().unwrap() = Some(match change {
            0 => |execution| {
                let injection = execution.pending_delegate_injection("node-1").unwrap();
                execution.record_delegate_injected(&injection, 8.0);
            },
            1 => |execution| {
                let injection = execution.pending_delegate_injection("node-1").unwrap();
                execution.record_delegate_injected(&injection, 8.0);
                submit_again(execution);
                execution.record_node_completion_signal(
                    "node-1",
                    NodeCompletionSignal::Submit,
                    10.0,
                );
                execution
                    .apply_node_completion_handshake("node-1", &mut || "node-3".into(), 10.0)
                    .unwrap();
                execution.record_node_completion_signal("node-1", NodeCompletionSignal::Stop, 10.0);
                execution.record_pending_result(
                    "node-3",
                    None,
                    Some(serde_json::json!({"ok": false})),
                    None,
                    None,
                    11.0,
                );
                execution
                    .complete_leaf_and_advance("node-3", &mut || "unused".into(), 11.0)
                    .unwrap();
                assert_eq!(
                    execution
                        .pending_delegate_injection("node-1")
                        .unwrap()
                        .child_execution_id,
                    "node-3"
                );
            },
            _ => |execution| {
                execution.abort_active_node_executions(8.0);
                execution.abort();
            },
        });
        let usecase = continuation(gateway.clone());
        // When
        assert!(usecase.execute("tree", &injection).await.unwrap().is_none());
        // Then
        assert_eq!(*gateway.calls.lock().unwrap(), ["restore", "send"]);
    }
}

#[tokio::test]
async fn test_delegate_競合中の失敗をnodeから回数と時刻付きで観測できる() {
    // Given
    let (gateway, injection) = fixture(None);
    gateway
        .conflicts
        .store(usize::MAX, std::sync::atomic::Ordering::SeqCst);
    let usecase = continuation(gateway.clone());
    // When
    let operation = usecase.execute("tree", &injection);
    tokio::pin!(operation);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            tokio::select! {
                result = &mut operation => panic!("unexpected completion: {result:?}"),
                _ = tokio::task::yield_now() => {}
            }
            let records = usecase.queue.records("node-1").await;
            if let Some(observation) = records.first().filter(|item| item.record.count >= 2) {
                // Then
                assert!(observation.record.active);
                assert!(!observation.requires_attention);
                assert_eq!(observation.record.operation, "workflow_delegate_injection");
                assert!(observation.record.last_observed_ms > observation.record.first_observed_ms);
                break;
            }
        }
    })
    .await
    .unwrap();
}
