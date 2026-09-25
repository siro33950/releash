use super::*;
use crate::domain::workflow::{IsolatedWorktree, IsolatedWorktreeGateway};
use crate::test_support::git::{add_and_commit, create_initial_commit};

fn repository() -> (tempfile::TempDir, git2::Repository, String) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("repository");
    let repo = git2::Repository::init(&path).unwrap();
    create_initial_commit(&repo);
    (
        directory,
        repo,
        path.canonicalize().unwrap().to_string_lossy().into_owned(),
    )
}

#[test]
fn test_隔離生成_親のheadから生成し未コミット変更を継承しない() {
    // Given
    let (_directory, repo, root) = repository();
    let head = add_and_commit(&repo, "file", "committed", "parent");
    std::fs::write(repo.workdir().unwrap().join("file"), "uncommitted").unwrap();
    let worktree = IsolatedWorktree::for_attempt(&root, "node", 1);
    let gateway = RepositoryIsolatedWorktreeGateway;

    // When
    gateway.create(&root, &worktree).unwrap();
    let isolated = git2::Repository::open(&worktree.path).unwrap();

    // Then
    assert_eq!(isolated.head().unwrap().target(), Some(head));
    assert_eq!(
        isolated.head().unwrap().shorthand().unwrap(),
        worktree.branch.as_str()
    );
    assert_eq!(
        std::fs::read_to_string(format!("{}/file", worktree.path)).unwrap(),
        "committed"
    );
    assert_eq!(
        std::fs::read_to_string(format!("{root}/file")).unwrap(),
        "uncommitted"
    );
    assert!(!std::path::Path::new(&worktree.path).starts_with(&root));
    assert_eq!(gateway.repository_root(&worktree.path).unwrap(), root);
}

#[test]
fn test_隔離生成_入れ子は直近親のheadを使いslotと再試行を独立させる() {
    // Given
    let (_directory, repo, root) = repository();
    let root_head = add_and_commit(&repo, "file", "root", "root");
    let gateway = RepositoryIsolatedWorktreeGateway;
    let parent = IsolatedWorktree::for_attempt(&root, "parent", 1);
    gateway.create(&root, &parent).unwrap();
    let parent_repo = git2::Repository::open(&parent.path).unwrap();
    let parent_head = add_and_commit(&parent_repo, "file", "parent", "parent");
    let first = IsolatedWorktree::for_attempt(&root, "slot-one", 1);
    let second = IsolatedWorktree::for_attempt(&root, "slot-two", 1);
    let retry = IsolatedWorktree::for_attempt(&root, "slot-retry", 2);

    // When
    gateway.create(&parent.path, &first).unwrap();
    std::fs::write(format!("{}/file", first.path), "attempt one").unwrap();
    gateway.create(&parent.path, &second).unwrap();
    gateway.create(&parent.path, &retry).unwrap();

    // Then
    for child in [&second, &retry] {
        let child_repo = git2::Repository::open(&child.path).unwrap();
        assert_eq!(child_repo.head().unwrap().target(), Some(parent_head));
        assert_eq!(
            std::fs::read_to_string(format!("{}/file", child.path)).unwrap(),
            "parent"
        );
    }
    assert_eq!(repo.head().unwrap().target(), Some(root_head));
    assert_eq!(
        std::fs::read_to_string(format!("{root}/file")).unwrap(),
        "root"
    );
    assert_eq!(
        std::fs::read_to_string(format!("{}/file", parent.path)).unwrap(),
        "parent"
    );
    assert_eq!(
        std::fs::read_to_string(format!("{}/file", first.path)).unwrap(),
        "attempt one"
    );
}

