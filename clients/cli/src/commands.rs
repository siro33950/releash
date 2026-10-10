use crate::client::{connect, snapshot, to_rpc, to_wire, Client};
use crate::output::OutputSubcommand as Output;
use crate::review::ReviewSubcommand as Review;
use crate::workflow::WorkflowSubcommand as Workflow;
use crate::{rpc, wire};
use crate::{HookCommand, HookProvider, TopCommand};
use connectrpc::{ConnectError, ErrorCode};
use serde_json::{json, Value};
use std::io::Read;
use std::path::Path;

pub fn output_options(command: &TopCommand) -> (bool, Option<&'static str>) {
    let json = match command {
        TopCommand::Server {
            command: crate::server::ServerSubcommand::Start { json },
        } => return (*json, Some(crate::client::startup_guidance())),
        TopCommand::Status { json } => *json,
        TopCommand::Workflow { command } => match command {
            Workflow::Status { json, .. } | Workflow::Diagnostics { json, .. } => *json,
            Workflow::Output {
                command: Output::Get { json, .. },
            } => *json,
            _ => false,
        },
        TopCommand::Review { command } => match command {
            Review::List { json, .. }
            | Review::Get { json, .. }
            | Review::Create { json, .. }
            | Review::Comment { json, .. }
            | Review::Resolve { json, .. }
            | Review::History { json, .. } => *json,
        },
        _ => false,
    };
    (json, None)
}

fn display<M: prost::Message>(name: &str, value: &M) -> Result<Value, ConnectError> {
    crate::json::from_message(&format!("releash.client.v1.{name}"), value)
        .map_err(ConnectError::internal)
}

#[derive(Clone, Copy)]
enum View {
    Execution,
    Threads,
    Thread,
    History,
}

fn json_text(value: &Value) -> Result<String, ConnectError> {
    serde_json::to_string_pretty(value)
        .map(|s| format!("{s}\n"))
        .map_err(|e| ConnectError::internal(e.to_string()))
}

fn render(value: Value, json: bool, view: View) -> Result<String, ConnectError> {
    if json {
        return json_text(&value);
    }
    match view {
        View::Execution => Ok(format!("execution_id:  {}\nworkflow:      {}\nstatus:        {}\ncurrent_node:  {}\nupdated_at:    {}\ninput_tokens:  {}\noutput_tokens: {}\n",
            text(&value["id"]), text(&value["workflowName"]), text(&value["status"]), text(&value["currentNode"]),
            value["updatedAt"], value["totalTokenUsage"]["inputTokens"], value["totalTokenUsage"]["outputTokens"])),
        View::Threads => {
            let items = value.as_array().ok_or_else(|| ConnectError::internal("Invalid threads response"))?;
            if items.is_empty() { return Ok("(no review threads)\n".into()); }
            let mut result = format!("{:<36}  {:<9}  {:<20}  UPDATED\n", "THREAD_ID", "STATE", "AUTHOR");
            for item in items { result.push_str(&format!("{:<36}  {:<9}  {:<20}  {}\n", text(&item["id"]), text(&item["state"]), text(&item["author"]["displayName"]), item["updatedAt"])); }
            Ok(result)
        }
        View::Thread => {
            let target = &value["target"];
            let mut location = target["filePath"].as_str().unwrap_or("(general)").to_owned();
            if let Some(line) = target["lineNumber"].as_u64() { location.push_str(&format!(":L{line}"));
                if let Some(end) = target["endLine"].as_u64() { location.push_str(&format!("-L{end}")); } }
            let mut result = format!("thread_id: {}\nstate:     {}\nauthor:    {}\nlocation:  {}\nupdated:   {}\ncomments:  {}\n",
                text(&value["id"]), text(&value["state"]), text(&value["author"]["displayName"]), location, value["updatedAt"], value["comments"].as_array().map_or(0, Vec::len));
            if value["resolve"].is_object() { let r = &value["resolve"]; result.push_str(&format!("resolve:   {} by {} ({})\n", text(&r["outcome"]), text(&r["actor"]["displayName"]), text(&r["summary"]))); }
            Ok(result)
        }
        View::History => {
            let entries = value.as_array().ok_or_else(|| ConnectError::internal("Invalid history response"))?;
            if entries.is_empty() { return Ok("(no review history)\n".into()); }
            Ok(entries.iter().map(|e| format!("{} {} by {}: {}\n", e["at"], text(&e["kind"]), text(&e["actor"]["displayName"]), e["content"].as_str().or_else(|| e["summary"].as_str()).unwrap_or(""))).collect())
        }
    }
}
fn text(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}
fn absent<T>(value: Option<T>) -> Result<T, ConnectError> {
    value.ok_or_else(|| ConnectError::new(ErrorCode::NotFound, "Requested state was not found"))
}

