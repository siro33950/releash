mod support;
use releash_sdk::wire;
use std::process::Command;

#[test]
fn test_cli_サーバが古い場合は操作を呼ばずfailed_preconditionを返す() {
    // Given
    let server = support::Server::start(0, wire::StatePayload::default());
    // When
    let output = server.run(&["workflow", "status", "id", "--json"]);
    // Then
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "failed_precondition");
    assert!(error["error"]["message"]
        .as_str()
        .unwrap()
        .contains("server is older"));
    assert!(error["error"]["message"]
        .as_str()
        .unwrap()
        .contains("releash server restart"));
    assert!(error["error"]["message"]
        .as_str()
        .unwrap()
        .contains("server-fixture"));
    assert!(error["error"]["message"]
        .as_str()
        .unwrap()
        .contains(env!("CARGO_PKG_VERSION")));
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn test_cli_購読で存在しない実行を受け取るとnot_foundを返す() {
    // Given
    let server = support::Server::start(
        1,
        wire::StatePayload {
            value: Some(wire::state_payload::Value::WorkflowExecution(
                wire::NullableWorkflowExecutionView { value: None },
            )),
        },
    );
    // When
    let output = server.run(&["workflow", "status", "id", "--json"]);
    // Then
    assert_eq!(output.status.code(), Some(1));
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "not_found");
    assert_eq!(server.finish().len(), 3);
}

#[test]
fn test_cli_output_getの既存jsonを購読から出力する() {
    // Given
    let payload = wire::StatePayload {
        value: Some(wire::state_payload::Value::WorkflowOutput(
            wire::NullableWorkflowOutputView {
                value: Some(wire::WorkflowOutputView {
                    variant: Some(wire::workflow_output_view::Variant::Submitted(
                        wire::WorkflowOutputViewSubmitted {
                            contract: Some("result".into()),
                            structured_output: Some(
                                serde_json::json!({"tasks":["task"]}).try_into().unwrap(),
                            ),
                            submitted_at: Some(12.0),
                            request_id: Some("request".into()),
                            timestamp: Some(13.0),
                        },
                    )),
                }),
            },
        )),
    };
    let server = support::Server::start(1, payload);
    // When
    let output = server.run(&[
        "workflow", "output", "get", "id", "--node", "node", "--json",
    ]);
    // Then
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"status":"submitted","contract":"result","artifact":{"tasks":["task"]},"submitted_at":12.0,"request_id":"request","timestamp":13.0})
    );
    assert_eq!(server.finish().len(), 3);
}

#[test]
fn test_cli_存在しないdata_dirを作らずunavailableを返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("missing");
    // When
    let output = Command::new(env!("CARGO_BIN_EXE_releash"))
        .args([
            "review",
            "list",
            "--session-id",
            "abc",
            "--json",
            "--data-dir",
        ])
        .arg(&missing)
        .output()
        .unwrap();
    // Then
    assert_eq!(output.status.code(), Some(1));
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "unavailable");
    assert!(!missing.exists());
}

