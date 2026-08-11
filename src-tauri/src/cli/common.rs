use std::path::{Path, PathBuf};

pub(super) fn cli_result_exit_code(result: Result<String, CliError>) -> i32 {
    match result {
        Ok(stdout) => {
            print!("{stdout}");
            0
        }
        Err(error) => {
            eprintln!("{}", cli_error_stderr(&error));
            cli_error_exit_code(&error)
        }
    }
}

pub(super) fn cli_error_exit_code(error: &CliError) -> i32 {
    match error {
        CliError::NotFound(_) => 4,
        CliError::InvalidInput(_) => 2,
        CliError::Other(_) => 1,
    }
}

pub(super) fn cli_error_stderr(error: &CliError) -> String {
    match error {
        CliError::NotFound(msg) => msg.clone(),
        CliError::InvalidInput(msg) | CliError::Other(msg) => format!("error: {msg}"),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum CliError {
    /// execution / template が見つからない。
    NotFound(String),
    /// 入力フォーマット不正（不正な execution_id、不正な status filter 値など）。
    InvalidInput(String),
    /// その他の I/O / serialization エラー。
    Other(String),
}

impl From<String> for CliError {
    fn from(msg: String) -> Self {
        CliError::Other(msg)
    }
}

pub(super) fn resolve_data_dir() -> Result<PathBuf, String> {
    resolve_data_dir_from_env(std::env::var("RELEASH_DATA_DIR").ok())
}

/// `resolve_data_dir` の pure 版（env を入力で受ける）。
///
/// spec [01] 解決順序「明示指定 > alias 内包値」をテストで検証可能にするための分離。
/// 明示指定が空文字列の場合は未設定扱いとし、alias 内包値にフォールバックする。
fn resolve_data_dir_from_env(env_value: Option<String>) -> Result<PathBuf, String> {
    if let Some(custom) = env_value.filter(|s| !s.is_empty()) {
        return Ok(PathBuf::from(custom));
    }
    let aliases = crate::infrastructure::platform::path_aliases::PathAliases::from_runtime(None)?;
    Ok(aliases.releash().data_dir.clone())
}

/// data_dir を解決し、パスが実在することを確認する。
///
/// [05] 観測経路境界: `RELEASH_DATA_DIR` の typo / アプリ未起動などで data_dir
/// が存在しない場合に「executions が 0 件」と紛れないよう、CLI 入口で `NotFound`
/// として弾く（5-1 修正）。
pub(super) fn resolve_existing_data_dir() -> Result<PathBuf, CliError> {
    let path = resolve_data_dir().map_err(CliError::Other)?;
    ensure_existing_data_dir(&path)?;
    Ok(path)
}

/// data_dir パスの実在を確認する純粋判定（環境変数に依存せずテスト可能）。
pub(super) fn ensure_existing_data_dir(path: &Path) -> Result<(), CliError> {
    if !path.exists() {
        return Err(CliError::NotFound(format!(
            "data directory does not exist: {}",
            path.display()
        )));
    }
    Ok(())
}

pub(super) fn validate_execution_id(execution_id: &str) -> Result<(), CliError> {
    uuid::Uuid::parse_str(execution_id)
        .map(|_| ())
        .map_err(|_| {
            CliError::InvalidInput("Invalid execution_id format (must be UUID)".to_string())
        })
}

pub(super) fn validate_node(node: &str) -> Result<(), CliError> {
    if node.trim().is_empty() {
        return Err(CliError::InvalidInput(
            "--node must not be empty".to_string(),
        ));
    }
    Ok(())
}

/// 表示用の固定幅列に収まるよう文字列を短縮する。
pub(super) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{truncated}…")
    }
}

#[cfg(test)]
pub(in crate::cli) mod test_support {
    use std::fs;
    use std::path::Path;

    use sha2::Digest as _;

    use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
    use crate::adaptor::gateway::workflow::event::WorkflowEvent;
    use crate::adaptor::gateway::workflow::execution_store::{
        workflow_execution_record, WorkflowExecutionMetadata,
    };
    use crate::domain::comment::{
        ReviewActor, ReviewComment, ReviewHistoryEntry, ReviewResolveInfo, ReviewTarget,
        ReviewThread, ReviewThreadState,
    };
    use crate::domain::local_event::{
        CommitIdentity, CommitOperationKind, IdempotencyBinding, LocalAtomicBatch,
        LocalEventTransactionRepository, LocalStateMutation, Revision, RevisionGuard,
        SessionProjectionMutation, SessionProjectionRecord, WorkflowExecutionProjectionMutation,
        WorkflowExecutionProjectionRecord,
    };
    use crate::domain::workflow::{ExecutionOrigin, ExecutionStatus, TokenUsage};