pub async fn run(dir: &Path, command: TopCommand) -> Result<(String, i32), ConnectError> {
    if let TopCommand::Hook {
        command: HookCommand::Receive { provider },
    } = command
    {
        return hook(dir, provider).await.map(|()| (String::new(), 0));
    }
    match command {
        TopCommand::Status { json } => {
            return crate::server::status(dir, json).await.map(|s| (s, 0))
        }
        TopCommand::Server { command } => {
            return crate::server::run(dir, command).await.map(|s| (s, 0))
        }
        _ => {}
    }
    let client = connect(dir, None).await?;
    let output = match command {
        TopCommand::Workflow { command } => match command {
            Workflow::Status { execution_id, json } => {
                let payload = snapshot(&client, "workflow-execution", vec![execution_id]).await?;
                let Some(wire::state_payload::Value::WorkflowExecution(value)) = payload.value
                else {
                    return Err(ConnectError::internal("Invalid execution response"));
                };
                render(
                    display("WorkflowExecutionView", &absent(value.value)?)?,
                    json,
                    View::Execution,
                )?
            }
            Workflow::Diagnostics { dir, json } => {
                let dir = dir
                    .map(|path| {
                        if path.is_absolute() {
                            Ok(path)
                        } else {
                            std::env::current_dir()
                                .map(|cwd| cwd.join(path))
                                .map_err(|e| ConnectError::internal(e.to_string()))
                        }
                    })
                    .transpose()?
                    .map(|path| {
                        path.into_os_string().into_string().map_err(|_| {
                            ConnectError::new(ErrorCode::InvalidArgument, "Directory must be UTF-8")
                        })
                    })
                    .transpose()?;
                let response = client
                    .diagnose_workflow_directory(to_rpc::<rpc::DiagnoseWorkflowDirectoryRequest>(
                        &wire::DiagnoseWorkflowDirectoryRequest { dir },
                    )?)
                    .await?;
                let response: wire::DiagnoseWorkflowDirectoryResponse =
                    to_wire(&response.into_owned())?;
                let report = absent(response.report)?;
                let value = display("DiagnosticReport", &report)?;
                let code = if report.items.as_ref().is_some_and(|items| {
                    items.items.iter().any(|item| {
                        item.severity.as_ref().and_then(|s| s.value)
                            == Some(wire::severity::Value::Error as i32)
                    })
                }) {
                    3
                } else {
                    0
                };
                let output = if json {
                    json_text(&value)?
                } else {
                    diagnostics(&value)
                };
                return Ok((output, code));
            }
            Workflow::Output {
                command:
                    Output::Get {
                        execution_id,
                        node,
                        json,
                    },
            } => {
                let payload =
                    snapshot(&client, "workflow-output", vec![execution_id, node.clone()]).await?;
                let Some(wire::state_payload::Value::WorkflowOutput(value)) = payload.value else {
                    return Err(ConnectError::internal("Invalid output response"));
                };
                let mut value = display("WorkflowOutputView", &absent(value.value)?)?;
                if let Some(object) = value.as_object_mut() {
                    if let Some(artifact) = object.remove("structured_output") {
                        object.insert("artifact".into(), artifact);
                    }
                }
                if json {
                    json_text(&value)?
                } else if value["status"] == "not_submitted" {
                    format!("not_submitted: node={node}\n")
                } else {
                    let mut output = format!(
                        "submitted: node={node} contract={}\n",
                        value["contract"].as_str().unwrap_or("none")
                    );
                    for field in ["submitted_at", "request_id", "timestamp"] {
                        if let Some(v) = value.get(field) {
                            output.push_str(&format!(
                                "{field}: {}\n",
                                v.as_str()
                                    .map(str::to_owned)
                                    .unwrap_or_else(|| v.to_string())
                            ));
                        }
                    }
                    output.push_str(&format!("artifact:\n{}", json_text(&value["artifact"])?));
                    output
                }
            }
            Workflow::Output {
                command:
                    Output::Submit {
                        node_execution,
                        contract,
                        json,
                        file,
                    },
            } => {
                let artifact = if let Some(contract) = &contract {
                    let raw = match (json, file) {
                        (Some(raw), _) => raw,
                        (_, Some(path)) => std::fs::read_to_string(path).map_err(|e| {
                            ConnectError::new(ErrorCode::InvalidArgument, e.to_string())
                        })?,
                        _ => {
                            return Err(ConnectError::new(
                                ErrorCode::InvalidArgument,
                                "Artifact input is required",
                            ))
                        }
                    };
                    let value: Value = serde_json::from_str(&raw).map_err(|e| {
                        ConnectError::new(ErrorCode::InvalidArgument, e.to_string())
                    })?;
                    Some(wire::WorkflowSubmitArtifactInput {
                        contract: Some(contract.clone()),
                        value: Some(value.try_into().map_err(ConnectError::internal)?),
                    })
                } else {
                    None
                };
                client
                    .workflow_submit_output(to_rpc::<rpc::WorkflowSubmitOutputRequest>(
                        &wire::WorkflowSubmitOutputRequest {
                            node_execution_id: Some(node_execution.clone()),
                            artifact,
                        },
                    )?)
                    .await?;
                format!(
                    "submitted: node_execution_id={node_execution}{}\n",
                    contract.map(|s| format!(" type={s}")).unwrap_or_default()
                )
            }
        },
        TopCommand::Review { command } => review(&client, command).await?,
        _ => unreachable!(),
    };
    Ok((output, 0))
}