#[test]
fn test_cli_補完と引数関係はサーバ無しで処理する() {
    // Given / When
    let output = Command::new(env!("CARGO_BIN_EXE_releash"))
        .args(["completion", "zsh"])
        .output()
        .unwrap();
    // Then
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .contains("#compdef releash"));
    for option in ["--author", "--unread"] {
        let output = Command::new(env!("CARGO_BIN_EXE_releash"))
            .args(["review", "list", option, "self"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
    }
}

#[test]
fn test_hook_互換でないサーバにはpayloadを送らずhealthを維持してexit0にする() {
    use std::io::Write;
    use std::process::Stdio;
    // Given
    let server = support::Server::start(0, wire::StatePayload::default());
    let health = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(health.path(), "existing-health").unwrap();
    // When
    let mut child = server
        .command(&["hook", "receive", "--provider", "claude"])
        .env("RELEASH_PROVIDER_LIFECYCLE_TOKEN", "operator")
        .env("RELEASH_PROVIDER_LIFECYCLE_HEALTH_FILE", health.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"opaque payload")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    // Then
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"{}");
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("failed_precondition"));
    assert_eq!(std::fs::read(health.path()).unwrap(), b"existing-health");
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn test_cli_historyのjsonと人向け表示が内部型を出さず既存項目を保持する() {
    // Given
    let actor = wire::ReviewActorWireDto {
        kind: Some("agent".try_into().unwrap()),
        backend_id: Some("codex".into()),
        model: None,
        display_name: Some("codex".into()),
    };
    let entry = wire::ReviewHistoryEntryDto {
        entry: Some(wire::review_history_entry_dto::Entry::CommentAppended(
            wire::ReviewHistoryCommentAppendedDto {
                id: "event".into(),
                thread_id: "thread".into(),
                comment_id: "comment".into(),
                actor: Some(actor),
                content: "内容".into(),
                at: 12.0,
            },
        )),
    };
    let payload = wire::StatePayload {
        value: Some(wire::state_payload::Value::ReviewSessionThreadHistory(
            wire::NullableListReviewHistoryEntryDto {
                value: Some(wire::ListReviewHistoryEntryDto { items: vec![entry] }),
            },
        )),
    };
    for json in [true, false] {
        let server = support::Server::start(1, payload.clone());
        let mut args = vec!["review", "history", "thread", "--session-id", "session"];
        if json {
            args.push("--json");
        }
        // When
        let output = server.run(&args);
        // Then
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if json {
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
                serde_json::json!([{"kind":"comment_appended","id":"event","threadId":"thread","commentId":"comment","actor":{"kind":"agent","backendId":"codex","model":null,"displayName":"codex"},"content":"内容","at":12.0}])
            );
        } else {
            let text = String::from_utf8(output.stdout).unwrap();
            assert!(text.contains("codex: 内容"));
            assert!(!text.contains("CommentAppended"));
        }
        assert_eq!(server.finish().len(), 3);
    }
}

#[test]
fn test_hook_不透明なpayloadとenvのscopeをconnectに渡す() {
    use std::io::Write;
    use std::process::Stdio;
    let server = support::Server::start_hook();
    let mut child = server
        .command(&["hook", "receive", "--provider", "claude"])
        .env("RELEASH_PROVIDER_LIFECYCLE_TOKEN", "hook-token")
        .env("RELEASH_PROVIDER_LIFECYCLE_SLOT_ID", "slot")
        .env("RELEASH_PROVIDER_LIFECYCLE_BINDING_ID", "binding")
        .env("RELEASH_PROVIDER_LIFECYCLE_CAPABILITY", "capability")
        .env("RELEASH_PROVIDER_LIFECYCLE_AGENT_SESSION_ID", "session")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"\0opaque provider bytes\xff")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"{}");
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(server.finish().len(), 2);
}

#[test]
fn test_cli_クライアントが古い場合は操作を送らない() {
    let server = support::Server::start(2, wire::StatePayload::default());
    let output = server.run(&["workflow", "status", "id", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "failed_precondition");
    assert!(error["error"]["message"]
        .as_str()
        .unwrap()
        .contains("client is older"));
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn test_cli_購読の失敗はサーバのcodeとmessageでexit1にする() {
    let server = support::Server::start_failure(3, "invalid execution id");
    let output = server.run(&["workflow", "status", "invalid", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stderr).unwrap(),
        serde_json::json!({"error":{"code":"invalid_argument","message":"invalid execution id"}})
    );
    assert_eq!(server.finish().len(), 3);
}

#[test]
fn test_cli_発見したサーバへ接続できない場合はunavailableで終わる() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let pid = std::process::id();
    let discovery = releash_sdk::discovery::LocalApiDiscovery {
        port,
        token: "operator".into(),
        instance_id: "fixture".into(),
        pid,
        process_started_at: releash_sdk::discovery::process_start_time(pid).unwrap(),
    };
    std::fs::write(
        directory.path().join("client-api.json"),
        serde_json::to_vec(&discovery).unwrap(),
    )
    .unwrap();
    // When
    let output = Command::new(env!("CARGO_BIN_EXE_releash"))
        .arg("--data-dir")
        .arg(directory.path())
        .args(["workflow", "status", "id", "--json"])
        .output()
        .unwrap();
    // Then
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "unavailable");
}

