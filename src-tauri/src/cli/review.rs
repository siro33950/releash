use crate::usecase::agent_session::AgentSessionQueryService;
use std::io::{self, Write};
use std::path::Path;

use clap::Subcommand;

use super::common::{truncate, CliError};
use crate::adaptor::controller::wiring::build_review_comment_usecase;
use crate::adaptor::presenter::comment::{ReviewHistoryEntryDto, ReviewThreadDto};
use crate::domain::comment::{
    AuthorScope, ReviewActor, ReviewError, ReviewHistoryEntry, ReviewTarget, ReviewThread,
    ReviewThreadFilter, ReviewThreadState,
};
use crate::usecase::agent_session::{AgentSessionItemDto, AgentSessionLifecycleDto};
use crate::usecase::provider_dto::AgentSessionProviderDto;

enum ReviewSessionContext {
    Provider(AgentSessionItemDto),
}

#[derive(Subcommand, Debug)]
pub enum ReviewSubcommand {
    /// review Thread 一覧を表示する。
    List {
        #[arg(long)]
        session_id: Option<String>,
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

#[cfg(any(test, feature = "test-support"))]
pub async fn review_actor(data_dir: &Path, session_id: &str) -> Result<ReviewActor, CliError> {
    review_actor_and_worktree(data_dir, session_id)
        .await
        .map(|(actor, _)| actor)
}

pub async fn review_actor_and_worktree(
    data_dir: &Path,
    session_id: &str,
) -> Result<(ReviewActor, String), CliError> {
    let session = required_review_session_context(data_dir, session_id).await?;
    review_actor_and_worktree_from_context(session_id, session)
}

fn review_actor_and_worktree_from_context(
    session_id: &str,
    session: ReviewSessionContext,
) -> Result<(ReviewActor, String), CliError> {
    match session {
        ReviewSessionContext::Provider(session) => {
            if session.lifecycle != AgentSessionLifecycleDto::Open {
                return Err(CliError::InvalidInput(format!(
                    "Session is not open and cannot be used as a review actor: {session_id}"
                )));
            }
            let provider = match session.provider {
                AgentSessionProviderDto::Claude => "claude",
                AgentSessionProviderDto::Codex => "codex",
            };
            Ok((
                ReviewActor::provider_agent(provider.to_string(), Some(session_id.to_string())),
                session.workspace_worktree_path,
            ))
        }
    }
}

async fn review_actor_and_worktree_for_read(
    data_dir: &Path,
    session_id: &str,
) -> Result<(ReviewActor, String), CliError> {
    let session = required_review_session_context(data_dir, session_id).await?;
    Ok(match session {
        ReviewSessionContext::Provider(session) => {
            let provider = match session.provider {
                AgentSessionProviderDto::Claude => "claude",
                AgentSessionProviderDto::Codex => "codex",
            };
            (
                ReviewActor::provider_agent(provider.to_string(), Some(session_id.to_string())),
                session.workspace_worktree_path,
            )
        }
    })
}

/// 読み取り専用 review コマンド (`list` / `get` / `history`) 向けの軽量 helper。
///
/// List / Get / HistoryはAgentSessionのlifecycleに関係なくworktreeだけを解決する。
pub async fn review_worktree_from_session(
    data_dir: &Path,
    session_id: &str,
) -> Result<String, CliError> {
    let session = required_review_session_context(data_dir, session_id).await?;
    Ok(match session {
        ReviewSessionContext::Provider(session) => session.workspace_worktree_path,
    })
}

async fn required_review_session_context(
    data_dir: &Path,
    session_id: &str,
) -> Result<ReviewSessionContext, CliError> {
    if session_id.trim().is_empty() {
        return Err(CliError::InvalidInput(
            "--session-id must not be empty".to_string(),
        ));
    }
    review_session_context(data_dir, session_id)
        .await?
        .ok_or_else(|| CliError::NotFound(format!("Session not found: {session_id}")))
}

pub async fn review_list_actor_and_worktree(
    data_dir: &Path,
    session_id: Option<&str>,
    worktree_path: Option<&str>,
    actor_required: bool,
) -> Result<(ReviewActor, String), CliError> {
    if actor_required {
        let session_id = session_id.ok_or_else(|| {
            CliError::InvalidInput(
                "--session-id is required when --author or --unread is specified".to_string(),
            )
        })?;
        return review_actor_and_worktree_for_read(data_dir, session_id).await;
    }
    if let Some(session_id) = session_id {
        return Ok((
            ReviewActor::human(),
            review_worktree_from_session(data_dir, session_id).await?,
        ));
    }
    let worktree_path = worktree_path
        .filter(|path| !path.trim().is_empty())
        .ok_or_else(|| {
            CliError::InvalidInput(
                "review list requires --session-id or RELEASH_WORKTREE_PATH".to_string(),
            )
        })?;
    Ok((
        ReviewActor::human(),
        review_workspace_worktree(data_dir, worktree_path).await?,
    ))
}

async fn review_session_context(
    data_dir: &Path,
    session_id: &str,
) -> Result<Option<ReviewSessionContext>, CliError> {
    let store_path = crate::adaptor::gateway::local_event_store::layout::StoreLayout::new(data_dir)
        .database_path();
    if !store_path
        .try_exists()
        .map_err(|error| CliError::Other(format!("AgentSession store lookup failed: {error}")))?
    {
        return Ok(None);
    }
    let provider = crate::adaptor::controller::wiring::build_canonical_agent_session_query(
        data_dir.to_path_buf(),
    )?;
    let context = provider
        .get(session_id)
        .await
        .map_err(|error| CliError::Other(format!("AgentSession query failed: {error:?}")))?;
    Ok(context.map(ReviewSessionContext::Provider))
}

async fn review_workspace_worktree(data_dir: &Path, path: &str) -> Result<String, CliError> {
    crate::adaptor::controller::wiring::build_workspace_worktree_path_usecase(data_dir)
        .workspace_worktree_path(path)
        .await
        .map_err(|error| CliError::Other(error.to_string()))
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
        ReviewError::SessionNotOpen(id) => {
            CliError::InvalidInput(format!("Session is not open: {id}"))
        }
        ReviewError::Technical(error) => CliError::Other(error.to_string()),
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

pub async fn cmd_review(data_dir: &Path, command: ReviewSubcommand) -> Result<String, CliError> {
    let usecase = build_review_comment_usecase(
        crate::adaptor::controller::wiring::build_review_context(data_dir),
    );
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
            let state = parse_review_state(state)?;
            let author = parse_optional_author_scope(author)?;
            let unread = parse_optional_unread(unread)?;
            let actor_required = author.is_some() || unread.is_some();
            let worktree_path = std::env::var("RELEASH_WORKTREE_PATH").ok();
            let (actor, review_worktree) = review_list_actor_and_worktree(
                data_dir,
                session_id.as_deref(),
                worktree_path.as_deref(),
                actor_required,
            )
            .await?;
            let filter = ReviewThreadFilter {
                file,
                state,
                author,
                unread,
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
            let review_worktree = review_worktree_from_session(data_dir, &session_id).await?;
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
            let (actor, review_worktree) = review_actor_and_worktree(data_dir, &session_id).await?;
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
            let (actor, review_worktree) = review_actor_and_worktree(data_dir, &session_id).await?;
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
            let (actor, review_worktree) = review_actor_and_worktree(data_dir, &session_id).await?;
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
            let review_worktree = review_worktree_from_session(data_dir, &session_id).await?;
            let events = usecase
                .history(data_dir, &review_worktree, &thread_id)
                .map_err(review_error_to_cli_error)?;
            render_review_history(&events, json)
        }
    }
}

#[cfg(test)]
#[path = "review_test.rs"]
mod review_tests;