#[test]
fn test_隔離生成_衝突時は既存branchとworktreeを削除しない() {
    // Given
    let (_directory, _repo, root) = repository();
    let gateway = RepositoryIsolatedWorktreeGateway;
    let first = IsolatedWorktree::for_attempt(&root, "node", 1);
    gateway.create(&root, &first).unwrap();
    std::fs::write(format!("{}/keep", first.path), "keep").unwrap();

    // When
    let error = gateway.create(&root, &first).unwrap_err();

    // Then
    assert!(!error.to_string().is_empty());
    assert_eq!(
        std::fs::read_to_string(format!("{}/keep", first.path)).unwrap(),
        "keep"
    );
    assert!(git2::Repository::open(&first.path).is_ok());
}

#[test]
fn test_隔離生成_headが未生成のrepositoryは失敗する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("empty");
    git2::Repository::init(&root).unwrap();
    let root = root.to_str().unwrap();
    let worktree = IsolatedWorktree::for_attempt(root, "node", 1);

    // When
    let result = RepositoryIsolatedWorktreeGateway.create(root, &worktree);

    // Then
    assert!(result.is_err());
    assert!(!std::path::Path::new(&worktree.path).exists());
}

#[test]
fn test_隔離生成確認_同じattemptの登録とbranchとpathが揃えば変更を保って再利用できる() {
    // Given
    let (_directory, repo, root) = repository();
    let gateway = RepositoryIsolatedWorktreeGateway;
    let worktree = IsolatedWorktree::for_attempt(&root, "node", 1);
    assert!(!gateway.is_created(&root, &worktree).unwrap());
    gateway.create(&root, &worktree).unwrap();
    let isolated = git2::Repository::open(&worktree.path).unwrap();
    let head = add_and_commit(&isolated, "file", "isolated", "isolated");
    std::fs::write(format!("{}/keep", worktree.path), "uncommitted").unwrap();
    add_and_commit(&repo, "file", "parent advanced", "parent");

    // When / Then
    assert!(gateway.is_created(&root, &worktree).unwrap());
    assert!(gateway.is_created(&worktree.path, &worktree).unwrap());
    assert_eq!(isolated.head().unwrap().target(), Some(head));
    assert_eq!(
        std::fs::read_to_string(format!("{}/keep", worktree.path)).unwrap(),
        "uncommitted"
    );
    assert!(!gateway
        .is_created(&root, &IsolatedWorktree::for_attempt(&root, "node", 2))
        .unwrap());
}

#[test]
fn test_隔離生成確認_branchだけが存在する場合は生成済みと扱わない() {
    // Given
    let (_directory, repo, root) = repository();
    let gateway = RepositoryIsolatedWorktreeGateway;
    let worktree = IsolatedWorktree::for_attempt(&root, "node", 1);
    repo.branch(
        &worktree.branch,
        &repo.head().unwrap().peel_to_commit().unwrap(),
        false,
    )
    .unwrap();

    // When / Then
    assert!(!gateway.is_created(&root, &worktree).unwrap());
    assert!(gateway.create(&root, &worktree).is_err());
    assert!(!std::path::Path::new(&worktree.path).exists());
}

#[test]
fn test_隔離生成確認_branch不一致と実体喪失は再利用せず失敗する() {
    // Given
    let (_directory, _repo, root) = repository();
    let gateway = RepositoryIsolatedWorktreeGateway;
    let worktree = IsolatedWorktree::for_attempt(&root, "node", 1);
    gateway.create(&root, &worktree).unwrap();
    let isolated = git2::Repository::open(&worktree.path).unwrap();
    isolated
        .set_head_detached(isolated.head().unwrap().target().unwrap())
        .unwrap();

    // When / Then
    assert!(gateway.is_created(&root, &worktree).is_err());
    std::fs::remove_dir_all(&worktree.path).unwrap();
    assert!(gateway.is_created(&root, &worktree).is_err());
    assert!(!std::path::Path::new(&worktree.path).exists());
}

