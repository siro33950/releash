use std::collections::HashMap;

use crate::domain::app_config::value_objects::NotionRepoConfig;

/// 一つの repository の設定を所有し、その変更を受け付ける Entity。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryConfig {
    repo_path: String,
    base_branch: Option<String>,
    branch_overrides: HashMap<String, String>,
    default_branch: Option<String>,
    notion: Option<NotionRepoConfig>,
}

impl RepositoryConfig {
    /// 保存された設定と、Git から検出した既定ブランチから設定を復元する。
    pub fn new(
        repo_path: String,
        base_branch: Option<String>,
        branch_overrides: HashMap<String, String>,
        default_branch: Option<String>,
        notion: Option<NotionRepoConfig>,
    ) -> Self {
        let _ = (
            repo_path,
            base_branch,
            branch_overrides,
            default_branch,
            notion,
        );
        todo!()
    }

    /// この設定が属する repository のパスを返す。
    pub fn repo_path(&self) -> &str {
        &self.repo_path
    }

    /// 選択された base branch を返す。None は自動選択を表す。
    pub fn base_branch(&self) -> Option<&str> {
        self.base_branch.as_deref()
    }

    /// 個別指定、repository の設定、既定ブランチの優先順位で選ぶ。
    pub fn resolve_base_branch(&self, branch_name: &str) -> Option<&str> {
        let _ = (&self.branch_overrides, &self.default_branch, branch_name);
        todo!()
    }

    /// 現存しないブランチに残っている個別指定のブランチ名を返す。
    pub fn stale_branch_overrides(&self, existing_branches: &[String]) -> Vec<String> {
        let _ = (&self.branch_overrides, existing_branches);
        todo!()
    }

    /// 保存されている Notion 設定を返す。
    pub fn notion(&self) -> Option<&NotionRepoConfig> {
        self.notion.as_ref()
    }

    /// token と database の両方が設定されている Notion 設定を返す。
    pub fn configured_notion(&self) -> Option<&NotionRepoConfig> {
        self.notion.as_ref().filter(|config| config.is_configured())
    }

    /// base branch を変更する。None は自動選択に戻すことを表す。
    pub fn set_base_branch(&mut self, base_branch: Option<&str>) {
        self.base_branch = base_branch.map(str::to_owned);
    }

    /// Notion 設定を変更する。None は設定の削除を表す。
    pub fn set_notion(&mut self, notion: Option<NotionRepoConfig>) {
        self.notion = notion;
    }
}