fn diagnostics(report: &Value) -> String {
    let mut output = String::new();
    let mut errors = 0;
    let mut infos = 0;
    if let Some(items) = report["items"].as_array() {
        for item in items {
            let severity = text(&item["severity"]);
            if severity == "error" {
                errors += 1;
            } else {
                infos += 1;
            }
            let span = &item["span"];
            let location = if span.is_object() {
                format!(
                    " {}{}:{}",
                    span["source"]
                        .as_str()
                        .map(|s| format!("{s}:"))
                        .unwrap_or_default(),
                    span["start_line"],
                    span["start_col"]
                )
            } else {
                String::new()
            };
            let mut targets = Vec::new();
            for (key, label) in [
                ("workflow_name", "workflow"),
                ("node_name", "node"),
                ("field", "field"),
            ] {
                if let Some(s) = item[key].as_str() {
                    targets.push(format!("{label}={s}"));
                }
            }
            if let (Some(kind), Some(key)) =
                (item["facet_kind"].as_str(), item["facet_key"].as_str())
            {
                targets.push(format!("facet={kind}/{key}"));
            }
            let target = if targets.is_empty() {
                String::new()
            } else {
                format!(" [{}]", targets.join(", "))
            };
            output.push_str(&format!(
                "{severity} {}{location}{target}: {}\n",
                text(&item["code"]),
                text(&item["message"])
            ));
        }
    }
    output.push_str(&format!("\n{errors} error, {infos} info\n"));
    output
}