#[test]
fn test_隔離生成確認_同名登録でもpathまたはrepositoryが違えば再利用しない() {
    // Given
    let (_directory, _repo, root) = repository();
    let (_other_directory, _other_repo, other_root) = repository();
    let gateway = RepositoryIsolatedWorktreeGateway;
    let worktree = IsolatedWorktree::for_attempt(&root, "node", 1);
    let other = IsolatedWorktree::for_attempt(&other_root, "node", 1);
    gateway.create(&root, &worktree).unwrap();
    gateway.create(&other_root, &other).unwrap();

    // When / Then
    assert!(gateway.is_created(&root, &other).is_err());
    std::fs::copy(
        format!("{}/.git", other.path),
        format!("{}/.git", worktree.path),
    )
    .unwrap();
    assert!(gateway.is_created(&root, &worktree).is_err());
}

#[test]
fn test_隔離worktree_全入口で期限切れと取り消しの分類を保持する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind};
    use crate::domain::operation_context::{Deadline, OperationContext};
    use std::sync::Arc;
    use std::time::Instant;
    // Given
    let (_directory, repo, root) = repository();
    let gateway = RepositoryIsolatedWorktreeGateway;
    let worktree = IsolatedWorktree::for_attempt(&root, "stopped", 1);
    let token = tokio_util::sync::CancellationToken::new();
    token.cancel();
    for (context, expected) in [
        (
            OperationContext::new(None, Arc::new(token)),
            FailureKind::Cancelled,
        ),
        (
            OperationContext::default().with_deadline(Deadline::new(Instant::now())),
            FailureKind::Expired,
        ),
    ] {
        // When / Then
        crate::other::operation_context::sync_scope(context, || {
            assert_eq!(
                gateway.repository_root(&root).unwrap_err().failure_kind(),
                expected
            );
            assert_eq!(
                gateway
                    .is_created(&root, &worktree)
                    .unwrap_err()
                    .failure_kind(),
                expected
            );
            assert_eq!(
                gateway.create(&root, &worktree).unwrap_err().failure_kind(),
                expected
            );
        });
        assert!(!std::path::Path::new(&worktree.path).exists());
        assert!(repo
            .find_branch(&worktree.branch, git2::BranchType::Local)
            .is_err());
    }
}

#[test]
fn test_managed_worktree解決_各操作の停止で別repositoryへ進まない() {
    use crate::adaptor::gateway::repository::test_helpers::assert_stops_at_each_checkpoint;
    // Given
    let (_directory, _repo, root) = repository();
    let (_second_directory, _second_repo, second_root) = repository();
    let gateway = RepoPathsManagedWorktreeGateway::new(
        Arc::new(crate::adaptor::controller::wiring::build_repository_usecase()),
        vec![root.clone(), second_root],
    );
    // When / Then
    assert_stops_at_each_checkpoint(|| gateway.resolve(&root));
}

#[test]
fn test_隔離生成確認_validateの停止を後続のパス検証エラーへ変えない() {
    use crate::domain::operation_context::{Cancellation, OperationContext, OperationStopped};
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct CancelAfterValidate(AtomicUsize);
    impl Cancellation for CancelAfterValidate {
        fn is_cancelled(&self) -> bool {
            self.0.fetch_add(1, Ordering::SeqCst) >= 5
        }
    }
    // Given
    let (_directory, _repo, root) = repository();
    let gateway = RepositoryIsolatedWorktreeGateway;
    let mut worktree = IsolatedWorktree::for_attempt(&root, "validate", 1);
    gateway.create(&root, &worktree).unwrap();
    let name = std::path::Path::new(&worktree.path).file_name().unwrap();
    worktree.path = std::path::Path::new(&root)
        .join("missing")
        .join(name)
        .to_string_lossy()
        .into_owned();
    let cancellation = Arc::new(CancelAfterValidate(AtomicUsize::new(0)));
    // When
    let result = crate::other::operation_context::sync_scope(
        OperationContext::new(None, cancellation.clone()),
        || gateway.is_created(&root, &worktree),
    );
    // Then
    assert!(matches!(
        result,
        Err(WorkflowError::Stopped(OperationStopped::Cancelled))
    ));
    assert_eq!(cancellation.0.load(Ordering::SeqCst), 6);
}
