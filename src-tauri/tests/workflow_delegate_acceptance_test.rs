#![cfg(all(debug_assertions, feature = "desktop"))]

use releash_lib::workflow_delegate_acceptance::{
    AcceptanceIngressResult, AcceptanceProvider, NodeExecutionStatus,
    WorkflowDelegateAcceptanceHost,
};

#[tokio::test]
async fn test_delegate配下のレビュー_cwdと開始終了通知を解決し全提出後のstopで裁定を開始する() {
    for isolated_parent in [false, true] {
        // Given
        let directory = tempfile::tempdir().unwrap();
        let host = WorkflowDelegateAcceptanceHost::new(directory.path());
        let workspace = "/repo-worktrees/development";
        let nodes = r#"
  main: {sequence: {children: [impl]}}
  impl: {sequence: {children: [implement]}}
  implement:
    session: {provider: codex, facets: {instruction: policy-confirmation}}
    artifact: result
    completion:
      delegate: {child: inner_check, when: child.inner_adjudicate.passed, max_iterations: 2}
  inner_check: {sequence: {children: [inner_review, inner_adjudicate]}}
  inner_review: {fanout: {items: [1, 2, 3], children: [reviewer_claude, reviewer_codex]}}
  reviewer_claude:
    session: {provider: claude, facets: {instruction: policy-confirmation}}
    artifact: result
  reviewer_codex:
    session: {provider: codex, facets: {instruction: policy-confirmation}}
    artifact: result
  inner_adjudicate:
    session: {provider: codex, facets: {instruction: policy-confirmation}}
    artifact: result
schemas:
  result: {type: object, properties: {passed: {type: boolean}}, required: [passed]}
"#;
        let nodes = if isolated_parent {
            nodes.replace("  implement:\n", "  implement:\n    worktree: isolated\n")
        } else {
            nodes.to_string()
        };
        let tree = host.start(&nodes, workspace).await;
        let parent = host
            .nodes(&tree)
            .into_iter()
            .find(|node| node.node_name == "implement")
            .unwrap();
        host.submit(&parent.id, serde_json::json!({"passed": false}))
            .await;
        host.stop_parent(&tree, &parent).await;
        let reviewers: Vec<_> = host
            .nodes(&tree)
            .into_iter()
            .filter(|node| node.node_name.starts_with("reviewer_"))
            .collect();
        assert_eq!(reviewers.len(), 6);

        let mut launches = Vec::new();
        for reviewer in &reviewers {
            let session_id = reviewer.session_id.as_deref().unwrap();
            let provider = if reviewer.node_name == "reviewer_claude" {
                AcceptanceProvider::Claude
            } else {
                AcceptanceProvider::Codex
            };
            let launch = host.arm(session_id, provider).await;
            let provider_session_id = format!("provider-{session_id}");

            // When
            assert_eq!(
                host.session_started(&launch, &provider_session_id).await,
                AcceptanceIngressResult::Applied
            );

            // Then
            let (worktree_path, associated_provider_session) = host.session_context(session_id);
            assert_eq!(
                associated_provider_session.as_deref(),
                Some(provider_session_id.as_str())
            );
            assert_eq!(worktree_path, host.launched_cwd(&reviewer.id));
            assert_eq!(
                worktree_path,
                parent
                    .worktree
                    .as_ref()
                    .map_or(workspace, |worktree| worktree.path.as_str())
            );
            host.submit(&reviewer.id, serde_json::json!({"passed": true}))
                .await;
            launches.push((launch, provider_session_id));
        }

        for (reviewer, (launch, provider_session_id)) in reviewers.iter().zip(launches) {
            // Given
            let before = host.nodes(&tree);
            assert!(!before
                .iter()
                .any(|node| node.node_name == "inner_adjudicate"));
            assert_eq!(
                before
                    .iter()
                    .find(|node| node.id == reviewer.id)
                    .unwrap()
                    .status,
                NodeExecutionStatus::Running
            );

            // When
            assert_eq!(
                host.stop_observed(&launch, &provider_session_id).await,
                AcceptanceIngressResult::Applied
            );

            // Then
            let (provider_sessions, stops) = host.session_facts(&tree, &reviewer.id);
            assert!(provider_sessions.contains(&provider_session_id));
            assert_eq!(stops, 1);
            assert_eq!(
                host.nodes(&tree)
                    .iter()
                    .find(|node| node.id == reviewer.id)
                    .unwrap()
                    .status,
                NodeExecutionStatus::Succeeded
            );
        }

        let adjudicate = host
            .nodes(&tree)
            .into_iter()
            .find(|node| node.node_name == "inner_adjudicate")
            .unwrap();
        assert_eq!(adjudicate.status, NodeExecutionStatus::Running);
        assert!(adjudicate.session_id.is_some());
        assert!(host.is_activated(&adjudicate.id));
    }
}