    pub(in crate::cli) fn write_review_config(data_dir: &Path) {
        fs::write(data_dir.join("releash.toml"), "[agents.codex]\n").unwrap();
    }

    pub(in crate::cli) fn write_review_session(
        data_dir: &Path,
        session_id: &str,
        backend_id: Option<&str>,
    ) {
        write_review_session_with_lifecycle(
            data_dir,
            session_id,
            backend_id,
            crate::domain::local_event::AgentSessionLifecycleRecord::Open,
        );
    }

    pub(in crate::cli) fn write_review_session_with_lifecycle(
        data_dir: &Path,
        session_id: &str,
        backend_id: Option<&str>,
        lifecycle: crate::domain::local_event::AgentSessionLifecycleRecord,
    ) {
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(data_dir.to_path_buf()))
                .unwrap();
        let provider = match backend_id.unwrap_or("codex") {
            "claude" => crate::domain::local_event::AgentSessionProviderRecord::Claude,
            _ => crate::domain::local_event::AgentSessionProviderRecord::Codex,
        };
        let commit_id = uuid::Uuid::new_v4().to_string();
        let batch = LocalAtomicBatch {
            commit_id: CommitIdentity::parse(&commit_id).unwrap(),
            idempotency: IdempotencyBinding {
                installation_id: store.installation_id().to_string(),
                operation_kind: CommitOperationKind::Projection,
                idempotency_key: commit_id,
                payload_hash: [7; 32],
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: vec![LocalStateMutation::SessionProjection(
                SessionProjectionMutation {
                    session_id: format!("agent-session:{session_id}"),
                    projection: SessionProjectionRecord::AgentSession(
                        crate::domain::local_event::AgentSessionProjectionRecord {
                            id: session_id.to_string(),
                            workspace_identity: "/repo".to_string(),
                            worktree_path: "/repo".to_string(),
                            provider,
                            origin:
                                crate::domain::local_event::AgentSessionOriginRecord::Standalone,
                            lifecycle,
                            provider_session_id: None,
                            transcript_ref: None,
                            initial_instruction_admitted: false,
                            last_exit_abnormal: false,
                        },
                    ),
                    expected: RevisionGuard::Absent,
                    revision: Revision::new(0).unwrap(),
                },
            )],
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(store.commit_batch(batch)).unwrap();
    }

    pub(in crate::cli) fn make_execution(
        execution_id: &str,
        worktree: &str,
        status: ExecutionStatus,
        started_at: f64,
    ) -> WorkflowExecutionMetadata {
        WorkflowExecutionMetadata {
            execution_id: execution_id.to_string(),
            workflow_name: "wf".to_string(),
            status,
            worktree_path: worktree.to_string(),
            current_node: None,
            created_from: ExecutionOrigin::Cli,
            started_at,
            updated_at: started_at,
            completed_at: if status.is_terminal() {
                Some(started_at + 1.0)
            } else {
                None
            },
            error_reason: None,
            interruption_reason: None,
            resume_from_node: None,
            total_token_usage: TokenUsage::default(),
        }
    }

    pub(in crate::cli) fn write_execution_file(
        data_dir: &Path,
        execution: &WorkflowExecutionMetadata,
    ) {
        let executions_dir = data_dir.join("workflow_executions");
        fs::create_dir_all(&executions_dir).unwrap();
        let path = executions_dir.join(format!("{}.json", execution.execution_id));
        let json = serde_json::to_string_pretty(execution).unwrap();
        fs::write(path, json).unwrap();
        write_canonical_execution(data_dir, execution);
    }

    pub(in crate::cli) fn initialize_canonical_store(data_dir: &Path) {
        drop(
            LocalEventStore::open(LocalEventStoreConfig::production(data_dir.to_path_buf()))
                .expect("initialize canonical local event store"),
        );
    }

    pub(in crate::cli) fn append_workflow_event(data_dir: &Path, event: &WorkflowEvent) {
        crate::adaptor::gateway::workflow::log::WorkflowEventLog::new(data_dir)
            .append(event)
            .expect("append legacy workflow event fixture");
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(data_dir.to_path_buf()))
                .expect("open canonical local event store");
        let log = crate::adaptor::gateway::workflow::log::WorkflowEventLog::with_authority(
            store.clone(),
            store.installation_id().to_string(),
        );
        log.append_batch_durable_with_mutations_blocking_as(
            CommitOperationKind::Workflow,
            std::slice::from_ref(event),
            Vec::new(),
        )
        .expect("append canonical workflow event fixture");
    }

