use std::sync::Arc;

use crate::domain::app_config::value_objects as app_config_vo;
use crate::domain::app_config::NotionConfigRepository;
use crate::domain::notion::{
    NotionApiGateway, NotionLabelOption, NotionTaskPage, NotionTaskQuery, NotionValidationResult,
};

const NOTION_CONFIG_NOT_FOUND: &str = "Notion設定が見つかりません";

pub(crate) struct NotionUsecase {
    repository: Arc<dyn NotionConfigRepository>,
    api: Arc<dyn NotionApiGateway>,
}

impl NotionUsecase {
    pub(crate) fn new(
        repository: Arc<dyn NotionConfigRepository>,
        api: Arc<dyn NotionApiGateway>,
    ) -> Self {
        Self { repository, api }
    }

    pub(crate) fn query_tasks(
        &self,
        repo_path: &str,
        query: &NotionTaskQuery,
    ) -> Result<NotionTaskPage, String> {
        query_tasks(
            self.repository.as_ref(),
            self.api.as_ref(),
            repo_path,
            query,
        )
    }

    pub(crate) fn fetch_label_options(
        &self,
        repo_path: &str,
    ) -> Result<Vec<NotionLabelOption>, String> {
        fetch_label_options(self.repository.as_ref(), self.api.as_ref(), repo_path)
    }

    pub(crate) fn save_config(
        &self,
        repo_path: String,
        config: app_config_vo::NotionRepoConfig,
    ) -> Result<(), String> {
        save_config(self.repository.as_ref(), repo_path, config)
    }

    pub(crate) fn get_config(
        &self,
        repo_path: &str,
    ) -> Result<Option<app_config_vo::NotionRepoConfig>, String> {
        get_config(self.repository.as_ref(), repo_path)
    }

    pub(crate) fn delete_config(&self, repo_path: &str) -> Result<(), String> {
        delete_config(self.repository.as_ref(), repo_path)
    }

    pub(crate) fn validate_config(
        &self,
        api_token: String,
        database_id: String,
    ) -> NotionValidationResult {
        validate_config(self.api.as_ref(), api_token, database_id)
    }
}

fn query_tasks(
    repository: &dyn NotionConfigRepository,
    api: &dyn NotionApiGateway,
    repo_path: &str,
    query: &NotionTaskQuery,
) -> Result<NotionTaskPage, String> {
    let config = resolve_config(repository, repo_path)?;
    api.query_tasks(&config, query).map_err(|e| e.to_string())
}

fn fetch_label_options(
    repository: &dyn NotionConfigRepository,
    api: &dyn NotionApiGateway,
    repo_path: &str,
) -> Result<Vec<NotionLabelOption>, String> {
    let config = resolve_config(repository, repo_path)?;
    api.fetch_label_options(&config).map_err(|e| e.to_string())
}

fn save_config(
    repository: &dyn NotionConfigRepository,
    repo_path: String,
    config: app_config_vo::NotionRepoConfig,
) -> Result<(), String> {
    repository
        .upsert(repo_path, config)
        .map_err(|error| error.to_string())
}

fn get_config(
    repository: &dyn NotionConfigRepository,
    repo_path: &str,
) -> Result<Option<app_config_vo::NotionRepoConfig>, String> {
    repository.get(repo_path).map_err(|error| error.to_string())
}

fn delete_config(repository: &dyn NotionConfigRepository, repo_path: &str) -> Result<(), String> {
    repository
        .remove(repo_path)
        .map_err(|error| error.to_string())
}

fn validate_config(
    api: &dyn NotionApiGateway,
    api_token: String,
    database_id: String,
) -> NotionValidationResult {
    if api_token.is_empty() || database_id.is_empty() {
        return NotionValidationResult::not_configured();
    }

    let config = app_config_vo::NotionRepoConfig {
        api_token,
        database_id,
        property_mapping: app_config_vo::NotionPropertyMapping::default(),
    };
    api.validate(&config)
}

