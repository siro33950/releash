use std::path::{Path, PathBuf};

/// 診断 error が 1 件以上検出されたことを表す終了コード。
/// command 自体の失敗（1 = Other / 2 = InvalidInput / 4 = NotFound）と区別する。
pub(super) const DIAGNOSTIC_ERRORS_EXIT_CODE: i32 = 3;

/// CLI の成功結果。stdout と、その結果が表す終了コードの組。
#[derive(Debug, PartialEq, Eq)]
pub(super) struct CliSuccess {
    pub(super) stdout: String,
    pub(super) exit_code: i32,
}

impl CliSuccess {
    /// 終了コード 0 の成功。
    pub(super) fn ok(stdout: String) -> Self {
        Self {
            stdout,
            exit_code: 0,
        }
    }

    /// 処理は成功したが、結果の内容が非 zero 終了を表す場合。
    pub(super) fn with_exit_code(stdout: String, exit_code: i32) -> Self {
        Self { stdout, exit_code }
    }
}

pub(super) fn cli_result_exit_code(result: Result<CliSuccess, CliError>) -> i32 {
    match result {
        Ok(success) => {
            print!("{}", success.stdout);
            success.exit_code
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

    use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
    use crate::adaptor::gateway::workflow::event::WorkflowEvent;
    use crate::adaptor::gateway::workflow::execution_store::WorkflowExecutionMetadata;
    use crate::domain::comment::{
        ReviewActor, ReviewComment, ReviewHistoryEntry, ReviewResolveInfo, ReviewTarget,
        ReviewThread, ReviewThreadState,
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
            crate::domain::agent_session::aggregates::AgentSessionLifecycle::Open,
        );
    }

    pub(in crate::cli) fn write_review_session_with_lifecycle(
        data_dir: &Path,
        session_id: &str,
        backend_id: Option<&str>,
        lifecycle: crate::domain::agent_session::aggregates::AgentSessionLifecycle,
    ) {
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(data_dir.to_path_buf()))
                .unwrap();
        let provider = match backend_id.unwrap_or("codex") {
            "claude" => crate::domain::provider_lifecycle::ProviderKind::Claude,
            _ => crate::domain::provider_lifecycle::ProviderKind::Codex,
        };
        let meta = crate::domain::workflow::NodeFactMeta {
            tree_id: session_id.to_string(),
            node_execution_id: session_id.to_string(),
            parent_id: None,
            node_name: "session".to_string(),
            kind: crate::domain::workflow::NodeKindName::Session,
            attempt: 1,
        };
        let root =
            crate::domain::workflow::NodeFact::Started(crate::domain::workflow::StartedFact {
                parent: None,
                root: Some(crate::domain::workflow::TreeRootFact::Session(
                    crate::domain::workflow::SessionRootFact {
                        workspace_identity: "/repo".to_string(),
                        worktree_path: "/repo".to_string(),
                        session: crate::domain::workflow::SessionSpec {
                            provider,
                            model: None,
                            permission: None,
                            facets: Default::default(),
                        },
                        created_from: ExecutionOrigin::DesktopUi,
                    },
                )),
            });
        crate::adaptor::gateway::workflow::fact_log::append_single_fact(&store, &meta, &root, 1)
            .unwrap();
        let lifecycle_fact = match lifecycle {
            crate::domain::agent_session::aggregates::AgentSessionLifecycle::Open => None,
            crate::domain::agent_session::aggregates::AgentSessionLifecycle::Paused => {
                Some(crate::domain::workflow::NodeFact::ProcessExited(
                    crate::domain::workflow::ProcessExitedFact {
                        exit_code: Some(0),
                        result_summary: None,
                        failure_reason: None,
                        failure_kind: None,
                    },
                ))
            }
            crate::domain::agent_session::aggregates::AgentSessionLifecycle::Archived => {
                Some(crate::domain::workflow::NodeFact::ArchiveRequested)
            }
        };
        if let Some(fact) = lifecycle_fact {
            crate::adaptor::gateway::workflow::fact_log::append_single_fact(
                &store, &meta, &fact, 2,
            )
            .unwrap();
        }
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
        append_workflow_events(data_dir, std::slice::from_ref(event));
    }

    pub(in crate::cli) fn append_workflow_events(data_dir: &Path, events: &[WorkflowEvent]) {
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(data_dir.to_path_buf()))
                .expect("open canonical local event store");
        crate::adaptor::gateway::workflow::fact_log::append_facts_for_events(&store, events)
            .expect("append canonical node fact fixture");
    }

    fn write_canonical_execution(data_dir: &Path, execution: &WorkflowExecutionMetadata) {
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(data_dir.to_path_buf()))
                .expect("open canonical local event store");
        crate::adaptor::gateway::workflow::test_support::seed_canonical_execution(
            &store,
            execution,
            &[],
        );
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
                nodes: vec![crate::adaptor::gateway::workflow::schema::NodeDefinition {
                    name: "main".to_string(),
                    kind: crate::adaptor::gateway::workflow::schema::NodeKind::Session(
                        crate::adaptor::gateway::workflow::schema::SessionSpec::default(),
                    ),
                    ..Default::default()
                }],
                entry: "main".to_string(),
            },
            timestamp: 100.0,
        }
    }

    pub(in crate::cli) fn root_node_started_event(
        execution_id: &str,
        node_execution_id: &str,
        node_name: &str,
        timestamp: f64,
    ) -> WorkflowEvent {
        WorkflowEvent::NodeStarted {
            execution_id: execution_id.to_string(),
            node_execution_id: node_execution_id.to_string(),
            node_name: node_name.to_string(),
            kind: crate::domain::workflow::NodeKindName::Session,
            attempt: 1,
            parent: None,
            timestamp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn exit_code_mapping_is_stable() {
        assert_eq!(cli_result_exit_code(Ok(CliSuccess::ok(String::new()))), 0);
        assert_eq!(
            cli_result_exit_code(Ok(CliSuccess::with_exit_code(
                String::new(),
                DIAGNOSTIC_ERRORS_EXIT_CODE,
            ))),
            3
        );
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
