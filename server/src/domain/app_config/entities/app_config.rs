use std::collections::HashMap;

use crate::domain::agent_session::aggregates::ProviderExecutable;
use crate::domain::app_config::value_objects::{TelemetryConfig, WorkflowConfig};
use crate::domain::app_config::{AppConfigError, ProviderConfigurationRevisionExhausted};
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::repository::normalize_repo_path;

/// アプリ全体の設定を所有し、その変更を受け付ける Entity。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    auto_launch: bool,
    start_minimized: bool,
    repo_paths: Vec<String>,
    external_editor: String,
    terminal_startup_command: String,
    telemetry: TelemetryConfig,
    workflow: WorkflowConfig,
    provider_executables: HashMap<ProviderKind, ProviderExecutable>,
    provider_configuration_revisions: HashMap<ProviderKind, u32>,
}

impl AppConfig {
    /// 保存されている値からアプリ全体の設定を復元する。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        auto_launch: bool,
        start_minimized: bool,
        repo_paths: Vec<String>,
        external_editor: String,
        terminal_startup_command: String,
        telemetry: TelemetryConfig,
        workflow: WorkflowConfig,
        provider_executables: HashMap<ProviderKind, ProviderExecutable>,
        provider_configuration_revisions: HashMap<ProviderKind, u32>,
    ) -> Self {
        Self {
            auto_launch,
            start_minimized,
            repo_paths,
            external_editor,
            terminal_startup_command,
            telemetry,
            workflow,
            provider_executables,
            provider_configuration_revisions,
        }
    }

    /// ログイン時に起動する設定を返す。
    pub fn auto_launch(&self) -> bool {
        self.auto_launch
    }

    /// 最小化状態で起動する設定を返す。
    pub fn start_minimized(&self) -> bool {
        self.start_minimized
    }

    /// 登録されている repository のパスを返す。
    pub fn repo_paths(&self) -> &[String] {
        &self.repo_paths
    }

    /// 選択されている外部エディタを返す。
    pub fn external_editor(&self) -> &str {
        &self.external_editor
    }

    /// Terminal の起動コマンドを返す。
    pub fn terminal_startup_command(&self) -> &str {
        &self.terminal_startup_command
    }

    /// telemetry の設定を返す。
    pub fn telemetry(&self) -> &TelemetryConfig {
        &self.telemetry
    }

    /// workflow の設定を返す。
    pub fn workflow(&self) -> &WorkflowConfig {
        &self.workflow
    }

    /// provider ごとの実行ファイルの上書きを返す。
    pub fn provider_executables(&self) -> &HashMap<ProviderKind, ProviderExecutable> {
        &self.provider_executables
    }

    /// 指定した provider の実行ファイルの上書きを返す。
    pub fn provider_executable(&self, provider: ProviderKind) -> Option<&ProviderExecutable> {
        self.provider_executables.get(&provider)
    }

    /// 指定した provider の設定の版を返す。未更新の設定の版はゼロである。
    pub fn provider_configuration_revision(&self, provider: ProviderKind) -> u32 {
        self.provider_configuration_revisions
            .get(&provider)
            .copied()
            .unwrap_or(0)
    }

    /// ログイン時に起動する設定を変更する。
    pub fn update_auto_launch(&mut self, enabled: bool) {
        self.auto_launch = enabled;
    }

    /// 最小化状態で起動する設定を変更する。
    pub fn update_start_minimized(&mut self, enabled: bool) {
        self.start_minimized = enabled;
    }

    /// 空でなく未登録の repository パスを追加し、追加できたかを返す。
    pub fn add_repo_path(&mut self, path: &str) -> bool {
        let normalized = normalize_repo_path(path);
        if normalized.is_empty() {
            return false;
        }
        if self.repo_paths.iter().any(|p| p == &normalized) {
            return false;
        }
        self.repo_paths.push(normalized);
        true
    }

    /// repository パスの登録を削除し、削除できたかを返す。
    pub fn remove_repo_path(&mut self, path: &str) -> bool {
        let normalized = normalize_repo_path(path);
        let new_paths: Vec<String> = self
            .repo_paths
            .iter()
            .filter(|p| *p != &normalized)
            .cloned()
            .collect();
        if new_paths.len() == self.repo_paths.len() {
            return false;
        }
        self.repo_paths = new_paths;
        true
    }

    /// 使用する外部エディタを変更する。空の値はシステム既定を表す。
    pub fn update_external_editor(&mut self, editor: String) {
        self.external_editor = editor;
    }

    /// NUL を含まない Terminal の起動コマンドへ変更する。
    pub fn update_terminal_startup_command(
        &mut self,
        command: String,
    ) -> Result<(), AppConfigError> {
        if command.contains('\0') {
            return Err(AppConfigError::InvalidInput(
                "Terminal startup command contains NUL".into(),
            ));
        }
        self.terminal_startup_command = command;
        Ok(())
    }

    /// performance telemetry の有効・無効を変更する。
    pub fn update_performance_telemetry(&mut self, enabled: bool) {
        self.telemetry.performance_telemetry = enabled;
    }

    /// crash reporting の有効・無効を変更する。
    pub fn update_crash_reporting(&mut self, enabled: bool) {
        self.telemetry.crash_reporting = enabled;
    }

    /// 承認の自動化の有効・無効を変更する。
    pub fn update_approval_auto_approve(&mut self, enabled: bool) {
        self.workflow = WorkflowConfig {
            approval_auto_approve: enabled,
        };
    }

    /// provider の実行ファイル指定と版を更新する。None は上書きの解除を表す。
    /// 同じ指定でも版を進める。版が上限に達している場合は変更しない。
    pub fn update_provider_executable(
        &mut self,
        provider: ProviderKind,
        executable: Option<ProviderExecutable>,
    ) -> Result<(), ProviderConfigurationRevisionExhausted> {
        let revision = self
            .provider_configuration_revision(provider)
            .checked_add(1)
            .ok_or(ProviderConfigurationRevisionExhausted)?;
        match executable {
            Some(executable) => {
                self.provider_executables.insert(provider, executable);
            }
            None => {
                self.provider_executables.remove(&provider);
            }
        }
        self.provider_configuration_revisions
            .insert(provider, revision);
        Ok(())
    }
}

#[cfg(test)]
#[path = "app_config_test.rs"]
mod app_config_tests;
