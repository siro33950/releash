use crate::domain::app_config::value_objects::NotionRepoConfig;

/// 一つの repository の設定を所有し、その変更を受け付ける Entity。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryConfig {
    repo_path: String,
    base_branch: Option<String>,
    notion: Option<NotionRepoConfig>,
}

impl RepositoryConfig {
    /// repository の識別と保存されている値から設定を復元する。
    pub fn new(
        repo_path: String,
        base_branch: Option<String>,
        notion: Option<NotionRepoConfig>,
    ) -> Self {
        Self {
            repo_path,
            base_branch,
            notion,
        }
    }

    /// この設定が属する repository のパスを返す。
    pub fn repo_path(&self) -> &str {
        &self.repo_path
    }

    /// 選択された base branch を返す。None は自動選択を表す。
    pub fn base_branch(&self) -> Option<&str> {
        self.base_branch.as_deref()
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