fn review_thread() -> (wire::ReviewThreadDto, serde_json::Value) {
    let actor = wire::ReviewActorWireDto {
        kind: Some("agent".try_into().unwrap()),
        backend_id: Some("codex".into()),
        model: Some("server-model".into()),
        display_name: Some("server-agent".into()),
    };
    (
        wire::ReviewThreadDto {
            id: Some("thread".into()),
            worktree_name: Some("/workspace".into()),
            author: Some(actor.clone()),
            target: Some(wire::ReviewTargetWireDto {
                file_path: Some("src/main.rs".into()),
                line_number: Some(3),
                end_line: Some(5),
            }),
            state: Some("open".try_into().unwrap()),
            comments: Some(wire::ListReviewCommentDto {
                items: vec![wire::ReviewCommentDto {
                    id: Some("comment".into()),
                    thread_id: Some("thread".into()),
                    author: Some(actor),
                    content: Some("server-content".into()),
                    created_at: Some(10.0),
                }],
            }),
            resolve: None,
            created_at: Some(10.0),
            updated_at: Some(11.0),
            version: Some(2),
            can_resolve: Some(true),
        },
        serde_json::json!({"id":"thread","worktreeName":"/workspace","author":{"kind":"agent","backendId":"codex","model":"server-model","displayName":"server-agent"},"target":{"filePath":"src/main.rs","lineNumber":3,"endLine":5},"state":"open","comments":[{"id":"comment","threadId":"thread","author":{"kind":"agent","backendId":"codex","model":"server-model","displayName":"server-agent"},"content":"server-content","createdAt":10.0}],"resolve":null,"createdAt":10.0,"updatedAt":11.0,"version":2,"canResolve":true}),
    )
}

#[test]
fn test_cli_review読取_sessionと隔離worktreeの対象と絞り込みをサーバへ渡し既存jsonを表示する() {
    use prost::Message;
    for mode in ["session", "worktree", "get"] {
        let (thread, golden) = review_thread();
        let payload = wire::StatePayload {
            value: Some(if mode == "get" {
                wire::state_payload::Value::ReviewSessionThread(wire::NullableReviewThreadDto {
                    value: Some(thread),
                })
            } else if mode == "worktree" {
                wire::state_payload::Value::ReviewThreads(wire::ListReviewThreadDto {
                    items: vec![thread],
                })
            } else {
                wire::state_payload::Value::ReviewSessionThreads(
                    wire::NullableListReviewThreadDto {
                        value: Some(wire::ListReviewThreadDto {
                            items: vec![thread],
                        }),
                    },
                )
            }),
        };
        let server = support::Server::start_checked(
            payload,
            Box::new(move |method, body| {
                assert!(method.contains("/StartStateSubscription "));
                let request = wire::StartStateSubscriptionRequest::decode(body).unwrap();
                let (target, args): (&str, Vec<&str>) = match mode {
                    "session" => (
                        "review-session-threads",
                        vec![
                            "session",
                            "file=src/main.rs",
                            "state=open",
                            "author=self",
                            "unread=self",
                            "thread=a",
                            "thread=z",
                        ],
                    ),
                    "worktree" => (
                        "review-worktree-threads",
                        vec![
                            "/isolated/worktree",
                            "file=src/main.rs",
                            "state=open",
                            "thread=a",
                            "thread=z",
                        ],
                    ),
                    _ => ("review-session-thread", vec!["session", "thread"]),
                };
                assert_eq!(request.target, target);
                assert_eq!(request.args, args);
                (200, Vec::new())
            }),
        );
        let mut args = if mode == "get" {
            vec!["review", "get", "thread"]
        } else {
            vec![
                "review",
                "list",
                "--file",
                "src/main.rs",
                "--state",
                "open",
                "--thread-id",
                "z",
                "--thread-id",
                "a",
                "--thread-id",
                "z",
            ]
        };
        if mode != "worktree" {
            args.extend(["--session-id", "session"]);
        }
        if mode == "session" {
            args.extend(["--author", "self", "--unread", "self"]);
        }
        args.push("--json");
        let output = server
            .command(&args)
            .env("RELEASH_WORKTREE_PATH", "/isolated/worktree")
            .env("RELEASH_BACKEND_ID", "wrong-client-author")
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
            if mode == "get" {
                golden
            } else {
                serde_json::json!([golden])
            }
        );
        assert_eq!(server.finish().len(), 3);
    }
}