    fn write_canonical_execution(data_dir: &Path, execution: &WorkflowExecutionMetadata) {
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(data_dir.to_path_buf()))
                .expect("open canonical local event store");
        let record = workflow_execution_record(execution);
        let revision = Revision::new(0).unwrap();
        let batch = LocalAtomicBatch {
            commit_id: CommitIdentity::parse(&uuid::Uuid::new_v4().to_string()).unwrap(),
            idempotency: IdempotencyBinding {
                installation_id: store.installation_id().to_string(),
                operation_kind: CommitOperationKind::Workflow,
                idempotency_key: format!("cli-test-execution:{}", execution.execution_id),
                payload_hash: sha2::Sha256::digest(
                    format!(
                        "{}:{}:{}",
                        execution.execution_id, execution.worktree_path, execution.updated_at
                    )
                    .as_bytes(),
                )
                .into(),
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: vec![
                LocalStateMutation::SessionProjection(SessionProjectionMutation {
                    session_id: format!("workflow:{}", execution.execution_id),
                    projection: SessionProjectionRecord::WorkflowExecution(
                        WorkflowExecutionProjectionRecord::Present(record.clone()),
                    ),
                    expected: RevisionGuard::Absent,
                    revision,
                }),
                LocalStateMutation::WorkflowExecutionProjection(
                    WorkflowExecutionProjectionMutation {
                        projection: WorkflowExecutionProjectionRecord::Present(record),
                        expected: RevisionGuard::Absent,
                        revision,
                    },
                ),
            ],
        };
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(store.commit_batch(batch))
            .expect("seed canonical workflow execution");
    }

    pub(in crate::cli) fn test_uuid(seed: u8) -> String {
        uuid::Uuid::from_bytes([seed; 16]).to_string()
    }

    pub(in crate::cli) fn review_cli_thread(state: ReviewThreadState) -> ReviewThread {
        let thread_id = test_uuid(42);
        let author = ReviewActor::provider_agent("codex".to_string(), None).redacted_for_public();
        let resolver = ReviewActor::human().redacted_for_public();
        ReviewThread {
            id: thread_id.clone(),
            worktree_name: "/repo".to_string(),
            author: author.clone(),
            target: ReviewTarget {
                file_path: Some("src/main.rs".to_string()),
                line_number: Some(3),
                end_line: Some(5),
            },
            state: state.clone(),
            comments: vec![ReviewComment {
                id: test_uuid(43),
                thread_id,
                author,
                content: "Claim".to_string(),
                created_at: 10.0,
            }],
            resolve: (state == ReviewThreadState::Resolved).then_some(ReviewResolveInfo {
                actor: resolver,
                outcome: "accepted".to_string(),
                summary: "done".to_string(),
                resolved_at: 20.0,
            }),
            created_at: 10.0,
            updated_at: 20.0,
            version: 2,
            can_resolve: state == ReviewThreadState::Open,
        }
    }

    pub(in crate::cli) fn review_history_entries() -> Vec<ReviewHistoryEntry> {
        let thread = review_cli_thread(ReviewThreadState::Open);
        vec![
            ReviewHistoryEntry::ThreadCreated {
                id: test_uuid(50),
                thread_id: thread.id.clone(),
                comment_id: test_uuid(51),
                actor: thread.author.clone(),
                target: thread.target.clone(),
                content: "Claim".to_string(),
                at: 10.0,
            },
            ReviewHistoryEntry::ThreadResolved {
                id: test_uuid(52),
                thread_id: thread.id,
                actor: ReviewActor::human().redacted_for_public(),
                outcome: "accepted".to_string(),
                summary: "done".to_string(),
                at: 20.0,
            },
        ]
    }

