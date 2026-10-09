use super::*;
use crate::usecase::test_helpers::{usecase, FakeRepo};
use parking_lot::Mutex;

#[derive(Default)]
struct Launcher {
    paths: Mutex<Vec<String>>,
    fail: bool,
}
#[async_trait::async_trait]
impl WorktreeLaunch for Launcher {
    async fn launch(&self, path: &str, _: &LaunchAfterCreation) -> Result<(), UsecaseError> {
        self.paths.lock().push(path.into());
        if self.fail {
            return Err(RepositoryError::rule("launch failed").into());
        }
        Ok(())
    }
}

#[tokio::test]
async fn test_複数作成_各worktreeの作成後に同じ起動を適用する() {
    // Given
    let repo = Arc::new(usecase(Arc::new(FakeRepo::default())));
    let launcher = Launcher::default();
    // When
    let paths = create_worktrees(
        repo,
        &launcher,
        &Launcher::default(),
        "/r".into(),
        vec!["a".into(), "b".into()],
        Some("main".into()),
        LaunchAfterCreation::None,
    )
    .await
    .unwrap();
    // Then
    assert_eq!(paths, vec!["/r-worktrees/a", "/r-worktrees/b"]);
    assert_eq!(*launcher.paths.lock(), paths);
}

#[tokio::test]
async fn test_複数作成_作成または起動の失敗を返して後続を作らない() {
    // Given / When / Then
    for fail_create in [false, true] {
        let fake = Arc::new(FakeRepo {
            fail_create_worktree: fail_create,
            ..Default::default()
        });
        let repo = Arc::new(usecase(fake.clone()));
        let launcher = Launcher {
            fail: true,
            ..Default::default()
        };
        assert!(create_worktrees(
            repo,
            &launcher,
            &Launcher::default(),
            "/r".into(),
            vec!["a".into(), "b".into()],
            None,
            LaunchAfterCreation::None
        )
        .await
        .is_err());
        assert_eq!(launcher.paths.lock().len(), if fail_create { 0 } else { 1 });
        let created = fake.worktree_creations.lock();
        assert!(!created.iter().any(|entry| entry.1 == "b"));
        if !fail_create {
            assert_eq!(
                created
                    .iter()
                    .map(|entry| entry.1.as_str())
                    .collect::<Vec<_>>(),
                ["a"]
            );
        }
    }
}

#[tokio::test]
async fn test_複数作成_空と重複の入力では作成も起動もしない() {
    // Given / When / Then
    for branches in [vec![], vec!["".into()], vec!["a".into(), "a".into()]] {
        let fake = Arc::new(FakeRepo::default());
        let repo = Arc::new(usecase(fake.clone()));
        let launcher = Launcher::default();
        assert!(create_worktrees(
            repo,
            &launcher,
            &Launcher::default(),
            "/r".into(),
            branches,
            None,
            LaunchAfterCreation::None
        )
        .await
        .is_err());
        assert!(launcher.paths.lock().is_empty());
        assert!(fake.worktree_creations.lock().is_empty());
    }
}

#[derive(Default)]
struct LaunchRequests {
    sessions: Mutex<Vec<AgentSessionLaunchRequest>>,
    workflows: Mutex<Vec<StartExecutionCommand>>,
}
#[async_trait::async_trait]
impl WorktreeSessionLaunch for Arc<LaunchRequests> {
    async fn launch_session(&self, request: AgentSessionLaunchRequest) -> Result<(), UsecaseError> {
        self.sessions.lock().push(request);
        Ok(())
    }
}
#[async_trait::async_trait]
impl WorktreeWorkflowLaunch for LaunchRequests {
    async fn launch_workflow(&self, command: StartExecutionCommand) -> Result<(), UsecaseError> {
        self.workflows.lock().push(command);
        Ok(())
    }
}

#[tokio::test]
async fn test_複数作成_実launcherが新規と既存の起動引数を渡す() {
    // Given
    for launch in [
        LaunchAfterCreation::Session {
            provider: ProviderKind::Codex,
            rows: 31,
            cols: 110,
            request_id: "request".into(),
        },
        LaunchAfterCreation::Workflow {
            name: "review".into(),
            request: Some("check changes".into()),
        },
    ] {
        let fake = Arc::new(FakeRepo {
            branches: vec![crate::domain::repository::Branch::local("existing")],
            ..Default::default()
        });
        let requests = Arc::new(LaunchRequests::default());
        let launcher = WorktreeLauncher {
            sessions: requests.clone(),
            workflows: requests.clone(),
        };
        // When
        let paths = create_worktrees(
            Arc::new(usecase(fake.clone())),
            &launcher,
            &Launcher::default(),
            "/r".into(),
            vec!["new".into(), "existing".into()],
            Some("main".into()),
            launch.clone(),
        )
        .await
        .unwrap();
        // Then
        assert_eq!(
            *fake.worktree_creations.lock(),
            vec![
                (
                    "/r-worktrees/new".into(),
                    "new".into(),
                    true,
                    Some("main".into())
                ),
                (
                    "/r-worktrees/existing".into(),
                    "existing".into(),
                    false,
                    Some("main".into())
                )
            ]
        );
        for (index, path) in paths.iter().enumerate() {
            match &launch {
                LaunchAfterCreation::Session {
                    provider,
                    rows,
                    cols,
                    request_id,
                } => assert_eq!(
                    requests.sessions.lock()[index],
                    AgentSessionLaunchRequest {
                        workspace: WorkspaceIdentity::new(path),
                        worktree_path: path.clone(),
                        provider: *provider,
                        rows: *rows,
                        cols: *cols,
                        caller_request_id: format!("{request_id}:{path}")
                    }
                ),
                LaunchAfterCreation::Workflow { name, request } => assert_eq!(
                    requests.workflows.lock()[index],
                    StartExecutionCommand {
                        workflow_name: name.clone(),
                        worktree_path: path.clone(),
                        request: request.clone(),
                        created_from: crate::domain::workflow::ExecutionOrigin::DesktopUi
                    }
                ),
                LaunchAfterCreation::None => unreachable!(),
            }
        }
    }
}

impl WorktreeCreation for Launcher {
    fn begin_creation(
        &self,
        _: &str,
        _: &str,
    ) -> Result<Vec<crate::usecase::worktree_operation::WorktreeMutationGuard>, UsecaseError> {
        Ok(Vec::new())
    }
}