#[test]
fn test_cli_review変更_要求値を一度だけ送りサーバの書き手とjsonと拒否を表示する() {
    use prost::Message;
    for operation in ["create", "comment", "resolve"] {
        for rejected in [false, true] {
            let (thread, golden) = review_thread();
            let server = support::Server::start_unary(Box::new(move |method, body| {
                let response = match operation {
                    "create" => {
                        assert!(method.contains("/CreateSessionReviewThread "));
                        assert_eq!(
                            wire::CreateSessionReviewThreadRequest::decode(body).unwrap(),
                            wire::CreateSessionReviewThreadRequest {
                                session_id: Some("session".into()),
                                file_path: Some("src/main.rs".into()),
                                line_number: Some(3),
                                end_line: Some(5),
                                content: Some("本文".into())
                            }
                        );
                        wire::CreateSessionReviewThreadResponse {
                            thread: Some(thread.clone()),
                        }
                        .encode_to_vec()
                    }
                    "comment" => {
                        assert!(method.contains("/AppendSessionReviewComment "));
                        assert_eq!(
                            wire::AppendSessionReviewCommentRequest::decode(body).unwrap(),
                            wire::AppendSessionReviewCommentRequest {
                                session_id: Some("session".into()),
                                thread_id: Some("thread".into()),
                                content: Some("本文".into())
                            }
                        );
                        wire::AppendSessionReviewCommentResponse {
                            thread: Some(thread.clone()),
                        }
                        .encode_to_vec()
                    }
                    _ => {
                        assert!(method.contains("/ResolveSessionReviewThread "));
                        assert_eq!(
                            wire::ResolveSessionReviewThreadRequest::decode(body).unwrap(),
                            wire::ResolveSessionReviewThreadRequest {
                                session_id: Some("session".into()),
                                thread_id: Some("thread".into()),
                                outcome: Some("fixed".into()),
                                summary: Some("修正済み".into())
                            }
                        );
                        wire::ResolveSessionReviewThreadResponse {
                            thread: Some(thread.clone()),
                        }
                        .encode_to_vec()
                    }
                };
                if rejected {
                    (
                        403,
                        br#"{"code":"permission_denied","message":"server refused writer"}"#
                            .to_vec(),
                    )
                } else {
                    (200, response)
                }
            }));
            let mut args = vec!["review", operation];
            if operation != "create" {
                args.push("thread");
            }
            args.extend(["--session-id", "session", "--json"]);
            if operation == "resolve" {
                args.extend(["--outcome", "fixed", "--summary", "修正済み"]);
            } else {
                args.extend(["--content", "本文"]);
            }
            if operation == "create" {
                args.extend(["--file", "src/main.rs", "--line", "3", "--end-line", "5"]);
            }
            let output = server
                .command(&args)
                .env("RELEASH_WORKTREE_PATH", "/wrong-worktree")
                .env("RELEASH_BACKEND_ID", "wrong-client-author")
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(if rejected { 1 } else { 0 }),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            if rejected {
                assert!(output.stdout.is_empty());
                assert_eq!(
                    serde_json::from_slice::<serde_json::Value>(&output.stderr).unwrap(),
                    serde_json::json!({"error":{"code":"permission_denied","message":"server refused writer"}})
                );
            } else {
                assert_eq!(
                    serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
                    golden
                );
            }
            assert_eq!(server.finish().len(), 2);
        }
    }
}

#[test]
fn test_cli_workflow_status_三状態の既存jsonを表示する() {
    use prost::Message;
    for status in ["running", "completed", "aborted"] {
        let server = support::Server::start_checked(
            wire::StatePayload {
                value: Some(wire::state_payload::Value::WorkflowExecution(
                    wire::NullableWorkflowExecutionView {
                        value: Some(wire::WorkflowExecutionView {
                            id: Some("execution".into()),
                            workflow_name: Some("fixture".into()),
                            status: Some(status.try_into().unwrap()),
                            current_node: Some("main".into()),
                            worktree_path: Some("/workspace".into()),
                            created_from: Some("cli".try_into().unwrap()),
                            started_at: Some(10.0),
                            updated_at: Some(11.0),
                            completed_at: if status == "running" {
                                None
                            } else {
                                Some(12.0)
                            },
                            error_reason: if status == "aborted" {
                                Some("aborted reason".into())
                            } else {
                                None
                            },
                            total_token_usage: Some(wire::TokenUsageView {
                                input_tokens: Some(7),
                                output_tokens: Some(9),
                            }),
                            node_executions: Some(wire::ListNodeExecutionView::default()),
                            artifacts: Some(wire::ListArtifactView::default()),
                            fanouts: Some(wire::ListFanoutView::default()),
                            approval_target: None,
                        }),
                    },
                )),
            },
            Box::new(|method, body| {
                assert!(method.contains("/StartStateSubscription "));
                let request = wire::StartStateSubscriptionRequest::decode(body).unwrap();
                assert_eq!(request.target, "workflow-execution");
                assert_eq!(request.args, ["execution"]);
                (200, Vec::new())
            }),
        );
        let output = server.run(&["workflow", "status", "execution", "--json"]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
            serde_json::json!({"id":"execution","workflowName":"fixture","status":status,"currentNode":"main","worktreePath":"/workspace","createdFrom":"cli","startedAt":10.0,"updatedAt":11.0,"completedAt":if status == "running" { None } else { Some(12.0) },"errorReason":if status == "aborted" { Some("aborted reason") } else { None },"totalTokenUsage":{"inputTokens":7,"outputTokens":9},"nodeExecutions":[],"artifacts":[],"fanouts":[],"approvalTarget":null})
        );
        assert_eq!(server.finish().len(), 3);
    }
}

