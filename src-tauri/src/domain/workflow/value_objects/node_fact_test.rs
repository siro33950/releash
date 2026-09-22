use super::*;

#[test]
fn test_agent_session活動導出_stopとprocess_exitで次の指示待ちへ戻る() {
    let stop = NodeFact::StopReceived(StopReceivedFact {
        result_summary: None,
        token_usage: None,
    });
    let process_exit = NodeFact::ProcessExited(ProcessExitedFact {
        failure_kind: None,
        exit_code: Some(0),
        result_summary: None,
        failure_reason: None,
    });

    for fact in [stop, process_exit] {
        assert_eq!(
            AgentSessionActivity::Working.after_fact(&fact),
            AgentSessionActivity::AwaitingInstruction
        );
    }
}

#[test]
fn test_process_exited_終了結果の全項目から異常終了を判定する() {
    let process_exited = |exit_code, failure_reason, failure_kind| ProcessExitedFact {
        exit_code,
        result_summary: None,
        failure_reason,
        failure_kind,
    };

    assert!(!process_exited(Some(0), None, None).is_abnormal());
    assert!(process_exited(Some(1), None, None).is_abnormal());
    assert!(process_exited(None, None, None).is_abnormal());
    assert!(process_exited(Some(0), Some("failed".to_string()), None).is_abnormal());
    assert!(process_exited(
        Some(0),
        None,
        Some(NodeExecutionFailureKind::InfrastructureCrash),
    )
    .is_abnormal());
}

mod session_execution_tree_root_facts_tests {
    use super::*;

    #[test]
    fn test_session実行木root構築_session_node一個のcanonical事実を返す() {
        let session_id = "session-1";

        let facts = SessionExecutionTreeRootFacts::new(
            session_id,
            "workspace-1",
            "/repo",
            ProviderKind::Claude,
            Some("/main-repo".into()),
        )
        .unwrap();

        assert_eq!(facts.meta.tree_id, session_id);
        assert_eq!(facts.meta.node_execution_id, session_id);
        assert_eq!(facts.meta.parent_id, None);
        assert_eq!(facts.meta.node_name, "session");
        assert_eq!(facts.meta.kind, NodeKindName::Session);
        assert_eq!(facts.meta.attempt, 1);
        let NodeFact::Started(StartedFact {
            worktree: None,
            parent: None,
            root: Some(root),
        }) = &facts.started
        else {
            panic!("started root fact expected");
        };
        assert_eq!(root.launched_as, ExecutionTreeLaunch::Session);
        assert_eq!(root.repository_root.as_deref(), Some("/main-repo"));
        assert_eq!(root.definition.as_ref().unwrap().nodes.len(), 1);
        assert_eq!(root.definition.as_ref().unwrap().entry, "session");
        assert_eq!(root.definition.as_ref().unwrap().nodes[0].name, "session");
        assert_eq!(
            root.definition.as_ref().unwrap().nodes[0].completion,
            NodeCompletion::default()
        );
        assert!(matches!(
            root.definition.as_ref().unwrap().nodes[0].kind,
            NodeKind::Session(SessionSpec {
                provider: ProviderKind::Claude,
                ..
            })
        ));
        assert_eq!(
            facts.attached,
            NodeFact::SessionAttached(SessionAttachedFact {
                session_id: session_id.to_string(),
                provider_session_id: None,
                transcript_ref: None,
                initial_instruction_admitted: false,
            })
        );
    }

    #[test]
    fn test_session実行木root構築_空入力と空白だけの入力を各項目で拒否する() {
        // When / Then
        assert_eq!(
            SessionExecutionTreeRootFacts::new(
                "",
                "workspace-1",
                "/repo",
                ProviderKind::Claude,
                None
            )
            .unwrap_err(),
            SessionExecutionTreeRootFactsError::SessionId
        );
        assert_eq!(
            SessionExecutionTreeRootFacts::new(
                " \t",
                "workspace-1",
                "/repo",
                ProviderKind::Claude,
                None
            )
            .unwrap_err(),
            SessionExecutionTreeRootFactsError::SessionId
        );
        assert_eq!(
            SessionExecutionTreeRootFacts::new(
                "session-1",
                "",
                "/repo",
                ProviderKind::Claude,
                None
            )
            .unwrap_err(),
            SessionExecutionTreeRootFactsError::WorkspaceIdentity
        );
        assert_eq!(
            SessionExecutionTreeRootFacts::new(
                "session-1",
                " \t",
                "/repo",
                ProviderKind::Claude,
                None
            )
            .unwrap_err(),
            SessionExecutionTreeRootFactsError::WorkspaceIdentity
        );
        assert_eq!(
            SessionExecutionTreeRootFacts::new(
                "session-1",
                "workspace-1",
                "",
                ProviderKind::Claude,
                None,
            )
            .unwrap_err(),
            SessionExecutionTreeRootFactsError::WorktreePath
        );
        assert_eq!(
            SessionExecutionTreeRootFacts::new(
                "session-1",
                "workspace-1",
                " \t",
                ProviderKind::Claude,
                None,
            )
            .unwrap_err(),
            SessionExecutionTreeRootFactsError::WorktreePath
        );
    }
}
