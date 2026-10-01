use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use crate::domain::app_config::value_objects as app_config_vo;
use crate::domain::app_config::NotionConfigRepository;
use crate::domain::notion::{
    NotionApiGateway, NotionLabelOption, NotionTaskPage, NotionTaskQuery, NotionValidationResult,
};
use crate::usecase::notion::error::NotionUsecaseError;

use crate::usecase::fetched::Fetched;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct NotionTaskListRequest {
    pub path: String,
    pub count: usize,
    pub title: Option<String>,
    pub labels: BTreeMap<String, BTreeSet<String>>,
}

type Results<K, T> = parking_lot::Mutex<HashMap<K, (u64, Fetched<T, NotionUsecaseError>)>>;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct NotionRepoConfigDto {
    pub api_token: String,
    pub database_id: String,
    pub property_mapping: NotionPropertyMappingDto,
}

impl std::fmt::Debug for NotionRepoConfigDto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotionRepoConfigDto")
            .field("api_token", &"[REDACTED]")
            .field("database_id", &self.database_id)
            .field("property_mapping", &self.property_mapping)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NotionPropertyMappingDto {
    pub title: String,
    pub labels: Vec<NotionLabelPropertyDto>,
    pub branch_name: String,
    pub branch_prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NotionLabelPropertyDto {
    pub name: String,
    pub property_type: String,
}

pub(crate) struct NotionUsecase {
    repository: Arc<dyn NotionConfigRepository>,
    config_query: Arc<dyn super::query_service::NotionConfigQueryService>,
    api: Arc<dyn NotionApiGateway>,
    result_generation: std::sync::atomic::AtomicU64,
    task_results: Results<NotionTaskListRequest, NotionTaskPage>,
    label_results: Results<String, Vec<NotionLabelOption>>,
    state_publisher: Option<crate::usecase::state_subscription::StateSubscriptionUsecase>,
}

impl NotionUsecase {
    pub(crate) fn new(
        repository: Arc<dyn NotionConfigRepository>,
        config_query: Arc<dyn super::query_service::NotionConfigQueryService>,
        api: Arc<dyn NotionApiGateway>,
    ) -> Self {
        Self {
            repository,
            config_query,
            api,
            state_publisher: None,
            result_generation: Default::default(),
            task_results: Default::default(),
            label_results: Default::default(),
        }
    }

    pub(crate) fn with_state_publisher(
        mut self,
        publisher: crate::usecase::state_subscription::StateSubscriptionUsecase,
    ) -> Self {
        self.state_publisher = Some(publisher);
        self
    }

    fn config_changed(&self, repo_path: &str) {
        if let Some(publisher) = &self.state_publisher {
            publisher.notify(crate::usecase::state_subscription::StateChangeSource::AppConfig);
            publisher.notify(
                crate::usecase::state_subscription::StateChangeSource::NotionConfig(
                    repo_path.into(),
                ),
            );
        }
    }

    pub(crate) fn acquire_tasks(&self, request: &NotionTaskListRequest) {
        acquire_result(&self.task_results, &self.result_generation, request.clone());
    }

    pub(crate) fn refresh_tasks(&self, request: &NotionTaskListRequest) {
        refresh_result(&self.task_results, request, || {
            query_task_list(
                self.repository.as_ref(),
                self.api.as_ref(),
                &request.path,
                request.count,
                request.title.as_deref(),
                &request.labels,
            )
        });
    }

    pub(crate) fn cached_tasks(
        &self,
        request: &NotionTaskListRequest,
    ) -> Option<Fetched<NotionTaskPage, NotionUsecaseError>> {
        self.task_results
            .lock()
            .get(request)
            .map(|(_, value)| value.clone())
    }

    pub(crate) fn release_tasks(&self, request: &NotionTaskListRequest) {
        self.task_results.lock().remove(request);
    }

    pub(crate) fn acquire_label_options(&self, path: &str) {
        acquire_result(
            &self.label_results,
            &self.result_generation,
            path.to_owned(),
        );
    }

    pub(crate) fn refresh_label_options(&self, path: &str) {
        refresh_result(&self.label_results, &path.to_owned(), || {
            fetch_label_options(self.repository.as_ref(), self.api.as_ref(), path)
        });
    }

    pub(crate) fn cached_label_options(
        &self,
        path: &str,
    ) -> Option<Fetched<Vec<NotionLabelOption>, NotionUsecaseError>> {
        self.label_results
            .lock()
            .get(path)
            .map(|(_, value)| value.clone())
    }

    pub(crate) fn release_label_options(&self, path: &str) {
        self.label_results.lock().remove(path);
    }

    pub(crate) fn save_config(
        &self,
        repo_path: String,
        config: app_config_vo::NotionRepoConfig,
    ) -> Result<(), NotionUsecaseError> {
        save_config(self.repository.as_ref(), repo_path.clone(), config)?;
        self.config_changed(&repo_path);
        Ok(())
    }

    pub(crate) fn get_config(
        &self,
        repo_path: &str,
    ) -> Result<Option<NotionRepoConfigDto>, NotionUsecaseError> {
        Ok(self.config_query.get_config(repo_path)?)
    }

    pub(crate) fn delete_config(&self, repo_path: &str) -> Result<(), NotionUsecaseError> {
        delete_config(self.repository.as_ref(), repo_path)?;
        self.config_changed(repo_path);
        Ok(())
    }

    pub(crate) fn validate_config(
        &self,
        api_token: String,
        database_id: String,
    ) -> Result<NotionValidationResult, NotionUsecaseError> {
        validate_config(self.api.as_ref(), api_token, database_id)
    }
}

fn acquire_result<K: Eq + std::hash::Hash, T>(
    results: &Results<K, T>,
    generation: &std::sync::atomic::AtomicU64,
    key: K,
) {
    let generation = generation
        .fetch_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |value| value.checked_add(1),
        )
        .expect("Notion result generation exhausted");
    results.lock().insert(key, (generation, Fetched::default()));
}