#[test]
fn test_cli_output_submit_入力artifactを一度だけ送って提出状態へ反映する() {
    use prost::Message;
    use std::sync::{Arc, Mutex};
    for file_input in [false, true] {
        let submitted = Arc::new(Mutex::new(Vec::new()));
        let recorded = submitted.clone();
        let artifact = serde_json::json!({"tasks":["task"],"ok":true});
        let server = support::Server::start_unary(Box::new(move |method, body| {
            assert!(method.contains("/WorkflowSubmitOutput "));
            let request = wire::WorkflowSubmitOutputRequest::decode(body).unwrap();
            recorded.lock().unwrap().push(request);
            (200, Vec::new())
        }));
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), artifact.to_string()).unwrap();
        let raw = artifact.to_string();
        let input = if file_input {
            file.path().to_str().unwrap()
        } else {
            &raw
        };
        let output = server.run(&[
            "workflow",
            "output",
            "submit",
            "--node-execution",
            "node",
            "--type",
            "result",
            if file_input { "--file" } else { "--json" },
            input,
        ]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            output.stdout,
            b"submitted: node_execution_id=node type=result\n"
        );
        assert_eq!(server.finish().len(), 2);
        let requests = submitted.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0],
            wire::WorkflowSubmitOutputRequest {
                node_execution_id: Some("node".into()),
                artifact: Some(wire::WorkflowSubmitArtifactInput {
                    contract: Some("result".into()),
                    value: Some(artifact.clone().try_into().unwrap())
                })
            }
        );
        let stored = requests[0].artifact.as_ref().unwrap();
        let server = support::Server::start_checked(
            wire::StatePayload {
                value: Some(wire::state_payload::Value::WorkflowOutput(
                    wire::NullableWorkflowOutputView {
                        value: Some(wire::WorkflowOutputView {
                            variant: Some(wire::workflow_output_view::Variant::Submitted(
                                wire::WorkflowOutputViewSubmitted {
                                    contract: stored.contract.clone(),
                                    structured_output: stored.value.clone(),
                                    submitted_at: Some(12.0),
                                    request_id: Some("request".into()),
                                    timestamp: Some(13.0),
                                },
                            )),
                        }),
                    },
                )),
            },
            Box::new(|method, body| {
                assert!(method.contains("/StartStateSubscription "));
                let request = wire::StartStateSubscriptionRequest::decode(body).unwrap();
                assert_eq!(request.target, "workflow-output");
                assert_eq!(request.args, ["execution", "main"]);
                (200, Vec::new())
            }),
        );
        let output = server.run(&[
            "workflow",
            "output",
            "get",
            "execution",
            "--node",
            "main",
            "--json",
        ]);
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
            serde_json::json!({"status":"submitted","contract":"result","artifact":artifact,"submitted_at":12.0,"request_id":"request","timestamp":13.0})
        );
        assert_eq!(server.finish().len(), 3);
    }
}

