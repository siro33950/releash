use std::sync::Arc;

use crate::adaptor::controller::wiring::{
    build_repository_usecase, build_workflow_usecase_and_store,
};
use crate::adaptor::gateway::workflow::{fact_log, RepoPathsManagedWorktreeGateway};
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::SessionExecutionTreeRootFacts;
use crate::test_support::git::{create_initial_commit, create_test_repo};

#[test]
fn test_archive_restore認可_別名は受理し非管理対象と別worktreeは拒否する() {
    // Given
    let (repo_dir, repo) = create_test_repo();
    create_initial_commit(&repo);
    let directory = tempfile::tempdir().unwrap();
    let worktree = directory.path().join("managed-worktree");
    repo.worktree("managed-worktree", &worktree, None).unwrap();
    let worktree = worktree.canonicalize().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_path = outside.path().canonicalize().unwrap();
    let managed_id = "agent-session-00000000000040008000000000000001";
    let unmanaged_id = "agent-session-00000000000040008000000000000002";
    let (mut usecase, store) = build_workflow_usecase_and_store(directory.path().join("data"));
    usecase.worktrees = Arc::new(RepoPathsManagedWorktreeGateway::new(
        Arc::new(build_repository_usecase()),
        vec![repo_dir.path().to_str().unwrap().to_string()],
    ));
    for (id, path) in [(managed_id, &worktree), (unmanaged_id, &outside_path)] {
        let path = path.to_str().unwrap();
        let facts =
            SessionExecutionTreeRootFacts::new(id, path, path, ProviderKind::Codex, None).unwrap();
        fact_log::append_fact_batch_for_seed(&store, &facts.into_facts(), 1, id).unwrap();
    }

    // When / Then
    for (path, id, expected_error) in [
        (worktree.join("."), managed_id, None),
        (worktree.clone(), managed_id, None),
        (
            repo_dir.path().to_path_buf(),
            managed_id,
            Some("execution tree worktree does not match"),
        ),
        (
            outside_path,
            unmanaged_id,
            Some("worktree_path is not a configured git worktree"),
        ),
    ] {
        let result = usecase.authorize_archive_target(path.to_str().unwrap(), id);
        match expected_error {
            Some(message) => assert!(result.unwrap_err().to_string().contains(message)),
            None => result.unwrap(),
        }
    }
}
