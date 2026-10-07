use crate::test_support_git::{create_initial_commit, create_test_repo};
pub(crate) mod repository_usecase_tests {
    use super::*;

    use releashd::test_support::integration::repository::WorktreeGateway;

    use std::sync::Arc;

    use releashd::test_support::integration::repository::{
        repository_usecase as usecase, FakeRepo,
    };
    #[tokio::test]
    pub async fn test_worktree削除_実gitの拒否条件では実行木を変更しない() {
        use crate::test_support_git::create_initial_commit;
        use crate::test_support_git::create_test_repo;
        use releashd::test_support::integration::repository::WorktreeGateway;
        for condition in ["wrong-repo", "locked", "dirty", "valid", "archive-failure"] {
            // Given
            let (repo_dir, repo) = create_test_repo();
            create_initial_commit(&repo);
            let worktrees = tempfile::tempdir().unwrap();
            let path = worktrees.path().join("feature");
            let worktree = repo.worktree("feature", &path, None).unwrap();
            let (other_dir, _other) = create_test_repo();
            if condition == "locked" {
                worktree.lock(None).unwrap();
            }
            if condition == "dirty" {
                std::fs::write(path.join("dirty"), "change").unwrap();
            }
            let fake = Arc::new(FakeRepo {
                fail_archive: condition == "archive-failure",
                ..Default::default()
            });
            let mut usecase = usecase(fake.clone());
            usecase.test_replace_worktree_repository(Arc::new(WorktreeGateway));
            let root = if condition == "wrong-repo" {
                other_dir.path()
            } else {
                repo_dir.path()
            };
            // When
            let result = usecase
                .remove_worktree(
                    fake.as_ref(),
                    root.to_str().unwrap(),
                    path.to_str().unwrap(),
                    false,
                )
                .await;
            // Then
            assert_eq!(result.is_ok(), condition == "valid");
            fake.wait_for_deletion(path.to_str().unwrap()).await;
            assert_eq!(
                fake.archived_worktrees.lock().len(),
                usize::from(matches!(condition, "valid" | "archive-failure"))
            );
            assert_eq!(path.exists(), condition != "valid");
            assert_eq!(repo.find_worktree("feature").is_ok(), condition != "valid");
        }
    }

    #[tokio::test]
    pub async fn test_worktree削除_先行変更の待機後に削除条件を再検証する() {
        for condition in ["dirty", "locked", "unregistered", "valid", "force-dirty"] {
            // Given
            let (repo_dir, repo) = create_test_repo();
            create_initial_commit(&repo);
            let worktrees = tempfile::tempdir().unwrap();
            let path = worktrees.path().canonicalize().unwrap().join("feature");
            let worktree = repo.worktree("feature", &path, None).unwrap();
            let root = repo_dir.path().to_str().unwrap();
            let path_str = path.to_str().unwrap();
            let fake = Arc::new(FakeRepo {
                current_branch: "main".into(),
                ..Default::default()
            });
            let mut repository = usecase(fake.clone());
            repository.test_replace_worktree_repository(Arc::new(WorktreeGateway));
            let mutation = fake.operations.mutate(path_str).unwrap();
            let force = condition == "force-dirty";
            let deletion = repository.remove_worktree(fake.as_ref(), root, path_str, force);
            tokio::pin!(deletion);

            // When
            assert!(futures_util::poll!(&mut deletion).is_pending());
            assert!(fake.archived_worktrees.lock().is_empty());
            assert!(fake.operations.mutate(path_str).is_err());
            match condition {
                "dirty" | "force-dirty" => {
                    std::fs::write(path.join("dirty"), "change").unwrap();
                }
                "locked" => worktree.lock(None).unwrap(),
                "unregistered" => {
                    std::fs::remove_dir_all(repo.path().join("worktrees/feature")).unwrap();
                }
                _ => {}
            }
            drop(mutation);
            let result = deletion.await;
            fake.wait_for_deletion(path_str).await;

            // Then
            let accepted = matches!(condition, "valid" | "force-dirty");
            assert_eq!(result.is_ok(), accepted, "{condition}");
            assert_eq!(fake.archived_worktrees.lock().len(), usize::from(accepted));
            assert_eq!(
                fake.killed_worktree_terminals.lock().len(),
                usize::from(accepted)
            );
            assert_eq!(path.exists(), !accepted);
            assert!(fake.operations.mutate(path_str).is_ok());
        }
    }
}
