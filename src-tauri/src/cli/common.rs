use std::path::{Path, PathBuf};

/// 診断 error が 1 件以上検出されたことを表す終了コード。
/// command 自体の失敗（1 = Other / 2 = InvalidInput / 4 = NotFound）と区別する。
pub(super) const DIAGNOSTIC_ERRORS_EXIT_CODE: i32 = 3;

/// CLI の成功結果。stdout と、その結果が表す終了コードの組。
#[derive(Debug, PartialEq, Eq)]
pub struct CliSuccess {
    pub(super) stdout: String,
    pub(super) exit_code: i32,
}

impl CliSuccess {
    #[cfg(feature = "test-support")]
    pub fn test_stdout(&self) -> &str {
        &self.stdout
    }
    #[cfg(feature = "test-support")]
    pub fn test_exit_code(&self) -> i32 {
        self.exit_code
    }
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

pub fn cli_result_exit_code(result: Result<CliSuccess, CliError>) -> i32 {
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

pub fn cli_error_exit_code(error: &CliError) -> i32 {
    match error {
        CliError::NotFound(_) => 4,
        CliError::InvalidInput(_) => 2,
        CliError::Other(_) => 1,
    }
}

pub fn cli_error_stderr(error: &CliError) -> String {
    match error {
        CliError::NotFound(msg) => msg.clone(),
        CliError::InvalidInput(msg) | CliError::Other(msg) => format!("error: {msg}"),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum CliError {
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
pub fn resolve_data_dir_from_env(env_value: Option<String>) -> Result<PathBuf, String> {
    crate::infrastructure::platform::app_data_dir::resolve_for_profile(
        crate::infrastructure::platform::path_aliases::BuildProfile::current(),
        env_value,
    )
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
pub fn ensure_existing_data_dir(path: &Path) -> Result<(), CliError> {
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
#[path = "common_test.rs"]
mod common_tests;