#[test]
fn test_hook_到達不能と拒否と上限超過はstderrへ失敗を出しexit0にする() {
    use prost::Message;
    use std::io::Write;
    use std::process::Stdio;
    for failure in ["unavailable", "rejected", "oversize"] {
        let server = support::Server::start_unary(Box::new(move |method, body| {
            assert_eq!(
                failure, "rejected",
                "oversize payload must not reach the server"
            );
            assert!(method.contains("/ReceiveProviderSignal "));
            assert_eq!(
                wire::ReceiveProviderSignalRequest::decode(body)
                    .unwrap()
                    .payload,
                b"opaque"
            );
            (
                200,
                wire::ReceiveProviderSignalResponse {
                    result: Some(wire::receive_provider_signal_response::Result::Rejected(
                        wire::ReceiveProviderSignalRejected {
                            reason: "binding_expired".into(),
                        },
                    )),
                }
                .encode_to_vec(),
            )
        }));
        let missing = tempfile::tempdir().unwrap();
        if failure == "unavailable" {
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let pid = std::process::id();
            let discovery = releash_sdk::discovery::LocalApiDiscovery {
                port: listener.local_addr().unwrap().port(),
                token: "operator".into(),
                instance_id: "fixture".into(),
                pid,
                process_started_at: releash_sdk::discovery::process_start_time(pid).unwrap(),
            };
            std::fs::write(
                missing.path().join("client-api.json"),
                serde_json::to_vec(&discovery).unwrap(),
            )
            .unwrap();
            drop(listener);
        }
        let mut command = if failure == "unavailable" {
            let mut command = Command::new(env!("CARGO_BIN_EXE_releash"));
            command.arg("--data-dir").arg(missing.path()).args([
                "hook",
                "receive",
                "--provider",
                "claude",
            ]);
            command
        } else {
            server.command(&["hook", "receive", "--provider", "claude"])
        };
        let mut child = command
            .env("RELEASH_PROVIDER_LIFECYCLE_TOKEN", "operator")
            .env("RELEASH_PROVIDER_LIFECYCLE_SLOT_ID", "slot")
            .env("RELEASH_PROVIDER_LIFECYCLE_BINDING_ID", "binding")
            .env("RELEASH_PROVIDER_LIFECYCLE_CAPABILITY", "capability")
            .env("RELEASH_PROVIDER_LIFECYCLE_AGENT_SESSION_ID", "session")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&if failure == "oversize" {
                vec![b'x'; 65_537]
            } else {
                b"opaque".to_vec()
            })
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(output.stdout, b"{}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            stderr.contains(match failure {
                "unavailable" => "unavailable",
                "rejected" => "binding_expired",
                _ => "Provider payload exceeds 65536 bytes",
            }),
            "{stderr}"
        );
        assert_eq!(
            server.finish().len(),
            if failure == "rejected" { 2 } else { 0 }
        );
    }
}

#[test]
fn test_cli_status_互換性に関わらずserver_infoと接続先を出しtokenを隠す() {
    for (protocol, compatibility, guidance) in [
        (1, "compatible", "compatible"),
        (0, "server_older", "releash server restart"),
        (2, "client_older", "update the Releash client"),
    ] {
        // Given
        let server = support::Server::start(protocol, wire::StatePayload::default());
        // When
        let output = server.run(&["status", "--json"]);
        // Then
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["running"], true);
        assert_eq!(value["server"]["daemonId"], "fixture");
        assert_eq!(value["server"]["release"], "server-fixture");
        assert_eq!(value["server"]["pid"], std::process::id());
        assert_eq!(value["server"]["protocol"], serde_json::json!(protocol));
        assert_eq!(
            value["server"]["capabilities"],
            serde_json::json!(["fixture-capability"])
        );
        assert_eq!(value["compatibility"], compatibility);
        assert!(value["guidance"].as_str().unwrap().contains(guidance));
        assert_eq!(value["connection"]["host"], "127.0.0.1");
        assert!(value["connection"]["port"].as_u64().unwrap() > 0);
        assert!(value["uptime_seconds"].is_u64());
        assert!(value["discovery_file"]
            .as_str()
            .unwrap()
            .ends_with("/client-api.json"));
        assert!(!String::from_utf8(output.stdout)
            .unwrap()
            .contains("operator"));
        assert_eq!(server.finish().len(), 1);
    }
}