fn resolve_config(
    repository: &dyn NotionConfigRepository,
    repo_path: &str,
) -> Result<app_config_vo::NotionRepoConfig, String> {
    repository
        .get(repo_path)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| NOTION_CONFIG_NOT_FOUND.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use crate::domain::app_config::error::AppConfigError;
    use crate::domain::notion::{
        NotionConfigStatus, NotionError, NotionLabelOption, NotionPropertyInfo, NotionTask,
    };

    use super::*;

    #[derive(Default)]
    struct FakeNotionConfigRepository {
        configs: Mutex<HashMap<String, app_config_vo::NotionRepoConfig>>,
    }

    impl FakeNotionConfigRepository {
        fn with_config(repo_path: &str, config: app_config_vo::NotionRepoConfig) -> Self {
            Self {
                configs: Mutex::new(HashMap::from([(repo_path.to_string(), config)])),
            }
        }
    }

    impl NotionConfigRepository for FakeNotionConfigRepository {
        fn get(
            &self,
            repo_path: &str,
        ) -> Result<Option<app_config_vo::NotionRepoConfig>, AppConfigError> {
            Ok(self.configs.lock().unwrap().get(repo_path).cloned())
        }

        fn upsert(
            &self,
            repo_path: String,
            config: app_config_vo::NotionRepoConfig,
        ) -> Result<(), AppConfigError> {
            self.configs.lock().unwrap().insert(repo_path, config);
            Ok(())
        }

        fn remove(&self, repo_path: &str) -> Result<(), AppConfigError> {
            self.configs.lock().unwrap().remove(repo_path);
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeNotionApiGateway {
        query_calls: AtomicUsize,
        label_calls: AtomicUsize,
        validate_calls: AtomicUsize,
        query_result: Mutex<Option<Result<NotionTaskPage, NotionError>>>,
        label_result: Mutex<Option<Result<Vec<NotionLabelOption>, NotionError>>>,
        validate_result: Mutex<Option<NotionValidationResult>>,
    }

    impl FakeNotionApiGateway {
        fn with_query_result(result: Result<NotionTaskPage, NotionError>) -> Self {
            Self {
                query_result: Mutex::new(Some(result)),
                ..Self::default()
            }
        }

        fn with_label_result(result: Result<Vec<NotionLabelOption>, NotionError>) -> Self {
            Self {
                label_result: Mutex::new(Some(result)),
                ..Self::default()
            }
        }

        fn with_validate_result(result: NotionValidationResult) -> Self {
            Self {
                validate_result: Mutex::new(Some(result)),
                ..Self::default()
            }
        }
    }

    impl NotionApiGateway for FakeNotionApiGateway {
        fn query_tasks(
            &self,
            _config: &app_config_vo::NotionRepoConfig,
            _query: &NotionTaskQuery,
        ) -> Result<NotionTaskPage, NotionError> {
            self.query_calls.fetch_add(1, Ordering::SeqCst);
            self.query_result.lock().unwrap().take().unwrap_or_else(|| {
                Ok(NotionTaskPage {
                    tasks: Vec::new(),
                    has_more: false,
                    next_cursor: None,
                })
            })
        }

        fn fetch_label_options(
            &self,
            _config: &app_config_vo::NotionRepoConfig,
        ) -> Result<Vec<NotionLabelOption>, NotionError> {
            self.label_calls.fetch_add(1, Ordering::SeqCst);
            self.label_result
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| Ok(Vec::new()))
        }

        fn validate(&self, _config: &app_config_vo::NotionRepoConfig) -> NotionValidationResult {
            self.validate_calls.fetch_add(1, Ordering::SeqCst);
            self.validate_result
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(NotionValidationResult::not_configured)
        }
    }

    fn config() -> app_config_vo::NotionRepoConfig {
        app_config_vo::NotionRepoConfig {
            api_token: "ntn_token".to_string(),
            database_id: "db-1".to_string(),
            property_mapping: app_config_vo::NotionPropertyMapping::default(),
        }
    }

    fn query() -> NotionTaskQuery {
        NotionTaskQuery {
            title_filter: String::new(),
            label_filters: HashMap::new(),
            cursor: None,
            page_size: None,
        }
    }

    #[test]
    fn test_task_query_configured_repoはtask_pageを返す() {
        let repo = FakeNotionConfigRepository::with_config("/repo", config());
        let expected = NotionTaskPage {
            tasks: vec![NotionTask {
                id: "page-1".to_string(),
                title: "Task".to_string(),
                url: "https://notion.so/page-1".to_string(),
                labels: HashMap::new(),
                branch_name: String::new(),
                created_at: "2026-01-01T00:00:00.000Z".to_string(),
                last_edited_at: "2026-01-02T00:00:00.000Z".to_string(),
            }],
            has_more: true,
            next_cursor: Some("cursor-1".to_string()),
        };
        let api = FakeNotionApiGateway::with_query_result(Ok(expected.clone()));

        let result = query_tasks(&repo, &api, "/repo", &query()).unwrap();

        assert_eq!(result, expected);
        assert_eq!(api.query_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_task_query_unconfigured_repoはapiを呼ばずエラーにする() {
        let repo = FakeNotionConfigRepository::default();
        let api = FakeNotionApiGateway::default();

        let result = query_tasks(&repo, &api, "/repo", &query());

        assert_eq!(result.unwrap_err(), NOTION_CONFIG_NOT_FOUND);
        assert_eq!(api.query_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_label_fetch_unconfigured_repoはapiを呼ばずエラーにする() {
        let repo = FakeNotionConfigRepository::default();
        let api = FakeNotionApiGateway::default();

        let result = fetch_label_options(&repo, &api, "/repo");

        assert_eq!(result.unwrap_err(), NOTION_CONFIG_NOT_FOUND);
        assert_eq!(api.label_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_label_fetch_configured_repoはoptionsを返す() {
        let repo = FakeNotionConfigRepository::with_config("/repo", config());
        let expected = vec![NotionLabelOption {
            property_name: "Status".to_string(),
            property_type: "status".to_string(),
            options: vec!["Todo".to_string()],
            option_ids: Vec::new(),
        }];
        let api = FakeNotionApiGateway::with_label_result(Ok(expected.clone()));

        let result = fetch_label_options(&repo, &api, "/repo").unwrap();

        assert_eq!(result, expected);
        assert_eq!(api.label_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_task_query_api_errorは文字列化して伝播する() {
        let repo = FakeNotionConfigRepository::with_config("/repo", config());
        let api = FakeNotionApiGateway::with_query_result(Err(NotionError::ApiError(
            "HTTP 500".to_string(),
        )));

        let result = query_tasks(&repo, &api, "/repo", &query());

        assert_eq!(result.unwrap_err(), "API エラー: HTTP 500");
        assert_eq!(api.query_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_config_save_get_deleteはrepositoryに反映される() {
        let repo = Arc::new(FakeNotionConfigRepository::default());

        save_config(repo.as_ref(), "/repo".to_string(), config()).unwrap();
        assert_eq!(
            get_config(repo.as_ref(), "/repo")
                .unwrap()
                .unwrap()
                .database_id,
            "db-1"
        );

        delete_config(repo.as_ref(), "/repo").unwrap();
        assert!(get_config(repo.as_ref(), "/repo").unwrap().is_none());
    }

    #[test]
    fn test_config_get_unconfigured_repoはnoneを返す() {
        let repo = FakeNotionConfigRepository::default();

        let result = get_config(&repo, "/repo").unwrap();

        assert!(result.is_none());
    }

    #[test]
    fn test_validate_空入力はnot_configuredでapiを呼ばない() {
        for (api_token, database_id) in [("", "db-1"), ("ntn_token", ""), ("", "")] {
            let api = FakeNotionApiGateway::default();

            let result = validate_config(&api, api_token.to_string(), database_id.to_string());

            assert_eq!(result.status, NotionConfigStatus::NotConfigured);
            assert!(result.properties.is_empty());
            assert_eq!(api.validate_calls.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn test_validate_空でない入力はapiへ委譲する() {
        let expected = NotionValidationResult {
            status: NotionConfigStatus::Configured,
            properties: vec![NotionPropertyInfo {
                name: "Name".to_string(),
                property_type: "title".to_string(),
                options: Vec::new(),
            }],
        };
        let api = FakeNotionApiGateway::with_validate_result(expected.clone());

        let result = validate_config(&api, "ntn_token".to_string(), "db-1".to_string());

        assert_eq!(result, expected);
        assert_eq!(api.validate_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_validate_invalid_tokenはgateway結果をそのまま返す() {
        let expected = NotionValidationResult {
            status: NotionConfigStatus::InvalidToken,
            properties: Vec::new(),
        };
        let api = FakeNotionApiGateway::with_validate_result(expected.clone());

        let result = validate_config(&api, "ntn_invalid".to_string(), "db-1".to_string());

        assert_eq!(result, expected);
        assert_eq!(api.validate_calls.load(Ordering::SeqCst), 1);
    }
}