fn refresh_result<K: Eq + std::hash::Hash, T>(
    results: &Results<K, T>,
    key: &K,
    fetch: impl FnOnce() -> Result<T, NotionUsecaseError>,
) {
    let Some(generation) = results.lock().get(key).map(|(generation, _)| *generation) else {
        return;
    };
    let result = fetch();
    let mut results = results.lock();
    if let Some((_, current)) = results
        .get_mut(key)
        .filter(|(current, _)| *current == generation)
    {
        current.record(result);
    }
}

fn query_task_list(
    repository: &dyn NotionConfigRepository,
    api: &dyn NotionApiGateway,
    repo_path: &str,
    count: usize,
    title: Option<&str>,
    labels: &BTreeMap<String, BTreeSet<String>>,
) -> Result<NotionTaskPage, NotionUsecaseError> {
    let config = resolve_config(repository, repo_path)?;
    let mut query = NotionTaskQuery {
        title_filter: title.unwrap_or_default().into(),
        label_filters: labels
            .iter()
            .map(|(key, values)| (key.clone(), values.iter().cloned().collect()))
            .collect(),
        cursor: None,
        page_size: Some(20),
    };
    let mut tasks = Vec::new();
    loop {
        let page = api.query_tasks(&config, &query)?;
        tasks.extend(page.tasks);
        if tasks.len() >= count || !page.has_more {
            let has_more = tasks.len() > count || page.has_more;
            tasks.truncate(count);
            return Ok(NotionTaskPage {
                tasks,
                has_more,
                next_cursor: None,
            });
        }
        let cursor = page
            .next_cursor
            .filter(|cursor| Some(cursor) != query.cursor.as_ref())
            .ok_or_else(|| {
                crate::domain::notion::NotionError::ParseError(
                    "Notion pagination cursor is missing or repeated".into(),
                )
            })?;
        query.cursor = Some(cursor);
    }
}

fn fetch_label_options(
    repository: &dyn NotionConfigRepository,
    api: &dyn NotionApiGateway,
    repo_path: &str,
) -> Result<Vec<NotionLabelOption>, NotionUsecaseError> {
    let config = resolve_config(repository, repo_path)?;
    api.fetch_label_options(&config).map_err(Into::into)
}

fn save_config(
    repository: &dyn NotionConfigRepository,
    repo_path: String,
    config: app_config_vo::NotionRepoConfig,
) -> Result<(), NotionUsecaseError> {
    repository.upsert(repo_path, config).map_err(Into::into)
}

fn delete_config(
    repository: &dyn NotionConfigRepository,
    repo_path: &str,
) -> Result<(), NotionUsecaseError> {
    repository.remove(repo_path).map_err(Into::into)
}

fn validate_config(
    api: &dyn NotionApiGateway,
    api_token: String,
    database_id: String,
) -> Result<NotionValidationResult, NotionUsecaseError> {
    let config = app_config_vo::NotionRepoConfig {
        api_token,
        database_id,
        property_mapping: app_config_vo::NotionPropertyMapping::default(),
    };
    if !config.is_configured() {
        return Ok(NotionValidationResult::not_configured());
    }
    api.validate(&config).map_err(Into::into)
}

fn resolve_config(
    repository: &dyn NotionConfigRepository,
    repo_path: &str,
) -> Result<app_config_vo::NotionRepoConfig, NotionUsecaseError> {
    repository
        .get(repo_path)?
        .filter(app_config_vo::NotionRepoConfig::is_configured)
        .ok_or(NotionUsecaseError::ConfigNotFound)
}

#[cfg(test)]
#[path = "usecase_test.rs"]
mod usecase_tests;
