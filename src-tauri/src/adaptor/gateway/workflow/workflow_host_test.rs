use super::*;

#[test]
fn command_env_includes_worktree_path() {
    let input = CommandExecutionInput {
        execution_id: "execution-1".to_string(),
        node_execution_id: "node-execution-1".to_string(),
        node_name: "check".to_string(),
        attempt: 1,
        worktree_path: "/repo/worktree".to_string(),
        raw_command: Some("true".to_string()),
        definition_env: Vec::new(),
        contract: None,
        schemas: BTreeMap::new(),
        session_id: None,
    };

    let env = command_env(
        &input,
        vec![
            ("DOC".to_string(), "document".to_string()),
            (
                "RELEASH_WORKTREE_PATH".to_string(),
                "/definition/attempted-override".to_string(),
            ),
        ],
    );

    assert!(env.contains(&("DOC".to_string(), "document".to_string())));
    assert!(env.contains(&(
        "RELEASH_WORKTREE_PATH".to_string(),
        "/repo/worktree".to_string()
    )));
    assert_eq!(
        env.iter()
            .rev()
            .find(|(name, _)| name == "RELEASH_WORKTREE_PATH")
            .map(|(_, value)| value.as_str()),
        Some("/repo/worktree")
    );
}

mod restored_memory_cases {
    use super::super::*;
    use crate::adaptor::gateway::workflow::test_helpers::*;
    use std::sync::Arc;

    #[tokio::test]
    pub async fn test_runtime再試行_競合だけを上限まで再実行する() {
        use std::sync::atomic::AtomicUsize;
        use std::sync::atomic::Ordering;
        for (conflicts, storage_error) in [(0, false), (1, false), (4, false), (0, true)] {
            // Given
            let calls = AtomicUsize::new(0);
            // When
            let result =
                retry_runtime_conflicts(crate::test_support::retry::shared(), "test", || async {
                    let attempt = calls.fetch_add(1, Ordering::SeqCst);
                    if storage_error {
                        Err(WorkflowRuntimeError::SessionStore("unavailable".into()))
                    } else if attempt < conflicts {
                        Err(WorkflowRuntimeError::Conflict("advanced".into()))
                    } else {
                        Ok(())
                    }
                })
                .await;
            // Then
            assert_eq!(calls.load(Ordering::SeqCst), (conflicts + 1));
            assert_eq!(result.is_ok(), !storage_error);
        }
    }

    #[tokio::test]
    pub async fn test_committed_runtime_effects_停止失敗後も残りのagent_sessionを停止する() {
        let stop_calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sessions: Arc<dyn WorkflowAgentSessionPort> =
            Arc::new(RecordingWorkflowAgentSessions {
                stop_calls: stop_calls.clone(),
                prepare_calls: Arc::new(std::sync::Mutex::new(Vec::new())),
                provider_running_checks: Arc::new(std::sync::Mutex::new(Vec::new())),
                recovery_fails: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                failing_agent_session_id: "agent-session-1".to_string(),
            });

        WorkflowRuntimeHost::run_committed_runtime_effects(
            sessions,
            vec![
                WorkflowRuntimeEffect::BroadcastState,
                WorkflowRuntimeEffect::StopWorkflowAgentSession {
                    node_execution_id: "node-1".to_string(),
                    agent_session_id: "agent-session-1".to_string(),
                },
                WorkflowRuntimeEffect::StopWorkflowAgentSession {
                    node_execution_id: "node-2".to_string(),
                    agent_session_id: "agent-session-2".to_string(),
                },
            ],
        )
        .await;

        assert_eq!(
            stop_calls.lock().unwrap().as_slice(),
            &[
                ("node-1".to_string(), "agent-session-1".to_string()),
                ("node-2".to_string(), "agent-session-2".to_string()),
            ]
        );
    }
}