    pub(in crate::cli) fn execution_started_event(
        execution_id: &str,
        workflow_name: &str,
        worktree: &str,
    ) -> WorkflowEvent {
        WorkflowEvent::ExecutionStarted {
            execution_id: execution_id.to_string(),
            workflow_name: workflow_name.to_string(),
            worktree_path: worktree.to_string(),
            created_from: ExecutionOrigin::Cli,
            request: String::new(),
            definition: crate::adaptor::gateway::workflow::schema::WorkflowDefinitionYaml {
                name: workflow_name.to_string(),
                description: "test".to_string(),
                builtin: false,
                schemas: Default::default(),
                nodes: vec![],
            },
            timestamp: 100.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn exit_code_mapping_is_stable() {
        assert_eq!(cli_result_exit_code(Ok(String::new())), 0);
        assert_eq!(
            cli_error_exit_code(&CliError::InvalidInput("bad".to_string())),
            2
        );
        assert_eq!(
            cli_error_exit_code(&CliError::NotFound("missing".to_string())),
            4
        );
        assert_eq!(cli_error_exit_code(&CliError::Other("io".to_string())), 1);
    }

    #[test]
    fn cli_error_stderr_mapping_is_stable() {
        assert_eq!(
            cli_error_stderr(&CliError::InvalidInput("bad".to_string())),
            "error: bad"
        );
        assert_eq!(
            cli_error_stderr(&CliError::NotFound("missing".to_string())),
            "missing"
        );
        assert_eq!(
            cli_error_stderr(&CliError::Other("io".to_string())),
            "error: io"
        );
    }

    #[test]
    fn validate_execution_id_rejects_non_uuid() {
        assert!(validate_execution_id("not-a-uuid").is_err());
        assert!(validate_execution_id("").is_err());
        assert!(validate_execution_id("../etc/passwd").is_err());
    }

    #[test]
    fn validate_execution_id_accepts_valid_uuid() {
        assert!(validate_execution_id("550e8400-e29b-41d4-a716-446655440000").is_ok());
    }

    /// [05] 観測経路境界 (5-1 修正): data_dir が存在しない場合は `NotFound` として
    /// 扱い、「executions 0 件」と「向き先がそもそも無い」を区別する。
    #[test]
    fn ensure_existing_data_dir_returns_not_found_for_missing_path() {
        let missing = std::path::PathBuf::from("/non/existent/releash-data-dir-test-path");
        let err = ensure_existing_data_dir(&missing).expect_err("missing data_dir must error");
        let CliError::NotFound(msg) = &err else {
            panic!("expected CliError::NotFound for missing data_dir, got: {err:?}");
        };
        assert!(
            msg.contains(&missing.display().to_string()),
            "error message must contain the path, got: {msg}"
        );
    }

    /// [05] 観測経路境界 (5-1 修正): data_dir が存在する場合は Ok を返す。
    #[test]
    fn ensure_existing_data_dir_returns_ok_for_existing_path() {
        let tmp = TempDir::new().unwrap();
        ensure_existing_data_dir(tmp.path()).expect("existing data_dir must succeed");
    }

    /// spec [01] 解決順序「明示指定 > alias 内包値」: RELEASH_DATA_DIR が明示
    /// 指定されている場合は、その値がそのまま採用される（PathBuf 化のみ）。
    #[test]
    fn resolve_data_dir_uses_explicit_env_when_set() {
        let resolved = resolve_data_dir_from_env(Some("/explicit/path".to_string())).unwrap();
        assert_eq!(resolved, std::path::PathBuf::from("/explicit/path"));
    }

    /// spec [01] 解決順序「明示指定 > alias 内包値」: 明示指定が無い場合は
    /// `PathAliases` から導いた alias 内包の data_dir を返す（既定値は bundle
    /// identifier suffix を持つ）。
    #[test]
    fn resolve_data_dir_falls_back_to_alias_data_dir_when_env_unset() {
        if dirs::data_dir().is_none() {
            return;
        }
        let resolved = resolve_data_dir_from_env(None).unwrap();
        let expected_suffix =
            crate::infrastructure::platform::path_aliases::default_data_dir_name_for_profile(
                crate::infrastructure::platform::path_aliases::BuildProfile::current(),
            );
        assert!(
            resolved.ends_with(expected_suffix),
            "expected suffix {expected_suffix}, got {}",
            resolved.display()
        );
    }

    /// spec [01]: 明示指定が空文字列のときは未設定扱いとし alias 内包値に
    /// フォールバックする（空文字列を data_dir として採用すると以降の
    /// 観測経路で「executions 0 件」と紛れるため）。
    #[test]
    fn resolve_data_dir_treats_empty_env_as_unset() {
        if dirs::data_dir().is_none() {
            return;
        }
        let resolved = resolve_data_dir_from_env(Some(String::new())).unwrap();
        let expected_suffix =
            crate::infrastructure::platform::path_aliases::default_data_dir_name_for_profile(
                crate::infrastructure::platform::path_aliases::BuildProfile::current(),
            );
        assert!(
            resolved.ends_with(expected_suffix),
            "empty env should fall through to alias data_dir, got {}",
            resolved.display()
        );
    }
}
