use releash_lib::test_support::integration::platform::build_review_comment_usecase;
use releash_lib::test_support::integration::platform::CliError;
use releash_lib::test_support::integration::platform::ReviewActor;
use releash_lib::test_support::integration::platform::ReviewTarget;
use releash_lib::test_support::integration::review::cmd_review;
use releash_lib::test_support::integration::review::ReviewSubcommand;

#[tokio::test]
pub async fn test_review_cli変更_別所有者のworktree削除中は拒否し読み取りを許可する() {
    use crate::cli_common_test_support::write_review_config;
    use crate::cli_common_test_support::write_review_session;
    use releash_lib::test_support::integration::platform::WorktreeOperations;
    use releash_lib::test_support::integration::repository::FileWorktreeOperationLocks;
    // Given
    let directory = tempfile::tempdir().unwrap();
    write_review_config(directory.path());
    let id = uuid::Uuid::new_v4().to_string();
    write_review_session(directory.path(), &id, Some("codex")).await;
    let thread = build_review_comment_usecase()
        .create_thread(
            directory.path(),
            "/repo",
            ReviewActor::human(),
            ReviewTarget {
                file_path: None,
                line_number: None,
                end_line: None,
            },
            "before".into(),
        )
        .unwrap();
    let file =
        releash_lib::test_support::integration::platform::state_file(directory.path(), "/repo");
    let before = std::fs::read(&file).unwrap();
    let operations = WorktreeOperations::new(std::sync::Arc::new(FileWorktreeOperationLocks::new(
        directory.path(),
    )));
    let deletion = operations.delete("/repo").await.unwrap();
    // When / Then
    for command in [
        ReviewSubcommand::Create {
            session_id: id.clone(),
            content: "during".into(),
            file: None,
            line: None,
            end_line: None,
            json: true,
        },
        ReviewSubcommand::Comment {
            session_id: id.clone(),
            thread_id: thread.id.clone(),
            content: "during".into(),
            json: true,
        },
        ReviewSubcommand::Resolve {
            session_id: id.clone(),
            thread_id: thread.id.clone(),
            outcome: "accepted".into(),
            summary: "during".into(),
            json: true,
        },
    ] {
        assert!(
            matches!(cmd_review(directory.path(), command).await, Err(CliError::InvalidInput(message)) if message.contains("deletion is in progress"))
        );
        assert_eq!(std::fs::read(&file).unwrap(), before);
    }
    for command in [
        ReviewSubcommand::List {
            session_id: Some(id.clone()),
            file: None,
            state: None,
            author: None,
            unread: None,
            thread_id: vec![],
            json: true,
        },
        ReviewSubcommand::Get {
            session_id: id.clone(),
            thread_id: thread.id.clone(),
            json: true,
        },
        ReviewSubcommand::History {
            session_id: id,
            thread_id: thread.id.clone(),
            json: true,
        },
    ] {
        assert!(cmd_review(directory.path(), command)
            .await
            .unwrap()
            .contains(&thread.id));
    }
    drop(deletion);
}
