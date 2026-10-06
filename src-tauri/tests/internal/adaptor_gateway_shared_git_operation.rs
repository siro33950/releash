use releash_lib::test_support::integration::platform::checkout;
use releash_lib::test_support::integration::platform::detect_default_branch;
use releash_lib::test_support::integration::platform::get_branch_name_for_repo;
use releash_lib::test_support::integration::platform::git_operation_run as run;
use releash_lib::test_support::integration::platform::CancelAfter;
use releash_lib::test_support::integration::platform::Deadline;
use releash_lib::test_support::integration::platform::GitOperationError;
use releash_lib::test_support::integration::platform::OperationContext;
use releash_lib::test_support::integration::platform::OperationStopped;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[test]
pub fn test_checkout_notifyで操作途中の取消を検出する() {
    struct CancelOnNotify {
        calls: AtomicUsize,
    }
    impl releash_lib::test_support::integration::platform::Cancellation for CancelOnNotify {
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
    releash_lib::test_support::integration::platform::sync_scope(
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
pub fn test_既定ブランチ探索_停止地点0を未検出や空名へ変換しない() {
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
    let result = releash_lib::test_support::integration::platform::sync_scope(context, || {
        detect_default_branch(&repo)
    });
    // Then
    assert!(matches!(
        result,
        Err(GitOperationError::Stopped(OperationStopped::Cancelled))
    ));
}

#[test]
pub fn test_既定ブランチ探索_停止地点1を未検出や空名へ変換しない() {
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
    let result = releash_lib::test_support::integration::platform::sync_scope(context, || {
        detect_default_branch(&repo)
    });
    // Then
    assert!(matches!(
        result,
        Err(GitOperationError::Stopped(OperationStopped::Cancelled))
    ));
}

#[test]
pub fn test_既定ブランチ探索_停止地点2を未検出や空名へ変換しない() {
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
    let result = releash_lib::test_support::integration::platform::sync_scope(context, || {
        detect_default_branch(&repo)
    });
    // Then
    assert!(matches!(
        result,
        Err(GitOperationError::Stopped(OperationStopped::Cancelled))
    ));
}

#[test]
pub fn test_既定ブランチ探索_停止地点3を未検出や空名へ変換しない() {
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
    let result = releash_lib::test_support::integration::platform::sync_scope(context, || {
        detect_default_branch(&repo)
    });
    // Then
    assert!(matches!(
        result,
        Err(GitOperationError::Stopped(OperationStopped::Cancelled))
    ));
}

#[test]
pub fn test_既定ブランチ探索_停止地点4を未検出や空名へ変換しない() {
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
    let result = releash_lib::test_support::integration::platform::sync_scope(context, || {
        detect_default_branch(&repo)
    });
    // Then
    assert!(matches!(
        result,
        Err(GitOperationError::Stopped(OperationStopped::Cancelled))
    ));
}

#[test]
pub fn test_既定ブランチ探索_期限切れを未検出や空名へ変換しない() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(directory.path()).unwrap();
    let context =
        OperationContext::default().with_deadline(Deadline::new(std::time::Instant::now()));
    // When
    let result = releash_lib::test_support::integration::platform::sync_scope(context, || {
        detect_default_branch(&repo)
    });
    // Then
    assert!(matches!(
        result,
        Err(GitOperationError::Stopped(OperationStopped::Expired))
    ));
}

#[test]
pub fn test_ブランチ名探索_停止地点0を未検出や空名へ変換しない() {
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
    let result = releash_lib::test_support::integration::platform::sync_scope(context, || {
        get_branch_name_for_repo(&repo)
    });
    // Then
    assert!(matches!(
        result,
        Err(GitOperationError::Stopped(OperationStopped::Cancelled))
    ));
}

#[test]
pub fn test_ブランチ名探索_停止地点1を未検出や空名へ変換しない() {
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
    let result = releash_lib::test_support::integration::platform::sync_scope(context, || {
        get_branch_name_for_repo(&repo)
    });
    // Then
    assert!(matches!(
        result,
        Err(GitOperationError::Stopped(OperationStopped::Cancelled))
    ));
}

#[test]
pub fn test_ブランチ名探索_期限切れを未検出や空名へ変換しない() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(directory.path()).unwrap();
    let context =
        OperationContext::default().with_deadline(Deadline::new(std::time::Instant::now()));
    // When
    let result = releash_lib::test_support::integration::platform::sync_scope(context, || {
        get_branch_name_for_repo(&repo)
    });
    // Then
    assert!(matches!(
        result,
        Err(GitOperationError::Stopped(OperationStopped::Expired))
    ));
}

#[test]
pub fn test_既定ブランチ読取_参照の破損を未設定と区別する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(directory.path()).unwrap();
    std::fs::write(repo.path().join("packed-refs"), "invalid packed refs").unwrap();
    // When
    let result = detect_default_branch(&repo);
    // Then
    assert!(result.is_err());
}
