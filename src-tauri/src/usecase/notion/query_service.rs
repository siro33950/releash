use super::usecase::NotionRepoConfigDto;

pub(crate) trait NotionConfigQueryService: Send + Sync {
    fn get_config(
        &self,
        repo_path: &str,
    ) -> Result<Option<NotionRepoConfigDto>, crate::domain::app_config::AppConfigError>;
}
