use std::io::{self, Write};
use std::path::Path;

use clap::Subcommand;

use super::common::{truncate, CliError};
use crate::adaptor::controller::wiring::build_review_comment_usecase;
use crate::domain::comment::{
    AuthorScope, ReviewActor, ReviewError, ReviewHistoryEntry, ReviewTarget, ReviewThread,
    ReviewThreadFilter, ReviewThreadState,
};
use crate::usecase::agent_session::session::SessionState;
use crate::usecase::comment::{ReviewHistoryEntryDto, ReviewThreadDto};

#[derive(Subcommand, Debug)]
pub(super) enum ReviewSubcommand {
    /// review Thread 一覧を表示する。
    List {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        file: Option<String>,
        #[arg(long)]
        state: Option<String>,
        #[arg(long)]
        author: Option<String>,
        #[arg(long)]
        unread: Option<String>,
        #[arg(long = "thread-id")]
        thread_id: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    /// review Thread 詳細を表示する。
    Get {
        thread_id: String,
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        json: bool,
    },
    /// 初回 Comment とともに review Thread を作成する。
    Create {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        content: String,
        #[arg(long)]
        file: Option<String>,
        #[arg(long)]
        line: Option<u32>,
        #[arg(long)]
        end_line: Option<u32>,
        #[arg(long)]
        json: bool,
    },
    /// open Thread に Comment を追記する。
    Comment {
        thread_id: String,
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        content: String,
        #[arg(long)]
        json: bool,
    },
    /// 作成者 Agent として open Thread を resolve する。
    Resolve {
        thread_id: String,
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        outcome: String,
        #[arg(long)]
        summary: String,
        #[arg(long)]
        json: bool,
    },
    /// Thread 履歴を表示する。
    History {
        thread_id: String,
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        json: bool,
    },
}

#[cfg(test)]
fn review_actor(data_dir: &Path, session_id: &str) -> Result<ReviewActor, CliError> {
    review_actor_and_worktree(data_dir, session_id).map(|(actor, _)| actor)
}

fn review_actor_and_worktree(
    data_dir: &Path,
    session_id: &str,
) -> Result<(ReviewActor, String), CliError> {
    if session_id.trim().is_empty() {
        return Err(CliError::InvalidInput(
            "--session-id must not be empty".to_string(),
        ));
    }
    let session_store = crate::adaptor::controller::wiring::build_session_store();
    let session = session_store
        .get_session_review_context(data_dir, session_id)
        .map_err(CliError::Other)?
        .ok_or_else(|| CliError::NotFound(format!("Session not found: {session_id}")))?;
    if session.state == SessionState::Closed {
        return Err(CliError::InvalidInput(format!(
            "Session is closed and cannot be used as a review actor: {session_id}"
        )));
    }
    let backend_id = session.backend_id.clone().ok_or_else(|| {
        CliError::InvalidInput(format!(
            "Session has no backend_id and cannot be used as a review actor: {session_id}"
        ))
    })?;
    let model = session.selected_model.clone().ok_or_else(|| {
        CliError::InvalidInput(format!(
            "Session has no selected_model and cannot be used as a review actor: {session_id}"
        ))
    })?;
    Ok(ReviewActor::agent(
        backend_id,
        model,
        Some(session_id.to_string()),
    ))
    .map(|actor| (actor, session.worktree_path))
}

/// 読み取り専用 review コマンド (`get` / `history`) 向けの軽量 helper。
///
/// `review_actor_and_worktree` は actor 構築のため `backend_id` / `selected_model` /
/// `state != Closed` を必須としているが、Get / History は worktree path しか必要としない。
/// このため過去セッションや actor 用フィールドを持たないセッションでも閲覧できるよう、
/// session 存在チェックと worktree path 取り出しのみを行う。Closed セッションも許可する。
fn review_worktree_from_session(data_dir: &Path, session_id: &str) -> Result<String, CliError> {
    if session_id.trim().is_empty() {
        return Err(CliError::InvalidInput(
            "--session-id must not be empty".to_string(),
        ));
    }
    let session_store = crate::adaptor::controller::wiring::build_session_store();
    let session = session_store
        .get_session_review_context(data_dir, session_id)
        .map_err(CliError::Other)?
        .ok_or_else(|| CliError::NotFound(format!("Session not found: {session_id}")))?;
    Ok(session.worktree_path)
}

fn parse_review_state(value: Option<String>) -> Result<Option<ReviewThreadState>, CliError> {
    match value.as_deref() {
        None | Some("") => Ok(None),
        Some("open") => Ok(Some(ReviewThreadState::Open)),
        Some("resolved") => Ok(Some(ReviewThreadState::Resolved)),
        Some(other) => Err(CliError::InvalidInput(format!(
            "Invalid --state value: {other} (expected: open | resolved)"
        ))),
    }
}

fn parse_optional_author_scope(value: Option<String>) -> Result<Option<AuthorScope>, CliError> {
    match value.as_deref() {
        None | Some("") => Ok(None),
        Some("self") => Ok(Some(AuthorScope::Mine)),
        Some("other") => Ok(Some(AuthorScope::Other)),
        Some(other) => Err(CliError::InvalidInput(format!(
            "Invalid --author value: {other} (expected: self | other)"
        ))),
    }
}

fn parse_optional_unread(value: Option<String>) -> Result<Option<bool>, CliError> {
    match value.as_deref() {
        None | Some("") => Ok(None),
        Some("true") => Ok(Some(true)),
        Some("false") => Ok(Some(false)),
        Some(other) => Err(CliError::InvalidInput(format!(
            "Invalid --unread value: {other} (expected: true | false)"
        ))),
    }
}

fn review_error_to_cli_error(error: ReviewError) -> CliError {
    match error {
        ReviewError::InvalidInput(msg) => CliError::InvalidInput(msg),
        ReviewError::NotFound(msg) => CliError::NotFound(msg),
        ReviewError::AlreadyResolved(msg) | ReviewError::PermissionDenied(msg) => {
            CliError::InvalidInput(msg)
        }
        ReviewError::Io(e) => CliError::Other(e),
        ReviewError::Serialize(e) => CliError::Other(e),
    }
}

fn write_cli_error(error: io::Error) -> CliError {
    CliError::Other(error.to_string())
}

fn render_review_thread(thread: &ReviewThread, json: bool) -> Result<String, CliError> {
    let mut output = Vec::new();
    write_review_thread(&mut output, thread, json)?;
    String::from_utf8(output).map_err(|e| CliError::Other(e.to_string()))
}

fn render_review_thread_list(threads: &[ReviewThread], json: bool) -> Result<String, CliError> {
    let mut output = Vec::new();
    write_review_thread_list(&mut output, threads, json)?;
    String::from_utf8(output).map_err(|e| CliError::Other(e.to_string()))
}

fn render_review_history(events: &[ReviewHistoryEntry], json: bool) -> Result<String, CliError> {
    let mut output = Vec::new();
    write_review_history(&mut output, events, json)?;
    String::from_utf8(output).map_err(|e| CliError::Other(e.to_string()))
}

fn write_review_thread(
    writer: &mut impl Write,
    thread: &ReviewThread,
    json: bool,
) -> Result<(), CliError> {
    if json {
        let text = serde_json::to_string_pretty(&ReviewThreadDto::from(thread))
            .map_err(|e| format!("serialize thread: {e}"))?;
        writeln!(writer, "{text}").map_err(write_cli_error)?;
        return Ok(());
    }
    let location = match (
        thread.target.file_path.as_deref(),
        thread.target.line_number,
        thread.target.end_line,
    ) {
        (Some(file), Some(start), Some(end)) => format!("{file}:L{start}-L{end}"),
        (Some(file), Some(start), None) => format!("{file}:L{start}"),
        (Some(file), None, _) => file.to_string(),
        (None, _, _) => "(general)".to_string(),
    };
    writeln!(
        writer,
        "thread_id: {}\nstate:     {:?}\nauthor:    {}\nlocation:  {}\nupdated:   {}\ncomments:  {}",
        thread.id,
        thread.state,
        thread.author.display_name,
        location,
        thread.updated_at,
        thread.comments.len()
    )
    .map_err(write_cli_error)?;
    if let Some(resolve) = &thread.resolve {
        writeln!(
            writer,
            "resolve:   {} by {} ({})",
            resolve.outcome, resolve.actor.display_name, resolve.summary
        )
        .map_err(write_cli_error)?;
    }
    Ok(())
}

fn write_review_thread_list(
    writer: &mut impl Write,
    threads: &[ReviewThread],
    json: bool,
) -> Result<(), CliError> {
    if json {
        let thread_dtos: Vec<_> = threads.iter().map(ReviewThreadDto::from).collect();
        let text = serde_json::to_string_pretty(&thread_dtos)
            .map_err(|e| format!("serialize threads: {e}"))?;
        writeln!(writer, "{text}").map_err(write_cli_error)?;
    } else if threads.is_empty() {
        writeln!(writer, "(no review threads)").map_err(write_cli_error)?;
    } else {
        writeln!(
            writer,
            "{:<36}  {:<9}  {:<20}  UPDATED",
            "THREAD_ID", "STATE", "AUTHOR"
        )
        .map_err(write_cli_error)?;
        for thread in threads {
            writeln!(
                writer,
                "{:<36}  {:<9}  {:<20}  {}",
                thread.id,
                format!("{:?}", thread.state).to_lowercase(),
                truncate(&thread.author.display_name, 20),
                thread.updated_at
            )
            .map_err(write_cli_error)?;
        }
    }
    Ok(())
}

fn write_review_history(
    writer: &mut impl Write,
    events: &[ReviewHistoryEntry],
    json: bool,
) -> Result<(), CliError> {
    if json {
        let event_dtos: Vec<_> = events.iter().map(ReviewHistoryEntryDto::from).collect();
        let text = serde_json::to_string_pretty(&event_dtos)
            .map_err(|e| format!("serialize history: {e}"))?;
        writeln!(writer, "{text}").map_err(write_cli_error)?;
    } else if events.is_empty() {
        writeln!(writer, "(no review history)").map_err(write_cli_error)?;
    } else {
        for event in events {
            writeln!(writer, "{:?}", event).map_err(write_cli_error)?;
        }
    }
    Ok(())
}

pub(super) fn cmd_review(data_dir: &Path, command: ReviewSubcommand) -> Result<String, CliError> {
    let usecase = build_review_comment_usecase();
    match command {
        ReviewSubcommand::List {
            session_id,
            file,
            state,
            author,
            unread,
            thread_id,
            json,
        } => {
            let (actor, review_worktree) = review_actor_and_worktree(data_dir, &session_id)?;
            let filter = ReviewThreadFilter {
                file,
                state: parse_review_state(state)?,
                author: parse_optional_author_scope(author)?,
                unread: parse_optional_unread(unread)?,
                thread_id,
            };
            let threads = usecase
                .list_threads(data_dir, &review_worktree, Some(filter), actor)
                .map_err(review_error_to_cli_error)?;
            render_review_thread_list(&threads, json)
        }
        ReviewSubcommand::Get {
            thread_id,
            session_id,
            json,
        } => {
            let review_worktree = review_worktree_from_session(data_dir, &session_id)?;
            let thread = usecase
                .get_thread(data_dir, &review_worktree, &thread_id)
                .map_err(review_error_to_cli_error)?;
            render_review_thread(&thread, json)
        }
        ReviewSubcommand::Create {
            session_id,
            content,
            file,
            line,
            end_line,
            json,
        } => {
            let (actor, review_worktree) = review_actor_and_worktree(data_dir, &session_id)?;
            let target = ReviewTarget {
                file_path: file,
                line_number: line,
                end_line,
            };
            let thread = usecase
                .create_thread(data_dir, &review_worktree, actor, target, content)
                .map_err(review_error_to_cli_error)?;
            render_review_thread(&thread, json)
        }
        ReviewSubcommand::Comment {
            thread_id,
            session_id,
            content,
            json,
        } => {
            let (actor, review_worktree) = review_actor_and_worktree(data_dir, &session_id)?;
            let thread = usecase
                .append_comment(data_dir, &review_worktree, actor, &thread_id, content)
                .map_err(review_error_to_cli_error)?;
            render_review_thread(&thread, json)
        }
        ReviewSubcommand::Resolve {
            thread_id,
            session_id,
            outcome,
            summary,
            json,
        } => {
            let (actor, review_worktree) = review_actor_and_worktree(data_dir, &session_id)?;
            let thread = usecase
                .resolve_thread(
                    data_dir,
                    &review_worktree,
                    actor,
                    &thread_id,
                    outcome,
                    summary,
                )
                .map_err(review_error_to_cli_error)?;
            render_review_thread(&thread, json)
        }
        ReviewSubcommand::History {
            thread_id,
            session_id,
            json,
        } => {
            let review_worktree = review_worktree_from_session(data_dir, &session_id)?;
            let events = usecase
                .history(data_dir, &review_worktree, &thread_id)
                .map_err(review_error_to_cli_error)?;
            render_review_history(&events, json)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::super::common::test_support::{
        review_cli_thread, review_history_entries, test_uuid, write_review_config,
        write_review_session,
    };
    use super::super::common::{cli_error_exit_code, cli_error_stderr};
    use super::super::{Cli, TopCommand};
    use super::*;
    use crate::adaptor::gateway::comment::state_file;
    use clap::Parser;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

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
        assert_eq!(value["author"]["model"], "gpt-5");
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
        assert_eq!(value[0]["actor"]["displayName"], "codex/gpt-5");
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

    fn seed_review_thread(data_dir: &Path) -> String {
        let thread_id = test_uuid(42);
        let event_id = test_uuid(44);
        let comment_id = test_uuid(43);
        let path = state_file(data_dir, "/repo");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            format!(
                r#"[
  {{
    "eventType": "thread_created",
    "eventId": "{event_id}",
    "threadId": "{thread_id}",
    "commentId": "{comment_id}",
    "actor": {{
      "kind": "agent",
      "backendId": "codex",
      "model": "gpt-5",
      "sessionId": null,
      "displayName": "codex/gpt-5"
    }},
    "target": {{
      "filePath": "src/main.rs",
      "lineNumber": 3,
      "endLine": 5
    }},
    "content": "Claim",
    "at": 10.0
  }}
]"#
            ),
        )
        .unwrap();
        thread_id
    }

    fn human_line_value<'a>(output: &'a str, prefix: &str) -> &'a str {
        output
            .lines()
            .find_map(|line| line.strip_prefix(prefix))
            .unwrap_or_else(|| panic!("missing human output line with prefix: {prefix}"))
    }

    fn json_string(value: &serde_json::Value, pointer: &str) -> String {
        value
            .pointer(pointer)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| panic!("missing JSON string at pointer: {pointer}"))
            .to_string()
    }

    fn json_literal(value: &serde_json::Value, pointer: &str) -> String {
        value
            .pointer(pointer)
            .unwrap_or_else(|| panic!("missing JSON value at pointer: {pointer}"))
            .to_string()
    }

    #[test]
    fn review_list_get_handler_outputs_match_split_before_golden() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());
        let session_id = "550e8400-e29b-41d4-a716-446655440061".to_string();
        write_review_session(tmp.path(), &session_id, Some("codex"), Some("gpt-5"));
        let thread_id = seed_review_thread(tmp.path());
        let comment_id = test_uuid(43);

        let list_human = cmd_review(
            tmp.path(),
            ReviewSubcommand::List {
                session_id: session_id.clone(),
                file: None,
                state: None,
                author: None,
                unread: None,
                thread_id: Vec::new(),
                json: false,
            },
        )
        .unwrap();
        assert_eq!(
            list_human,
            format!(
                "{:<36}  {:<9}  {:<20}  UPDATED\n{:<36}  {:<9}  {:<20}  {}\n",
                "THREAD_ID", "STATE", "AUTHOR", thread_id, "open", "codex/gpt-5", "10"
            )
        );

        let list_json = cmd_review(
            tmp.path(),
            ReviewSubcommand::List {
                session_id: session_id.clone(),
                file: None,
                state: None,
                author: None,
                unread: None,
                thread_id: Vec::new(),
                json: true,
            },
        )
        .unwrap();
        assert_eq!(
            list_json,
            format!(
                r#"[
  {{
    "id": "{thread_id}",
    "worktreeName": "/repo",
    "author": {{
      "kind": "agent",
      "backendId": "codex",
      "model": "gpt-5",
      "displayName": "codex/gpt-5"
    }},
    "target": {{
      "filePath": "src/main.rs",
      "lineNumber": 3,
      "endLine": 5
    }},
    "state": "open",
    "comments": [
      {{
        "id": "{comment_id}",
        "threadId": "{thread_id}",
        "author": {{
          "kind": "agent",
          "backendId": "codex",
          "model": "gpt-5",
          "displayName": "codex/gpt-5"
        }},
        "content": "Claim",
        "createdAt": 10.0
      }}
    ],
    "resolve": null,
    "createdAt": 10.0,
    "updatedAt": 10.0,
    "version": 1,
    "canResolve": true
  }}
]
"#
            )
        );

        let get_human = cmd_review(
            tmp.path(),
            ReviewSubcommand::Get {
                thread_id: thread_id.clone(),
                session_id: session_id.clone(),
                json: false,
            },
        )
        .unwrap();
        assert_eq!(
            get_human,
            format!(
                "thread_id: {thread_id}\nstate:     Open\nauthor:    codex/gpt-5\nlocation:  src/main.rs:L3-L5\nupdated:   10\ncomments:  1\n"
            )
        );

        let get_json = cmd_review(
            tmp.path(),
            ReviewSubcommand::Get {
                thread_id: thread_id.clone(),
                session_id,
                json: true,
            },
        )
        .unwrap();
        assert_eq!(
            get_json,
            format!(
                r#"{{
  "id": "{thread_id}",
  "worktreeName": "/repo",
  "author": {{
    "kind": "agent",
    "backendId": "codex",
    "model": "gpt-5",
    "displayName": "codex/gpt-5"
  }},
  "target": {{
    "filePath": "src/main.rs",
    "lineNumber": 3,
    "endLine": 5
  }},
  "state": "open",
  "comments": [
    {{
      "id": "{comment_id}",
      "threadId": "{thread_id}",
      "author": {{
        "kind": "agent",
        "backendId": "codex",
        "model": "gpt-5",
        "displayName": "codex/gpt-5"
      }},
      "content": "Claim",
      "createdAt": 10.0
    }}
  ],
  "resolve": null,
  "createdAt": 10.0,
  "updatedAt": 10.0,
  "version": 1,
  "canResolve": true
}}
"#
            )
        );
    }

    #[test]
    fn review_create_handler_outputs_match_split_before_golden() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());
        let session_id = "550e8400-e29b-41d4-a716-446655440062".to_string();
        write_review_session(tmp.path(), &session_id, Some("codex"), Some("gpt-5"));

        let human = cmd_review(
            tmp.path(),
            ReviewSubcommand::Create {
                session_id: session_id.clone(),
                content: "Claim".to_string(),
                file: Some("src/main.rs".to_string()),
                line: Some(3),
                end_line: Some(5),
                json: false,
            },
        )
        .unwrap();
        let human_thread_id = human_line_value(&human, "thread_id: ");
        let human_updated = human_line_value(&human, "updated:   ");
        assert_eq!(
            human,
            format!(
                "thread_id: {human_thread_id}\nstate:     Open\nauthor:    codex/gpt-5\nlocation:  src/main.rs:L3-L5\nupdated:   {human_updated}\ncomments:  1\n"
            )
        );

        let json = cmd_review(
            tmp.path(),
            ReviewSubcommand::Create {
                session_id,
                content: "Claim".to_string(),
                file: Some("src/main.rs".to_string()),
                line: Some(3),
                end_line: Some(5),
                json: true,
            },
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let json_thread_id = json_string(&value, "/id");
        let json_comment_id = json_string(&value, "/comments/0/id");
        let json_created_at = json_literal(&value, "/createdAt");
        let json_updated_at = json_literal(&value, "/updatedAt");
        let json_comment_created_at = json_literal(&value, "/comments/0/createdAt");
        assert_eq!(json_created_at, json_updated_at);
        assert_eq!(json_created_at, json_comment_created_at);
        assert_eq!(
            json,
            format!(
                r#"{{
  "id": "{json_thread_id}",
  "worktreeName": "/repo",
  "author": {{
    "kind": "agent",
    "backendId": "codex",
    "model": "gpt-5",
    "displayName": "codex/gpt-5"
  }},
  "target": {{
    "filePath": "src/main.rs",
    "lineNumber": 3,
    "endLine": 5
  }},
  "state": "open",
  "comments": [
    {{
      "id": "{json_comment_id}",
      "threadId": "{json_thread_id}",
      "author": {{
        "kind": "agent",
        "backendId": "codex",
        "model": "gpt-5",
        "displayName": "codex/gpt-5"
      }},
      "content": "Claim",
      "createdAt": {json_created_at}
    }}
  ],
  "resolve": null,
  "createdAt": {json_created_at},
  "updatedAt": {json_updated_at},
  "version": 1,
  "canResolve": true
}}
"#
            )
        );
    }

    #[test]
    fn review_handler_error_stderr_and_exit_codes_match_split_before_golden() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());
        let session_id = "550e8400-e29b-41d4-a716-446655440063".to_string();
        write_review_session(tmp.path(), &session_id, Some("codex"), Some("gpt-5"));

        let invalid_state = cmd_review(
            tmp.path(),
            ReviewSubcommand::List {
                session_id: session_id.clone(),
                file: None,
                state: Some("paused".to_string()),
                author: None,
                unread: None,
                thread_id: Vec::new(),
                json: false,
            },
        )
        .unwrap_err();
        assert_eq!(
            cli_error_stderr(&invalid_state),
            "error: Invalid --state value: paused (expected: open | resolved)"
        );
        assert_eq!(cli_error_exit_code(&invalid_state), 2);

        let missing_thread_id = test_uuid(99);
        let missing_thread = cmd_review(
            tmp.path(),
            ReviewSubcommand::Get {
                thread_id: missing_thread_id.clone(),
                session_id: session_id.clone(),
                json: false,
            },
        )
        .unwrap_err();
        assert_eq!(
            cli_error_stderr(&missing_thread),
            format!("Review thread not found: {missing_thread_id}")
        );
        assert_eq!(cli_error_exit_code(&missing_thread), 4);

        let invalid_target = cmd_review(
            tmp.path(),
            ReviewSubcommand::Create {
                session_id: session_id.clone(),
                content: "Bad target".to_string(),
                file: Some("../secret".to_string()),
                line: Some(1),
                end_line: None,
                json: true,
            },
        )
        .unwrap_err();
        assert_eq!(
            cli_error_stderr(&invalid_target),
            "error: file_path must not contain root, prefix, '.', or '..' components"
        );
        assert_eq!(cli_error_exit_code(&invalid_target), 2);

        crate::test_support::build_session_store()
            .set_session_state(tmp.path(), &session_id, SessionState::Closed)
            .unwrap();
        let closed_session = cmd_review(
            tmp.path(),
            ReviewSubcommand::Create {
                session_id: session_id.clone(),
                content: "Claim".to_string(),
                file: None,
                line: None,
                end_line: None,
                json: false,
            },
        )
        .unwrap_err();
        assert_eq!(
            cli_error_stderr(&closed_session),
            format!("error: Session is closed and cannot be used as a review actor: {session_id}")
        );
        assert_eq!(cli_error_exit_code(&closed_session), 2);
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
    fn review_actor_resolves_backend_and_model_from_session_id() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());
        let session_id = uuid::Uuid::new_v4().to_string();
        write_review_session(tmp.path(), &session_id, Some("codex"), Some("gpt-5"));

        let actor = review_actor(tmp.path(), &session_id).unwrap();

        assert_eq!(actor.backend_id.as_deref(), Some("codex"));
        assert_eq!(actor.model.as_deref(), Some("gpt-5"));
        assert_eq!(actor.session_id.as_deref(), Some(session_id.as_str()));
    }

    #[test]
    fn review_actor_uses_saved_backend_model_without_catalog_validation() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());

        let missing = review_actor(tmp.path(), &uuid::Uuid::new_v4().to_string());
        assert!(matches!(missing, Err(CliError::NotFound(_))));

        let session_id = uuid::Uuid::new_v4().to_string();
        write_review_session(tmp.path(), &session_id, Some("codex"), Some("fake-model"));
        let actor = review_actor(tmp.path(), &session_id).unwrap();
        assert_eq!(actor.backend_id.as_deref(), Some("codex"));
        assert_eq!(actor.model.as_deref(), Some("fake-model"));

        let missing_backend_id = uuid::Uuid::new_v4().to_string();
        write_review_session(tmp.path(), &missing_backend_id, None, Some("gpt-5"));
        assert!(matches!(
            review_actor(tmp.path(), &missing_backend_id),
            Err(CliError::InvalidInput(_))
        ));

        let missing_model_id = uuid::Uuid::new_v4().to_string();
        write_review_session(tmp.path(), &missing_model_id, Some("codex"), None);
        assert!(matches!(
            review_actor(tmp.path(), &missing_model_id),
            Err(CliError::InvalidInput(_))
        ));
    }

    #[test]
    fn review_actor_treats_legacy_flat_session_as_not_found() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());
        let session_id = uuid::Uuid::new_v4().to_string();
        let sessions_dir = tmp.path().join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        fs::write(
            sessions_dir.join(format!("{session_id}.json")),
            format!(
                r#"{{
                    "id":"{session_id}",
                    "worktreePath":"/repo",
                    "messages":[
                        {{"id":"m1","role":"human","content":["not","a","string"],"timestamp":1000.0}}
                    ],
                    "state":"active",
                    "createdAt":1000.0,
                    "updatedAt":1001.0,
                    "permissionMode":"edit",
                    "selectedModel":"gpt-5",
                    "backendId":"codex",
                    "workflowStepSession":false
                }}"#
            ),
        )
        .unwrap();

        let err = review_actor(tmp.path(), &session_id).unwrap_err();

        assert!(matches!(err, CliError::NotFound(_)));
    }

    #[test]
    fn review_actor_treats_legacy_sidecar_session_as_not_found() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());
        let session_id = uuid::Uuid::new_v4().to_string();
        let sessions_dir = tmp.path().join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        fs::write(
            sessions_dir.join(format!("{session_id}.meta.json")),
            format!(
                r#"{{
                    "id":"{session_id}",
                    "worktreePath":"/repo",
                    "state":"active",
                    "createdAt":1000.0,
                    "updatedAt":1001.0,
                    "permissionMode":"edit",
                    "selectedModel":"gpt-5",
                    "backendId":"codex",
                    "workflowStepSession":false,
                    "firstMessagePreview":"",
                    "messageCount":0,
                    "bodyFormatVersion":1
                }}"#
            ),
        )
        .unwrap();

        let err = review_actor(tmp.path(), &session_id).unwrap_err();

        assert!(matches!(err, CliError::NotFound(_)));
    }

    #[test]
    fn review_actor_and_worktree_rejects_empty_session_id() {
        let tmp = TempDir::new().unwrap();

        for session_id in ["", " "] {
            let err = review_actor_and_worktree(tmp.path(), session_id).unwrap_err();
            assert!(matches!(err, CliError::InvalidInput(_)));
        }
    }

    #[test]
    fn review_worktree_from_session_rejects_empty_session_id() {
        let tmp = TempDir::new().unwrap();

        for session_id in ["", " "] {
            let err = review_worktree_from_session(tmp.path(), session_id).unwrap_err();
            assert!(matches!(err, CliError::InvalidInput(_)));
        }
    }

    #[test]
    fn review_worktree_resolution_allows_closed_session_without_actor_fields() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());
        let session_id = uuid::Uuid::new_v4().to_string();
        write_review_session(tmp.path(), &session_id, None, None);
        crate::test_support::build_session_store()
            .set_session_state(tmp.path(), &session_id, SessionState::Closed)
            .unwrap();

        let worktree = review_worktree_from_session(tmp.path(), &session_id).unwrap();

        assert_eq!(worktree, "/repo");
    }

    #[test]
    fn review_cli_rejects_mutation_for_closed_session() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());
        let session_id = uuid::Uuid::new_v4().to_string();
        write_review_session(tmp.path(), &session_id, Some("codex"), Some("gpt-5"));

        crate::test_support::build_session_store()
            .set_session_state(tmp.path(), &session_id, SessionState::Closed)
            .unwrap();
        let closed = cmd_review(
            tmp.path(),
            ReviewSubcommand::Create {
                session_id,
                content: "Claim".to_string(),
                file: None,
                line: None,
                end_line: None,
                json: true,
            },
        );
        match closed {
            Err(CliError::InvalidInput(msg)) => assert!(msg.contains("Session is closed")),
            other => panic!("expected closed session rejection, got {other:?}"),
        }
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

    /// `--worktree` フラグは spec design.md L37 で「提供しない」と明示されたため
    /// 受け付けない（session_id から worktree を解決する）。
    #[test]
    fn review_cli_parser_rejects_worktree_flag() {
        let result = Cli::try_parse_from([
            "releash",
            "review",
            "create",
            "--worktree",
            "/repo",
            "--session-id",
            "session-1",
            "--content",
            "Claim",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn cmd_review_create_list_get_and_json_mode_use_session_worktree_key() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());
        let session_id = uuid::Uuid::new_v4().to_string();
        write_review_session(tmp.path(), &session_id, Some("codex"), Some("gpt-5"));

        cmd_review(
            tmp.path(),
            ReviewSubcommand::Create {
                session_id: session_id.clone(),
                content: "Claim".to_string(),
                file: None,
                line: None,
                end_line: None,
                json: true,
            },
        )
        .unwrap();

        let usecase = build_review_comment_usecase();
        let threads = usecase
            .list_threads(tmp.path(), "/repo", None, ReviewActor::human())
            .unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].worktree_name, "/repo");
        let json = serde_json::to_string(&ReviewThreadDto::from(&threads[0])).unwrap();
        assert!(!json.contains("sessionId"));

        cmd_review(
            tmp.path(),
            ReviewSubcommand::List {
                session_id: session_id.clone(),
                file: None,
                state: Some("open".to_string()),
                author: None,
                unread: None,
                thread_id: Vec::new(),
                json: true,
            },
        )
        .unwrap();
        cmd_review(
            tmp.path(),
            ReviewSubcommand::Get {
                thread_id: threads[0].id.clone(),
                session_id,
                json: true,
            },
        )
        .unwrap();
    }

    #[test]
    fn cmd_review_comment_resolve_history_and_rejections_use_domain_reasons() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());
        let owner_session = uuid::Uuid::new_v4().to_string();
        let other_session = uuid::Uuid::new_v4().to_string();
        write_review_session(tmp.path(), &owner_session, Some("codex"), Some("gpt-5"));
        write_review_session(tmp.path(), &other_session, Some("claude"), Some("opus"));

        cmd_review(
            tmp.path(),
            ReviewSubcommand::Create {
                session_id: owner_session.clone(),
                content: "Claim".to_string(),
                file: Some("src/main.rs".to_string()),
                line: Some(3),
                end_line: Some(5),
                json: true,
            },
        )
        .unwrap();
        let usecase = build_review_comment_usecase();
        let thread_id = usecase
            .list_threads(tmp.path(), "/repo", None, ReviewActor::human())
            .unwrap()[0]
            .id
            .clone();

        cmd_review(
            tmp.path(),
            ReviewSubcommand::Comment {
                thread_id: thread_id.clone(),
                session_id: owner_session.clone(),
                content: "Follow-up".to_string(),
                json: true,
            },
        )
        .unwrap();
        cmd_review(
            tmp.path(),
            ReviewSubcommand::Comment {
                thread_id: thread_id.clone(),
                session_id: owner_session.clone(),
                content: "Another follow-up".to_string(),
                json: true,
            },
        )
        .unwrap();
        cmd_review(
            tmp.path(),
            ReviewSubcommand::History {
                thread_id: thread_id.clone(),
                session_id: owner_session.clone(),
                json: true,
            },
        )
        .unwrap();

        // 別 backend/model session からの Resolve も participant identity に依らず成功する
        // (spec issues-1022: Resolve 権限は participant 識別に依存しない)。
        cmd_review(
            tmp.path(),
            ReviewSubcommand::Resolve {
                thread_id: thread_id.clone(),
                session_id: other_session,
                outcome: "accepted".to_string(),
                summary: "non-owner resolve".to_string(),
                json: true,
            },
        )
        .unwrap();
        // resolved 後の Resolve / Comment 追記は state により拒否される。
        let rejected_after_resolve = cmd_review(
            tmp.path(),
            ReviewSubcommand::Resolve {
                thread_id: thread_id.clone(),
                session_id: owner_session.clone(),
                outcome: "accepted".to_string(),
                summary: "second resolve".to_string(),
                json: true,
            },
        );
        match rejected_after_resolve {
            Err(CliError::InvalidInput(msg)) => assert!(msg.contains("already resolved")),
            other => panic!("expected resolved rejection, got {other:?}"),
        }
        let rejected_late_comment = cmd_review(
            tmp.path(),
            ReviewSubcommand::Comment {
                thread_id: thread_id.clone(),
                session_id: owner_session.clone(),
                content: "late".to_string(),
                json: true,
            },
        );
        match rejected_late_comment {
            Err(CliError::InvalidInput(msg)) => assert!(msg.contains("already resolved")),
            other => panic!("expected resolved rejection, got {other:?}"),
        }

        let missing_history = cmd_review(
            tmp.path(),
            ReviewSubcommand::History {
                thread_id: "missing-thread".to_string(),
                session_id: owner_session.clone(),
                json: true,
            },
        );
        assert!(matches!(missing_history, Err(CliError::NotFound(_))));

        let invalid_target = cmd_review(
            tmp.path(),
            ReviewSubcommand::Create {
                session_id: owner_session,
                content: "Bad target".to_string(),
                file: Some("../secret".to_string()),
                line: Some(1),
                end_line: None,
                json: true,
            },
        );
        assert!(matches!(invalid_target, Err(CliError::InvalidInput(_))));
    }
}