pub(crate) fn review_filters(
    file: Option<String>,
    state: Option<String>,
    author: Option<String>,
    unread: Option<String>,
    mut threads: Vec<String>,
) -> Vec<String> {
    let mut args = Vec::new();
    for (key, value) in [
        ("file", file),
        ("state", state),
        ("author", author),
        ("unread", unread),
    ] {
        if let Some(value) = value {
            args.push(format!("{key}={value}"));
        }
    }
    threads.sort();
    threads.dedup();
    args.extend(threads.into_iter().map(|id| format!("thread={id}")));
    args
}

async fn review(client: &Client, command: Review) -> Result<String, ConnectError> {
    match command {
        Review::List {
            session_id,
            file,
            state,
            author,
            unread,
            thread_id,
            json,
        } => {
            let (target, id) = match session_id {
                Some(id) => ("review-session-threads", id),
                None => (
                    "review-worktree-threads",
                    std::env::var("RELEASH_WORKTREE_PATH").map_err(|_| {
                        ConnectError::new(
                            ErrorCode::InvalidArgument,
                            "RELEASH_WORKTREE_PATH is required",
                        )
                    })?,
                ),
            };
            let mut args = vec![id];
            args.extend(review_filters(file, state, author, unread, thread_id));
            let payload = snapshot(client, target, args).await?;
            let threads = match payload.value {
                Some(wire::state_payload::Value::ReviewSessionThreads(value)) => {
                    absent(value.value)?
                }
                Some(wire::state_payload::Value::ReviewThreads(value)) => value,
                _ => return Err(ConnectError::internal("Invalid threads response")),
            };
            render(
                display("ListReviewThreadDto", &threads)?,
                json,
                View::Threads,
            )
        }
        Review::Get {
            thread_id,
            session_id,
            json,
        } => {
            let payload =
                snapshot(client, "review-session-thread", vec![session_id, thread_id]).await?;
            let Some(wire::state_payload::Value::ReviewSessionThread(value)) = payload.value else {
                return Err(ConnectError::internal("Invalid thread response"));
            };
            render(
                display("ReviewThreadDto", &absent(value.value)?)?,
                json,
                View::Thread,
            )
        }
        Review::History {
            thread_id,
            session_id,
            json,
        } => {
            let payload = snapshot(
                client,
                "review-session-thread-history",
                vec![session_id, thread_id],
            )
            .await?;
            let Some(wire::state_payload::Value::ReviewSessionThreadHistory(value)) = payload.value
            else {
                return Err(ConnectError::internal("Invalid history response"));
            };
            let entries = absent(value.value)?
                .items
                .into_iter()
                .map(history_entry)
                .collect::<Result<Vec<_>, _>>()?;
            render(Value::Array(entries), json, View::History)
        }
        Review::Create {
            session_id,
            content,
            file,
            line,
            end_line,
            json,
        } => {
            let response = client
                .create_session_review_thread(to_rpc::<rpc::CreateSessionReviewThreadRequest>(
                    &wire::CreateSessionReviewThreadRequest {
                        session_id: Some(session_id),
                        content: Some(content),
                        file_path: file,
                        line_number: line,
                        end_line,
                    },
                )?)
                .await?;
            let value: wire::CreateSessionReviewThreadResponse = to_wire(&response.into_owned())?;
            render(
                display("ReviewThreadDto", &absent(value.thread)?)?,
                json,
                View::Thread,
            )
        }
        Review::Comment {
            thread_id,
            session_id,
            content,
            json,
        } => {
            let response = client
                .append_session_review_comment(to_rpc::<rpc::AppendSessionReviewCommentRequest>(
                    &wire::AppendSessionReviewCommentRequest {
                        session_id: Some(session_id),
                        thread_id: Some(thread_id),
                        content: Some(content),
                    },
                )?)
                .await?;
            let value: wire::AppendSessionReviewCommentResponse = to_wire(&response.into_owned())?;
            render(
                display("ReviewThreadDto", &absent(value.thread)?)?,
                json,
                View::Thread,
            )
        }
        Review::Resolve {
            thread_id,
            session_id,
            outcome,
            summary,
            json,
        } => {
            let response = client
                .resolve_session_review_thread(to_rpc::<rpc::ResolveSessionReviewThreadRequest>(
                    &wire::ResolveSessionReviewThreadRequest {
                        session_id: Some(session_id),
                        thread_id: Some(thread_id),
                        outcome: Some(outcome),
                        summary: Some(summary),
                    },
                )?)
                .await?;
            let value: wire::ResolveSessionReviewThreadResponse = to_wire(&response.into_owned())?;
            render(
                display("ReviewThreadDto", &absent(value.thread)?)?,
                json,
                View::Thread,
            )
        }
    }
}

