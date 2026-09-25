use crate::common::retry::RetryBackoff;
use crate::domain::failure::FailureKind;
use crate::domain::workflow::NodeKindName;
use crate::domain::workspace_tree::{
    WorkspaceNodeStatusClassification, WorkspaceStructureFact, WorkspaceTree,
    WorkspaceTreeProjector,
};
use crate::usecase::work_queue::{work_queue_tests::queue, WorkFailure, WorkKey};

fn tree() -> WorkspaceTree {
    let execution_id = "00000000-0000-4000-8000-000000000743";
    let mut tree = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(
        &mut tree,
        [
            WorkspaceStructureFact::WorkflowStarted {
                execution_id: execution_id.into(),
                workflow_name: "review".into(),
                worktree_path: "/repo".into(),
                dynamic_fanout_names: Default::default(),
                timestamp: 1.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.into(),
                node_execution_id: "command-execution".into(),
                node_name: "command".into(),
                kind: NodeKindName::Command,
                attempt: 1,
                parent: None,
                timestamp: 2.0,
            },
        ],
    )
    .unwrap();
    let nodes = tree
        .nodes()
        .iter()
        .cloned()
        .map(|mut node| {
            if node.id == execution_id {
                node.id = "display-root".into();
            }
            if node.parent_id.as_deref() == Some(execution_id) {
                node.parent_id = Some("display-root".into());
            }
            node
        })
        .collect();
    WorkspaceTree::restore("/repo", nodes).unwrap()
}

#[tokio::test]
async fn test_背景失敗の表示変換_三種類の対象idで要対応と理由を反映し成功後に解除する() {
    // Given
    let original = tree();
    let node = original
        .nodes()
        .iter()
        .find(|node| node.node_execution_id.is_some())
        .unwrap();
    for target in [
        node.id.clone(),
        node.node_execution_id.clone().unwrap(),
        node.execution_id.clone().unwrap(),
    ] {
        let queue = queue();
        let key = WorkKey::new("workflow_recovery", &target);
        queue
            .observe(
                &key,
                &WorkFailure {
                    kind: FailureKind::StateRequired,
                    message: "repair required".into(),
                },
            )
            .await;
        // When
        let mut projected = original.clone();
        queue
            .failure_query()
            .apply_workflow_failures(&mut projected)
            .await;
        // Then
        let actual = projected
            .nodes()
            .iter()
            .find(|candidate| candidate.id == node.id)
            .unwrap();
        assert_eq!(
            actual.status_classification,
            WorkspaceNodeStatusClassification::Attention
        );
        assert_eq!(actual.error_reason.as_deref(), Some("repair required"));
        assert_eq!(
            projected
                .nodes()
                .iter()
                .find(|node| node.id == "display-root")
                .unwrap()
                .status_classification,
            WorkspaceNodeStatusClassification::Attention
        );
        queue
            .execute(key, RetryBackoff::RECOVERY, |_| async { Ok(()) })
            .await
            .unwrap();
        let mut recovered = original.clone();
        queue
            .failure_query()
            .apply_workflow_failures(&mut recovered)
            .await;
        assert_eq!(recovered, original);
    }
}

#[tokio::test]
async fn test_背景失敗の表示変換_無関係な対象と要対応でない分類を反映しない() {
    // Given
    let original = tree();
    let target = original.nodes()[0].id.clone();
    for (target, kind) in [
        (target.clone(), FailureKind::Cancelled),
        (target, FailureKind::Temporary),
        ("other-tree".into(), FailureKind::Internal),
    ] {
        let queue = queue();
        queue
            .observe(
                &WorkKey::new("workflow_recovery", &target),
                &WorkFailure {
                    kind,
                    message: "failure".into(),
                },
            )
            .await;
        // When
        let mut projected = original.clone();
        queue
            .failure_query()
            .apply_workflow_failures(&mut projected)
            .await;
        // Then
        assert_eq!(projected, original);
    }
}