#[test]
fn test_cli_クライアントが古い場合は更新を案内する() {
    // Given
    let server = support::Server::start(2, wire::StatePayload::default());
    // When
    let output = server.run(&["review", "list", "--session-id", "id", "--json"]);
    // Then
    assert_eq!(output.status.code(), Some(1));
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "failed_precondition");
    assert!(error["error"]["message"]
        .as_str()
        .unwrap()
        .contains("update the Releash client"));
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn test_cli_unimplementedと購読失敗で再起動を案内する() {
    // Given
    for subscribed in [false, true] {
        let server = if subscribed {
            support::Server::start_failure(12, "not implemented")
        } else {
            support::Server::start_unary(Box::new(|_, _| {
                (
                    501,
                    br#"{"code":"unimplemented","message":"not implemented"}"#.to_vec(),
                )
            }))
        };
        let args = if subscribed {
            vec!["workflow", "status", "id", "--json"]
        } else {
            vec![
                "review",
                "resolve",
                "thread",
                "--session-id",
                "id",
                "--outcome",
                "fixed",
                "--summary",
                "fixed",
                "--json",
            ]
        };
        // When
        let output = server.run(&args);
        // Then
        assert_eq!(output.status.code(), Some(1));
        let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(error["error"]["code"], "unimplemented");
        assert!(error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("releash server restart"));
        assert_eq!(server.finish().len(), if subscribed { 3 } else { 2 });
    }
}

#[test]
fn test_cli_古い発見ファイルではstatusが未起動で既存操作はunavailableになる() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    let discovery = releash_sdk::discovery::LocalApiDiscovery {
        port: 1,
        token: "secret".into(),
        instance_id: "old".into(),
        pid: std::process::id(),
        process_started_at: 1,
    };
    std::fs::write(
        dir.path().join("client-api.json"),
        serde_json::to_vec(&discovery).unwrap(),
    )
    .unwrap();
    for (args, code) in [
        (vec!["status", "--json"], 0),
        (vec!["review", "list", "--session-id", "id", "--json"], 1),
    ] {
        // When
        let output = Command::new(env!("CARGO_BIN_EXE_releash"))
            .arg("--data-dir")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap();
        // Then
        assert_eq!(output.status.code(), Some(code));
        if code == 0 {
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["running"],
                false
            );
        } else {
            let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
            assert_eq!(error["error"]["code"], "unavailable");
            assert!(error["error"]["message"]
                .as_str()
                .unwrap()
                .contains("client discovery is stale"));
        }
    }
}

#[test]
fn test_cli_停止を確認できなければshutdown期限で失敗する() {
    // Given
    let server = support::Server::start_unary(Box::new(|path, _| {
        assert!(path.contains("/StopDaemon "));
        (200, Vec::new())
    }));
    let started = std::time::Instant::now();
    // When
    let output = server.run(&["server", "stop"]);
    // Then
    assert_eq!(output.status.code(), Some(1));
    assert!(started.elapsed() >= releash_sdk::daemon::timeout("shutdown_timeout_ms"));
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("停止を確認できません"));
    assert_eq!(server.finish().len(), 2);
}

#[test]
fn test_cli_status_人向け表示でもサーバ情報と両方向の更新案内を表示する() {
    for (protocol, guidance) in [
        (1, "compatible"),
        (
            0,
            "server is older; run `releash server restart` to update the server",
        ),
        (2, "client is older; update the Releash client"),
    ] {
        // Given
        let server = support::Server::start(protocol, wire::StatePayload::default());
        let mut command = server.command(&["status"]);
        let data_dir = std::path::PathBuf::from(command.get_args().nth(1).unwrap());
        let discovery = releash_sdk::discovery::read(&data_dir).unwrap();
        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        // When
        let output = command.output().unwrap();
        let after = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        // Then
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        let text = String::from_utf8(output.stdout).unwrap();
        for line in [
            format!("client release: {}", env!("CARGO_PKG_VERSION")),
            format!("client protocol: {}", releash_sdk::descriptor::protocol()),
            format!("data dir: {}", data_dir.display()),
            "server: running".into(),
            format!("daemon_id: {}", discovery.instance_id),
            format!("pid: {}", discovery.pid),
            format!("process_started_at: {}", discovery.process_started_at),
            "server release: server-fixture".into(),
            format!("server protocol: {protocol}"),
            "capabilities: fixture-capability".into(),
            "serving status: SERVING_STATUS_SERVING".into(),
            format!("compatibility: {guidance}"),
        ] {
            assert!(
                text.lines().any(|actual| actual == line),
                "missing {line:?}: {text}"
            );
        }
        let uptime: u64 = text
            .lines()
            .find_map(|line| line.strip_prefix("uptime seconds: "))
            .expect("uptime must be displayed")
            .parse()
            .unwrap();
        assert!((before.saturating_sub(discovery.process_started_at)
            ..=after.saturating_sub(discovery.process_started_at))
            .contains(&uptime));
        assert_eq!(server.finish().len(), 1);
    }
}
