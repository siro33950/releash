use super::*;
use crate::common::operation_context::{Deadline, OperationContext};
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
    crate::common::operation_context::sync_scope(context, || {
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
    crate::common::operation_context::sync_scope(context, || {
        assert!(matches!(
            crate::adaptor::gateway::code::staging::git_stage("/missing", vec![]),
            Err(crate::domain::code::CodeError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                    ..
                }
            ))
        ));
        assert!(matches!(
            crate::adaptor::gateway::repository::branch::git_create_branch("/missing", "branch"),
            Err(crate::domain::repository::RepositoryError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                    ..
                }
            ))
        ));
    });
}

#[test]
fn test_checkout_notifyで操作途中の取消を検出する() {
    struct CancelOnNotify {
        calls: AtomicUsize,
    }
    impl crate::common::operation_context::Cancellation for CancelOnNotify {
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
    crate::common::operation_context::sync_scope(
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

struct CancelAfter {
    remaining: AtomicUsize,
}
impl crate::common::operation_context::Cancellation for CancelAfter {
    fn is_cancelled(&self) -> bool {
        self.remaining.fetch_sub(1, Ordering::SeqCst) == 0
    }
}
#[test]
fn test_既定ブランチ探索_停止地点0を未検出や空名へ変換しない() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(directory.path()).unwrap();
    let context = OperationContext::new(
        None,
        Arc::new(CancelAfter {
            remaining: AtomicUsize::new(0),
        }),
    );
    // When
    let result =
        crate::common::operation_context::sync_scope(context, || detect_default_branch(&repo));
    // Then
    assert!(matches!(
        result,
        Err(GitOperationError::Stopped(OperationStopped::Cancelled))
    ));
}
#[test]
fn test_既定ブランチ探索_停止地点1を未検出や空名へ変換しない() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(directory.path()).unwrap();
    let context = OperationContext::new(
        None,
        Arc::new(CancelAfter {
            remaining: AtomicUsize::new(1),
        }),
    );
    // When
    let result =
        crate::common::operation_context::sync_scope(context, || detect_default_branch(&repo));
    // Then
    assert!(matches!(
        result,
        Err(GitOperationError::Stopped(OperationStopped::Cancelled))
    ));
}
#[test]
fn test_既定ブランチ探索_停止地点2を未検出や空名へ変換しない() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(directory.path()).unwrap();
    let context = OperationContext::new(
        None,
        Arc::new(CancelAfter {
            remaining: AtomicUsize::new(2),
        }),
    );
    // When
    let result =
        crate::common::operation_context::sync_scope(context, || detect_default_branch(&repo));
    // Then
    assert!(matches!(
        result,
        Err(GitOperationError::Stopped(OperationStopped::Cancelled))
    ));
}
#[test]
fn test_既定ブランチ探索_停止地点3を未検出や空名へ変換しない() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(directory.path()).unwrap();
    let context = OperationContext::new(
        None,
        Arc::new(CancelAfter {
            remaining: AtomicUsize::new(3),
        }),
    );
    // When
    let result =
        crate::common::operation_context::sync_scope(context, || detect_default_branch(&repo));
    // Then
    assert!(matches!(
        result,
        Err(GitOperationError::Stopped(OperationStopped::Cancelled))
    ));
}
#[test]
fn test_既定ブランチ探索_停止地点4を未検出や空名へ変換しない() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(directory.path()).unwrap();
    let context = OperationContext::new(
        None,
        Arc::new(CancelAfter {
            remaining: AtomicUsize::new(4),
        }),
    );
    // When
    let result =
        crate::common::operation_context::sync_scope(context, || detect_default_branch(&repo));
    // Then
    assert!(matches!(
        result,
        Err(GitOperationError::Stopped(OperationStopped::Cancelled))
    ));
}
#[test]
fn test_既定ブランチ探索_期限切れを未検出や空名へ変換しない() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(directory.path()).unwrap();
    let context =
        OperationContext::default().with_deadline(Deadline::new(std::time::Instant::now()));
    // When
    let result =
        crate::common::operation_context::sync_scope(context, || detect_default_branch(&repo));
    // Then
    assert!(matches!(
        result,
        Err(GitOperationError::Stopped(OperationStopped::Expired))
    ));
}
#[test]
fn test_ブランチ名探索_停止地点0を未検出や空名へ変換しない() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(directory.path()).unwrap();
    let context = OperationContext::new(
        None,
        Arc::new(CancelAfter {
            remaining: AtomicUsize::new(0),
        }),
    );
    // When
    let result =
        crate::common::operation_context::sync_scope(context, || get_branch_name_for_repo(&repo));
    // Then
    assert!(matches!(
        result,
        Err(GitOperationError::Stopped(OperationStopped::Cancelled))
    ));
}
#[test]
fn test_ブランチ名探索_停止地点1を未検出や空名へ変換しない() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(directory.path()).unwrap();
    let context = OperationContext::new(
        None,
        Arc::new(CancelAfter {
            remaining: AtomicUsize::new(1),
        }),
    );
    // When
    let result =
        crate::common::operation_context::sync_scope(context, || get_branch_name_for_repo(&repo));
    // Then
    assert!(matches!(
        result,
        Err(GitOperationError::Stopped(OperationStopped::Cancelled))
    ));
}
#[test]
fn test_ブランチ名探索_期限切れを未検出や空名へ変換しない() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(directory.path()).unwrap();
    let context =
        OperationContext::default().with_deadline(Deadline::new(std::time::Instant::now()));
    // When
    let result =
        crate::common::operation_context::sync_scope(context, || get_branch_name_for_repo(&repo));
    // Then
    assert!(matches!(
        result,
        Err(GitOperationError::Stopped(OperationStopped::Expired))
    ));
}
#[test]
fn test_git任意読取_notfoundだけを未設定として扱う() {
    // Given
    let error = git2::Error::new(
        git2::ErrorCode::NotFound,
        git2::ErrorClass::Reference,
        "missing",
    );
    // When
    let result = optional::<()>(Err(error.into()));
    // Then
    assert_eq!(result.unwrap(), None);
}
#[test]
fn test_git任意読取_notfound以外の失敗を未設定に変えない() {
    // Given
    let error = git2::Error::from_str("read failed");
    // When
    let result = optional::<()>(Err(error.into()));
    // Then
    assert!(matches!(result, Err(GitOperationError::Git(_))));
}

#[test]
fn test_既定ブランチ読取_参照の破損を未設定と区別する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(directory.path()).unwrap();
    std::fs::write(repo.path().join("packed-refs"), "invalid packed refs").unwrap();
    // When
    let result = detect_default_branch(&repo);
    // Then
    assert!(result.is_err());
}
