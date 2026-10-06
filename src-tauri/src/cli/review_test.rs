use super::*;
use crate::cli::common::test_helpers_common::{
    review_cli_thread, review_history_entries, test_uuid,
};
use crate::cli::{Cli, TopCommand};
use clap::Parser;

#[test]
fn test_agent_session_resolves_review_actor_without_a_model() {
    let session_id = "agent-session-1";
    let (actor, worktree_path) = review_actor_and_worktree_from_context(
        session_id,
        ReviewSessionContext::Provider(AgentSessionItemDto {
            id: session_id.to_string(),
            workspace_identity: "/repo/worktree".to_string(),
            worktree_path: "/repo-worktrees/.releash-isolated/node-a1".to_string(),
            workspace_worktree_path: "/repo//worktree/".to_string(),
            provider: AgentSessionProviderDto::Claude,
            tree_location: crate::usecase::agent_session::AgentSessionTreeLocationDto {
                tree_id: "workflow-1".to_string(),
                node_execution_id: "node-1".to_string(),
            },
            lifecycle: AgentSessionLifecycleDto::Open,
            provider_session_id: None,
            transcript_ref: None,
            operations: crate::usecase::agent_session::AgentSessionOperationsDto {
                can_archive: false,
                can_restore: false,
                can_delete: false,
            },
            last_exit_abnormal: false,
            terminal_presence: None,
        }),
    )
    .unwrap();

    assert_eq!(worktree_path, "/repo//worktree/");
    assert_eq!(actor.backend_id.as_deref(), Some("claude"));
    assert_eq!(actor.model, None);
    assert_eq!(actor.session_id.as_deref(), Some(session_id));
}

#[test]
fn review_thread_json_formatter_preserves_field_shape() {
    let thread = review_cli_thread(ReviewThreadState::Resolved);
    let mut output = Vec::new();

    write_review_thread(&mut output, &thread, true).unwrap();

    let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(value["id"], thread.id);
    assert_eq!(value["worktreeName"], "/repo");
    assert_eq!(value["author"]["kind"], "agent");
    assert_eq!(value["author"]["backendId"], "codex");
    assert_eq!(value["author"]["model"], serde_json::Value::Null);
    assert!(value["author"].get("sessionId").is_none());
    assert_eq!(value["target"]["filePath"], "src/main.rs");
    assert_eq!(value["target"]["lineNumber"], 3);
    assert_eq!(value["target"]["endLine"], 5);
    assert_eq!(value["state"], "resolved");
    assert_eq!(value["comments"][0]["threadId"], thread.id);
    assert_eq!(value["resolve"]["outcome"], "accepted");
    assert_eq!(value["canResolve"], false);
}

#[test]
fn review_history_json_formatter_preserves_field_shape() {
    let entries = review_history_entries();
    let mut output = Vec::new();

    write_review_history(&mut output, &entries, true).unwrap();

    let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(value[0]["kind"], "thread_created");
    assert_eq!(value[0]["threadId"], test_uuid(42));
    assert_eq!(value[0]["commentId"], test_uuid(51));
    assert_eq!(value[0]["actor"]["displayName"], "codex");
    assert_eq!(value[0]["target"]["filePath"], "src/main.rs");
    assert_eq!(value[1]["kind"], "thread_resolved");
    assert_eq!(value[1]["outcome"], "accepted");
    assert!(value[1]["actor"].get("sessionId").is_none());
}

#[test]
fn review_human_formatters_preserve_representative_output() {
    let open = review_cli_thread(ReviewThreadState::Open);
    let resolved = review_cli_thread(ReviewThreadState::Resolved);

    let mut empty_list = Vec::new();
    write_review_thread_list(&mut empty_list, &[], false).unwrap();
    assert_eq!(
        String::from_utf8(empty_list).unwrap(),
        "(no review threads)\n"
    );

    let mut list = Vec::new();
    write_review_thread_list(&mut list, std::slice::from_ref(&open), false).unwrap();
    let list = String::from_utf8(list).unwrap();
    assert!(list.contains("THREAD_ID"));
    assert!(list.contains("STATE"));
    assert!(list.contains("AUTHOR"));
    assert!(list.contains("UPDATED"));
    assert!(list.contains(&open.id));
    assert!(list.contains("open"));

    let mut detail = Vec::new();
    write_review_thread(&mut detail, &resolved, false).unwrap();
    let detail = String::from_utf8(detail).unwrap();
    assert!(detail.contains("resolve:   accepted by Human (done)"));

    let mut empty_history = Vec::new();
    write_review_history(&mut empty_history, &[], false).unwrap();
    assert_eq!(
        String::from_utf8(empty_history).unwrap(),
        "(no review history)\n"
    );

    let entries = review_history_entries();
    let mut history = Vec::new();
    write_review_history(&mut history, &entries, false).unwrap();
    let history = String::from_utf8(history).unwrap();
    assert!(history.contains("ThreadCreated"));
    assert!(history.contains("ThreadResolved"));
    assert!(history.contains("thread_id"));
}

#[test]
fn review_subcommand_descriptions_match_split_before_golden() {
    let error = Cli::try_parse_from(["releash", "review", "--help"]).unwrap_err();
    assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp);
    assert_eq!(
            error.to_string(),
            "Agent review comment サブコマンド。\n\nUsage: releash review <COMMAND>\n\nCommands:\n  list     review Thread 一覧を表示する。\n  get      review Thread 詳細を表示する。\n  create   初回 Comment とともに review Thread を作成する。\n  comment  open Thread に Comment を追記する。\n  resolve  作成者 Agent として open Thread を resolve する。\n  history  Thread 履歴を表示する。\n\nOptions:\n  -h, --help  Print help\n"
        );
}

#[test]
fn review_cli_parser_accepts_review_subcommands() {
    let parsed = Cli::try_parse_from([
        "releash",
        "review",
        "create",
        "--session-id",
        "session-1",
        "--content",
        "Claim",
        "--json",
    ])
    .unwrap();

    match parsed.command {
        TopCommand::Review {
            command:
                ReviewSubcommand::Create {
                    session_id,
                    content,
                    json,
                    ..
                },
        } => {
            assert_eq!(session_id, "session-1");
            assert_eq!(content, "Claim");
            assert!(json);
        }
        _ => panic!("expected review create command"),
    }
}

#[test]
fn review_cli_parser_accepts_list_without_session_id() {
    let parsed = Cli::try_parse_from(["releash", "review", "list", "--state", "open"]).unwrap();

    match parsed.command {
        TopCommand::Review {
            command: ReviewSubcommand::List { session_id, .. },
        } => assert_eq!(session_id, None),
        _ => panic!("expected review list command"),
    }
}