fn history_entry(entry: wire::ReviewHistoryEntryDto) -> Result<Value, ConnectError> {
    use wire::review_history_entry_dto::Entry;
    let actor = |value| absent(value).and_then(|v| display("ReviewActorWireDto", &v));
    Ok(match absent(entry.entry)? {
        Entry::ThreadCreated(v) => {
            json!({"kind":"thread_created", "id":v.id,"threadId":v.thread_id,"commentId":v.comment_id,"actor":actor(v.actor)?,"target":display("ReviewTargetWireDto", &absent(v.target)?)?,"content":v.content,"at":v.at})
        }
        Entry::CommentAppended(v) => {
            json!({"kind":"comment_appended", "id":v.id,"threadId":v.thread_id,"commentId":v.comment_id,"actor":actor(v.actor)?,"content":v.content,"at":v.at})
        }
        Entry::ThreadResolved(v) => {
            json!({"kind":"thread_resolved", "id":v.id,"threadId":v.thread_id,"actor":actor(v.actor)?,"outcome":v.outcome,"summary":v.summary,"at":v.at})
        }
        Entry::ThreadDeleted(v) => {
            json!({"kind":"thread_deleted", "id":v.id,"threadId":v.thread_id,"actor":actor(v.actor)?,"at":v.at})
        }
    })
}

pub(crate) fn read_payload(reader: impl Read) -> Result<Vec<u8>, ConnectError> {
    let mut payload = Vec::new();
    reader
        .take(65_537)
        .read_to_end(&mut payload)
        .map_err(|e| ConnectError::internal(e.to_string()))?;
    if payload.len() > 65_536 {
        return Err(ConnectError::new(
            ErrorCode::InvalidArgument,
            "Provider payload exceeds 65536 bytes",
        ));
    }
    Ok(payload)
}

async fn hook(dir: &Path, provider: HookProvider) -> Result<(), ConnectError> {
    let payload = read_payload(std::io::stdin().lock())?;
    let env = |name| {
        std::env::var(name).map_err(|_| {
            ConnectError::new(ErrorCode::InvalidArgument, format!("{name} is required"))
        })
    };
    let client = connect(dir, Some(env("RELEASH_PROVIDER_LIFECYCLE_TOKEN")?)).await?;
    let request = wire::ReceiveProviderSignalRequest {
        provider: Some(
            match provider {
                HookProvider::Claude => "claude",
                HookProvider::Codex => "codex",
            }
            .try_into()
            .map_err(ConnectError::internal)?,
        ),
        slot_id: env("RELEASH_PROVIDER_LIFECYCLE_SLOT_ID")?,
        binding_id: env("RELEASH_PROVIDER_LIFECYCLE_BINDING_ID")?,
        capability: env("RELEASH_PROVIDER_LIFECYCLE_CAPABILITY")?,
        agent_session_id: env("RELEASH_PROVIDER_LIFECYCLE_AGENT_SESSION_ID")?,
        payload,
    };
    let response = client
        .receive_provider_signal(to_rpc::<rpc::ReceiveProviderSignalRequest>(&request)?)
        .await?;
    let response: wire::ReceiveProviderSignalResponse = to_wire(&response.into_owned())?;
    match absent(response.result)? {
        wire::receive_provider_signal_response::Result::Rejected(value) => Err(ConnectError::new(
            ErrorCode::FailedPrecondition,
            value.reason,
        )),
        _ => Ok(()),
    }
}

#[cfg(test)]
#[path = "commands_test.rs"]
mod commands_tests;
