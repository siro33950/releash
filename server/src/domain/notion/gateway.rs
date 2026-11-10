use crate::domain::app_config::value_objects::NotionRepoConfig;

use super::{NotionError, NotionTaskPage, NotionTaskQuery, NotionValidationResult};

#[async_trait::async_trait]

pub trait NotionApiGateway: Send + Sync {
    async fn query_tasks(
        &self,
        config: &NotionRepoConfig,
        query: &NotionTaskQuery,
    ) -> Result<NotionTaskPage, NotionError>;

    async fn validate(
        &self,
        config: &NotionRepoConfig,
    ) -> Result<NotionValidationResult, NotionError>;
}
