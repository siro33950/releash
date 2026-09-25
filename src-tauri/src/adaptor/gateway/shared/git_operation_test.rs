use super::*;
use crate::domain::operation_context::{Deadline, OperationContext};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[test]
fn test_git操作_実行中の取り消し後は次の操作に進まない() {
    // Given
    let token = tokio_util::sync::CancellationToken::new();
    let context = OperationContext::new(None, Arc::new(token.clone()));
    let calls = AtomicUsize::new(0);
    // When
    crate::other::operation_context::sync_scope(context, || {
        let result = run(|| {
            calls.fetch_add(1, Ordering::SeqCst);
            token.cancel();
            Ok(())
        });
        assert!(matches!(
            result,
            Err(GitOperationError::Stopped(OperationStopped::Cancelled))
        ));
        assert!(run(|| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .is_err());
    });
    // Then
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn test_git操作_期限切れではステージもブランチ作成も実行しない() {
    // Given
    let context =
        OperationContext::default().with_deadline(Deadline::new(std::time::Instant::now()));
    // When / Then
    crate::other::operation_context::sync_scope(context, || {
        assert!(matches!(
            crate::adaptor::gateway::code::staging::git_stage("/missing", vec![]),
            Err(crate::domain::code::CodeError::Stopped(
                OperationStopped::Expired
            ))
        ));
        assert!(matches!(
            crate::adaptor::gateway::repository::branch::git_create_branch("/missing", "branch"),
            Err(crate::domain::repository::RepositoryError::Stopped(
                OperationStopped::Expired
            ))
        ));
    });
}

#[test]
fn test_checkout_notifyで操作途中の取消を検出する() {
    struct CancelOnNotify {
        calls: AtomicUsize,
    }
    impl crate::domain::operation_context::Cancellation for CancelOnNotify {
        fn is_cancelled(&self) -> bool {
            self.calls.fetch_add(1, Ordering::SeqCst) > 0
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(directory.path()).unwrap();
    std::fs::write(directory.path().join("file"), "committed").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("file")).unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let signature = git2::Signature::now("test", "test@example.com").unwrap();
    repo.commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
        .unwrap();
    std::fs::remove_file(directory.path().join("file")).unwrap();
    let cancel = Arc::new(CancelOnNotify {
        calls: AtomicUsize::new(0),
    });
    crate::other::operation_context::sync_scope(
        OperationContext::new(None, cancel.clone()),
        || {
            let mut options = checkout();
            options.force();
            let result = run(|| {
                let result = repo.checkout_head(Some(&mut options));
                assert!(
                    cancel.calls.load(Ordering::SeqCst) >= 2,
                    "notify must check cancellation inside checkout"
                );
                assert!(
                    !directory.path().join("file").exists(),
                    "notify must stop checkout before restoring the file"
                );
                result
            });
            assert!(matches!(
                result,
                Err(GitOperationError::Stopped(OperationStopped::Cancelled))
            ));
        },
    );
    assert!(cancel.calls.load(Ordering::SeqCst) >= 3);
    assert!(!directory.path().join("file").exists());
}

#[test]
fn test_branch探索_停止を未検出や空名へ変換しない() {
    struct CancelAfter {
        remaining: AtomicUsize,
    }
    impl crate::domain::operation_context::Cancellation for CancelAfter {
        fn is_cancelled(&self) -> bool {
            self.remaining.fetch_sub(1, Ordering::SeqCst) == 0
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(directory.path()).unwrap();
    for after in 0..=4 {
        let context = OperationContext::new(
            None,
            Arc::new(CancelAfter {
                remaining: AtomicUsize::new(after),
            }),
        );
        let result =
            crate::other::operation_context::sync_scope(context, || detect_default_branch(&repo));
        assert_eq!(
            result,
            Err(OperationStopped::Cancelled),
            "checkpoint {after}"
        );
    }
    for after in 0..=1 {
        let context = OperationContext::new(
            None,
            Arc::new(CancelAfter {
                remaining: AtomicUsize::new(after),
            }),
        );
        assert_eq!(
            crate::other::operation_context::sync_scope(context, || get_branch_name_for_repo(
                &repo
            )),
            Err(OperationStopped::Cancelled)
        );
    }
    let context =
        OperationContext::default().with_deadline(Deadline::new(std::time::Instant::now()));
    crate::other::operation_context::sync_scope(context, || {
        assert_eq!(detect_default_branch(&repo), Err(OperationStopped::Expired));
        assert_eq!(
            get_branch_name_for_repo(&repo),
            Err(OperationStopped::Expired)
        );
    });
}
