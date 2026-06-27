use crate::domain::app_config::value_objects::NotionRepoConfig;

use super::{
    NotionError, NotionLabelOption, NotionTaskPage, NotionTaskQuery, NotionValidationResult,
};

pub(crate) trait NotionApiGateway: Send + Sync {
    fn query_tasks(
        &self,
        config: &NotionRepoConfig,
        query: &NotionTaskQuery,
    ) -> Result<NotionTaskPage, NotionError>;

    fn fetch_label_options(
        &self,
        config: &NotionRepoConfig,
    ) -> Result<Vec<NotionLabelOption>, NotionError>;

    fn validate(&self, config: &NotionRepoConfig) -> NotionValidationResult;
}
