use std::sync::Arc;

use crate::domain::app_config::value_objects as app_config_vo;
use crate::domain::app_config::NotionConfigRepository;
use crate::domain::notion::{
    NotionApiGateway, NotionLabelOption, NotionTaskPage, NotionTaskQuery, NotionValidationResult,
};
use crate::usecase::notion::error::NotionUsecaseError;

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
    results: parking_lot::Mutex<
        std::collections::HashMap<
            crate::usecase::state_subscription::SubscriptionTarget,
            crate::usecase::state_subscription::StateValue,
        >,
    >,
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
            results: Default::default(),
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

    pub(crate) fn refresh_subscription(
        &self,
        target: &crate::usecase::state_subscription::SubscriptionTarget,
    ) {
        use crate::usecase::state_subscription::{StateValue as V, SubscriptionTarget as T};
        // ponytail: refreshes share one lock; use per-target locks if Notion subscriptions contend.
        let mut results = self.results.lock();
        match target {
            T::NotionTasks(path, count, title, labels) => {
                let mut fetched = match results.remove(target) {
                    Some(V::NotionTasks(value)) => value,
                    _ => Default::default(),
                };
                fetched.record(query_task_list(
                    self.repository.as_ref(),
                    self.api.as_ref(),
                    path,
                    *count,
                    title.as_deref(),
                    labels,
                ));
                results.insert(target.clone(), V::NotionTasks(fetched));
            }
            T::NotionLabelOptions(path) => {
                let mut fetched = match results.remove(target) {
                    Some(V::NotionLabelOptions(value)) => value,
                    _ => Default::default(),
                };
                fetched.record(fetch_label_options(
                    self.repository.as_ref(),
                    self.api.as_ref(),
                    path,
                ));
                results.insert(target.clone(), V::NotionLabelOptions(fetched));
            }
            _ => unreachable!("Notion subscription required"),
        }
    }

    pub(crate) fn subscription_value(
        &self,
        target: &crate::usecase::state_subscription::SubscriptionTarget,
    ) -> Option<crate::usecase::state_subscription::StateValue> {
        self.results.lock().get(target).cloned()
    }

    pub(crate) fn release_subscription(
        &self,
        target: &crate::usecase::state_subscription::SubscriptionTarget,
    ) {
        self.results.lock().remove(target);
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

fn query_task_list(
    repository: &dyn NotionConfigRepository,
    api: &dyn NotionApiGateway,
    repo_path: &str,
    count: usize,
    title: Option<&str>,
    labels: &std::collections::BTreeMap<String, Vec<String>>,
) -> Result<NotionTaskPage, NotionUsecaseError> {
    let config = resolve_config(repository, repo_path)?;
    let mut query = NotionTaskQuery {
        title_filter: title.unwrap_or_default().into(),
        label_filters: labels.clone().into_iter().collect(),
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
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use crate::domain::app_config::error::AppConfigError;
    use crate::domain::notion::{
        NotionConfigStatus, NotionError, NotionLabelOption, NotionPropertyInfo, NotionTask,
    };
    use crate::usecase::notion::error::NOTION_CONFIG_NOT_FOUND;

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
        queries: Mutex<Vec<NotionTaskQuery>>,
        query_pages: Mutex<std::collections::VecDeque<Result<NotionTaskPage, NotionError>>>,
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
            query: &NotionTaskQuery,
        ) -> Result<NotionTaskPage, NotionError> {
            self.queries.lock().unwrap().push(query.clone());
            self.query_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(page) = self.query_pages.lock().unwrap().pop_front() {
                return page;
            }
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

        fn validate(
            &self,
            _config: &app_config_vo::NotionRepoConfig,
        ) -> Result<NotionValidationResult, NotionError> {
            self.validate_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self
                .validate_result
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(NotionValidationResult::not_configured))
        }
    }

    fn config() -> app_config_vo::NotionRepoConfig {
        app_config_vo::NotionRepoConfig {
            api_token: "ntn_token".to_string(),
            database_id: "db-1".to_string(),
            property_mapping: app_config_vo::NotionPropertyMapping::default(),
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
            has_more: false,
            next_cursor: None,
        };
        let api = FakeNotionApiGateway::with_query_result(Ok(expected.clone()));

        let result = query_task_list(&repo, &api, "/repo", 20, None, &Default::default()).unwrap();

        assert_eq!(result, expected);
        assert_eq!(api.query_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_task_query_unconfigured_repoはapiを呼ばずエラーにする() {
        let repo = FakeNotionConfigRepository::default();
        let api = FakeNotionApiGateway::default();

        let result = query_task_list(&repo, &api, "/repo", 20, None, &Default::default());

        assert_eq!(result.unwrap_err().to_string(), NOTION_CONFIG_NOT_FOUND);
        assert_eq!(api.query_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_label_fetch_unconfigured_repoはapiを呼ばずエラーにする() {
        let repo = FakeNotionConfigRepository::default();
        let api = FakeNotionApiGateway::default();

        let result = fetch_label_options(&repo, &api, "/repo");

        assert_eq!(result.unwrap_err().to_string(), NOTION_CONFIG_NOT_FOUND);
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

        let result = query_task_list(&repo, &api, "/repo", 20, None, &Default::default());

        assert_eq!(result.unwrap_err().to_string(), "API エラー: HTTP 500");
        assert_eq!(api.query_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_task_query_configが空文字ならapiを呼ばず未設定エラーにする() {
        let repo = FakeNotionConfigRepository::with_config(
            "/repo",
            app_config_vo::NotionRepoConfig {
                api_token: " \t".to_string(),
                database_id: "db-1".to_string(),
                property_mapping: app_config_vo::NotionPropertyMapping::default(),
            },
        );
        let api = FakeNotionApiGateway::default();

        let result = query_task_list(&repo, &api, "/repo", 20, None, &Default::default());

        assert_eq!(result.unwrap_err().to_string(), NOTION_CONFIG_NOT_FOUND);
        assert_eq!(api.query_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_label_fetch_configが空文字ならapiを呼ばず未設定エラーにする() {
        let repo = FakeNotionConfigRepository::with_config(
            "/repo",
            app_config_vo::NotionRepoConfig {
                api_token: "ntn_token".to_string(),
                database_id: "\n ".to_string(),
                property_mapping: app_config_vo::NotionPropertyMapping::default(),
            },
        );
        let api = FakeNotionApiGateway::default();

        let result = fetch_label_options(&repo, &api, "/repo");

        assert_eq!(result.unwrap_err().to_string(), NOTION_CONFIG_NOT_FOUND);
        assert_eq!(api.label_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_notion設定の保存と削除_repositoryに反映される() {
        // Given
        let repo = Arc::new(FakeNotionConfigRepository::default());

        // When
        save_config(repo.as_ref(), "/repo".to_string(), config()).unwrap();
        let saved = repo.get("/repo").unwrap().unwrap();
        // Then
        assert_eq!(saved.database_id, "db-1");

        // When
        delete_config(repo.as_ref(), "/repo").unwrap();
        let deleted = repo.get("/repo").unwrap();
        // Then
        assert!(deleted.is_none());
    }

    #[test]
    fn test_validate_空入力はnot_configuredでapiを呼ばない() {
        for (api_token, database_id) in [
            ("", "db-1"),
            ("ntn_token", ""),
            ("", ""),
            (" \t", "db-1"),
            ("ntn_token", "\n "),
        ] {
            let api = FakeNotionApiGateway::default();

            let result =
                validate_config(&api, api_token.to_string(), database_id.to_string()).unwrap();

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

        let result = validate_config(&api, "ntn_token".to_string(), "db-1".to_string()).unwrap();

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

        let result = validate_config(&api, "ntn_invalid".to_string(), "db-1".to_string()).unwrap();

        assert_eq!(result, expected);
        assert_eq!(api.validate_calls.load(Ordering::SeqCst), 1);
    }
    struct ConfigQuery;
    impl super::super::query_service::NotionConfigQueryService for ConfigQuery {
        fn get_config(&self, _: &str) -> Result<Option<NotionRepoConfigDto>, AppConfigError> {
            Ok(None)
        }
    }

    fn task(id: usize) -> NotionTask {
        NotionTask {
            id: id.to_string(),
            title: "Task".into(),
            url: String::new(),
            labels: Default::default(),
            branch_name: String::new(),
            created_at: String::new(),
            last_edited_at: String::new(),
        }
    }
    fn page(range: std::ops::Range<usize>, more: bool, cursor: Option<&str>) -> NotionTaskPage {
        NotionTaskPage {
            tasks: range.map(task).collect(),
            has_more: more,
            next_cursor: cursor.map(Into::into),
        }
    }

    #[test]
    fn test_notion一覧_先頭から件数までcursorをたどり絞り込みを各ページへ渡す() {
        // Given
        let repo = FakeNotionConfigRepository::with_config("/repo", config());
        let api = FakeNotionApiGateway::default();
        *api.query_pages.lock().unwrap() = [
            Ok(page(0..20, true, Some("next"))),
            Ok(page(20..40, true, Some("end"))),
        ]
        .into();
        let labels = std::collections::BTreeMap::from([("Status".into(), vec!["Todo".into()])]);
        // When
        let result = query_task_list(&repo, &api, "/repo", 40, Some("Task"), &labels).unwrap();
        // Then
        assert_eq!(result.tasks, (0..40).map(task).collect::<Vec<_>>());
        assert!(result.has_more);
        assert!(result.next_cursor.is_none());
        let queries = api.queries.lock().unwrap();
        assert_eq!(queries.len(), 2);
        assert_eq!(queries[0].cursor, None);
        assert_eq!(queries[1].cursor.as_deref(), Some("next"));
        for query in queries.iter() {
            assert_eq!(query.page_size, Some(20));
            assert_eq!(query.title_filter, "Task");
            assert_eq!(
                query.label_filters.get("Status"),
                Some(&vec!["Todo".into()])
            );
        }
    }

    #[test]
    fn test_notion一覧_最終ページと件数途中でhas_moreを返し欠損cursorを伝える() {
        // Given / When / Then
        let repo = FakeNotionConfigRepository::with_config("/repo", config());
        for (count, expected_more) in [(20, false), (10, true), (40, false)] {
            let api = FakeNotionApiGateway::with_query_result(Ok(page(0..20, false, None)));
            let result =
                query_task_list(&repo, &api, "/repo", count, None, &Default::default()).unwrap();
            assert_eq!(result.tasks.len(), count.min(20));
            assert_eq!(result.has_more, expected_more);
        }
        let api = FakeNotionApiGateway::with_query_result(Ok(page(0..20, true, None)));
        assert!(matches!(
            query_task_list(&repo, &api, "/repo", 40, None, &Default::default()),
            Err(NotionUsecaseError::Notion(NotionError::ParseError(_)))
        ));
        let api = FakeNotionApiGateway::default();
        *api.query_pages.lock().unwrap() = [
            Ok(page(0..20, true, Some("next"))),
            Ok(page(20..40, true, Some("next"))),
        ]
        .into();
        assert!(query_task_list(&repo, &api, "/repo", 60, None, &Default::default()).is_err());
    }

    #[test]
    fn test_notion購読_最後の値と失敗を保持して設定不足を区別し回復と解放を行う() {
        use crate::usecase::state_subscription::{StateValue as V, SubscriptionTarget as T};
        // Given
        let repo = Arc::new(FakeNotionConfigRepository::with_config("/repo", config()));
        let api = Arc::new(FakeNotionApiGateway::with_query_result(Ok(page(
            0..1,
            false,
            None,
        ))));
        let notion = NotionUsecase::new(repo.clone(), Arc::new(ConfigQuery), api.clone());
        let tasks = T::NotionTasks("/repo".into(), 20, None, Default::default());
        let labels = T::NotionLabelOptions("/repo".into());
        notion.refresh_subscription(&tasks);
        notion.refresh_subscription(&labels);
        // When
        *api.query_result.lock().unwrap() = Some(Err(NotionError::ApiError("offline".into())));
        *api.label_result.lock().unwrap() = Some(Err(NotionError::ApiError("offline".into())));
        notion.refresh_subscription(&tasks);
        notion.refresh_subscription(&labels);
        // Then
        let V::NotionTasks(result) = notion.subscription_value(&tasks).unwrap() else {
            panic!()
        };
        assert_eq!(result.value.unwrap().tasks, vec![task(0)]);
        assert!(matches!(result.error, Some(NotionUsecaseError::Notion(_))));
        let V::NotionLabelOptions(result) = notion.subscription_value(&labels).unwrap() else {
            panic!()
        };
        assert_eq!(result.value, Some(vec![]));
        assert!(matches!(result.error, Some(NotionUsecaseError::Notion(_))));
        notion.delete_config("/repo").unwrap();
        let calls = api.query_calls.load(Ordering::SeqCst);
        let label_calls = api.label_calls.load(Ordering::SeqCst);
        notion.refresh_subscription(&tasks);
        notion.refresh_subscription(&labels);
        let V::NotionTasks(result) = notion.subscription_value(&tasks).unwrap() else {
            panic!()
        };
        assert_eq!(result.value.unwrap().tasks, vec![task(0)]);
        assert_eq!(result.error, Some(NotionUsecaseError::ConfigNotFound));
        let V::NotionLabelOptions(result) = notion.subscription_value(&labels).unwrap() else {
            panic!()
        };
        assert_eq!(result.error, Some(NotionUsecaseError::ConfigNotFound));
        assert_eq!(api.query_calls.load(Ordering::SeqCst), calls);
        assert_eq!(api.label_calls.load(Ordering::SeqCst), label_calls);
        notion.save_config("/repo".into(), config()).unwrap();
        notion.refresh_subscription(&tasks);
        let V::NotionTasks(result) = notion.subscription_value(&tasks).unwrap() else {
            panic!()
        };
        assert!(result.error.is_none());
        notion.release_subscription(&tasks);
        notion.release_subscription(&labels);
        assert!(notion.subscription_value(&tasks).is_none());
        assert!(notion.subscription_value(&labels).is_none());
    }

    #[test]
    fn test_notion設定_保存と削除は対象repoの購読へ通知する() {
        // Given
        let subscriptions = crate::test_support::state_subscription::test_subscriptions();
        let mut changes = subscriptions.changes();
        let notion = NotionUsecase::new(
            Arc::new(FakeNotionConfigRepository::default()),
            Arc::new(ConfigQuery),
            Arc::new(FakeNotionApiGateway::default()),
        )
        .with_state_publisher(subscriptions);
        // When / Then
        for save in [true, false] {
            if save {
                notion.save_config("/repo".into(), config()).unwrap();
            } else {
                notion.delete_config("/repo").unwrap();
            }
            assert_eq!(
                changes.try_recv().unwrap(),
                crate::usecase::state_subscription::StateChangeSource::AppConfig
            );
            assert_eq!(
                changes.try_recv().unwrap(),
                crate::usecase::state_subscription::StateChangeSource::NotionConfig("/repo".into())
            );
        }
    }
}
