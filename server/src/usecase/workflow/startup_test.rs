use super::*;
use crate::domain::workflow::NodeFact;
use crate::usecase::workflow::test_helpers::startup::*;
use std::sync::{Arc, Mutex};
#[tokio::test]
async fn test_起動時定義確認_読めない定義は要対応を返す() {
    let repository = repository();
    for _ in 0..2 {
        assert!(matches!(
            check_startup_definition(&repository, "tree").await,
            Err(WorkflowError::IncompatibleStoredEvent(reason)) if reason.contains("completion")
        ));
    }
    assert!(repository.terminal.lock().unwrap().is_none());
}

#[tokio::test]
async fn test_起動時定義確認_読める定義と既存の終端事実を受理する() {
    for terminal in [
        None,
        Some(NodeFact::ExecutionCompleted),
        Some(NodeFact::AbortRequested(Default::default())),
    ] {
        let mut repository = repository();
        repository.unreadable = terminal.is_some();
        *repository.terminal.lock().unwrap() = terminal;
        check_startup_definition(&repository, "tree").await.unwrap();
    }
}

#[tokio::test]
async fn test_起動時定義確認_読取失敗を返し修復後に成功する() {
    let mut repository = repository();
    repository.fail_load = true;
    assert!(check_startup_definition(&repository, "tree")
        .await
        .unwrap_err()
        .to_string()
        .contains("read failed"));
    repository.fail_load = false;
    repository.unreadable = false;
    check_startup_definition(&repository, "tree").await.unwrap();
}

#[tokio::test]
async fn test_実行木の再開_読み直しのときだけ定義を確かめ直す() {
    // Given
    let startup = Arc::new(Startup {
        calls: Mutex::new(Vec::new()),
        failure: None,
        conflict: false,
        temporary: false,
    });
    let usecase = WorkflowStartupUsecase::new(startup.clone(), startup.clone());
    // When
    usecase
        .recover_tree("first", crate::common::retry::AttemptProgress::Continue)
        .await
        .unwrap();
    usecase
        .recover_tree("first", crate::common::retry::AttemptProgress::Continue)
        .await
        .unwrap();
    usecase
        .recover_tree("first", crate::common::retry::AttemptProgress::Reload)
        .await
        .unwrap();
    // Then
    assert_eq!(
        *startup.calls.lock().unwrap(),
        [
            "load:first",
            "reconcile:first",
            "reconcile:first",
            "load:first",
            "reconcile:first",
        ]
    );
}
