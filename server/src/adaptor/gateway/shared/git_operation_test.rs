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
