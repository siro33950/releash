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
    configs: Mutex<Vec<app_config_vo::NotionRepoConfig>>,
    database_pages: Mutex<HashMap<String, NotionTaskPage>>,
    database_labels: Mutex<HashMap<String, Vec<NotionLabelOption>>>,
    gate: Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>,
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
        config: &app_config_vo::NotionRepoConfig,
        query: &NotionTaskQuery,
    ) -> Result<NotionTaskPage, NotionError> {
        self.configs.lock().unwrap().push(config.clone());
        if let Some((entered, resume)) = self.gate.lock().unwrap().take() {
            entered.send(()).unwrap();
            resume
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        }
        if let Some(page) = self.database_pages.lock().unwrap().get(&config.database_id) {
            self.query_calls.fetch_add(1, Ordering::SeqCst);
            return Ok(page.clone());
        }
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
        config: &app_config_vo::NotionRepoConfig,
    ) -> Result<Vec<NotionLabelOption>, NotionError> {
        self.configs.lock().unwrap().push(config.clone());
        if let Some(labels) = self
            .database_labels
            .lock()
            .unwrap()
            .get(&config.database_id)
        {
            self.label_calls.fetch_add(1, Ordering::SeqCst);
            return Ok(labels.clone());
        }
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
    let api =
        FakeNotionApiGateway::with_query_result(Err(NotionError::ApiError("HTTP 500".to_string())));

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

        let result = validate_config(&api, api_token.to_string(), database_id.to_string()).unwrap();

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
    let labels = std::collections::BTreeMap::from([(
        "Status".into(),
        std::collections::BTreeSet::from(["Todo".into()]),
    )]);
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
fn test_notion一覧_最終ページで件数に達したら続き無しを返す() {
    // Given
    let repo = FakeNotionConfigRepository::with_config("/repo", config());
    let api = FakeNotionApiGateway::with_query_result(Ok(page(0..20, false, None)));
    // When
    let result = query_task_list(&repo, &api, "/repo", 20, None, &Default::default()).unwrap();
    let shorter = query_task_list(
        &repo,
        &FakeNotionApiGateway::with_query_result(Ok(page(0..20, false, None))),
        "/repo",
        40,
        None,
        &Default::default(),
    )
    .unwrap();
    // Then
    assert_eq!(result.tasks, (0..20).map(task).collect::<Vec<_>>());
    assert_eq!(result.has_more, false);
    assert_eq!(shorter.tasks, (0..20).map(task).collect::<Vec<_>>());
    assert!(!shorter.has_more);
}

#[test]
fn test_notion一覧_件数の途中で切ったら続き有りを返す() {
    // Given
    let repo = FakeNotionConfigRepository::with_config("/repo", config());
    let api = FakeNotionApiGateway::with_query_result(Ok(page(0..20, false, None)));
    // When
    let result = query_task_list(&repo, &api, "/repo", 10, None, &Default::default()).unwrap();
    // Then
    assert_eq!(result.tasks, (0..10).map(task).collect::<Vec<_>>());
    assert_eq!(result.has_more, true);
}

#[test]
fn test_notion一覧_続きがあるのにcursorが無ければ取得の失敗にする() {
    // Given
    let repo = FakeNotionConfigRepository::with_config("/repo", config());
    let api = FakeNotionApiGateway::default();
    *api.query_pages.lock().unwrap() = [Ok(page(0..20, true, None))].into();
    // When
    let result = query_task_list(&repo, &api, "/repo", 40, None, &Default::default());
    // Then
    assert!(matches!(
        result,
        Err(NotionUsecaseError::Notion(NotionError::ParseError(_)))
    ));
}

#[test]
fn test_notion一覧_同じcursorが繰り返されたら取得の失敗にする() {
    // Given
    let repo = FakeNotionConfigRepository::with_config("/repo", config());
    let api = FakeNotionApiGateway::default();
    *api.query_pages.lock().unwrap() = [
        Ok(page(0..20, true, Some("next"))),
        Ok(page(20..40, true, Some("next"))),
    ]
    .into();
    // When
    let result = query_task_list(&repo, &api, "/repo", 60, None, &Default::default());
    // Then
    assert!(matches!(
        result,
        Err(NotionUsecaseError::Notion(NotionError::ParseError(_)))
    ));
}

fn label_options() -> Vec<NotionLabelOption> {
    vec![
        NotionLabelOption {
            property_name: "Status".into(),
            property_type: "status".into(),
            options: vec!["Todo".into(), "Done".into()],
            option_ids: vec!["todo".into(), "done".into()],
        },
        NotionLabelOption {
            property_name: "Tags".into(),
            property_type: "multi_select".into(),
            options: vec!["Bug".into()],
            option_ids: vec!["bug".into()],
        },
    ]
}

fn notion_fixture() -> (NotionUsecase, Arc<FakeNotionApiGateway>) {
    let repo = Arc::new(FakeNotionConfigRepository::with_config("/repo", config()));
    let api = Arc::new(FakeNotionApiGateway::with_query_result(Ok(page(
        0..1,
        false,
        None,
    ))));
    *api.label_result.lock().unwrap() = Some(Ok(label_options()));
    (
        NotionUsecase::new(repo, Arc::new(ConfigQuery), api.clone()),
        api,
    )
}

#[test]
fn test_notion購読_タスクの取得失敗で前の一覧と失敗を両方持つ() {
    // Given
    let (notion, api) = notion_fixture();
    notion.refresh_tasks("/repo", 20, None, &Default::default());
    *api.query_result.lock().unwrap() = Some(Err(NotionError::ApiError("offline".into())));
    // When
    notion.refresh_tasks("/repo", 20, None, &Default::default());
    // Then
    let result = notion
        .cached_tasks("/repo", 20, None, &Default::default())
        .unwrap();
    assert_eq!(result.value, Some(page(0..1, false, None)));
    assert!(matches!(result.error, Some(NotionUsecaseError::Notion(_))));
}

#[test]
fn test_notion購読_ラベルの取得失敗で前の選択肢と失敗を両方持つ() {
    // Given
    let (notion, api) = notion_fixture();
    notion.refresh_label_options("/repo");
    *api.label_result.lock().unwrap() = Some(Err(NotionError::ApiError("offline".into())));
    // When
    notion.refresh_label_options("/repo");
    // Then
    let result = notion.cached_label_options("/repo").unwrap();
    assert_eq!(result.value, Some(label_options()));
    assert!(matches!(result.error, Some(NotionUsecaseError::Notion(_))));
}

#[test]
fn test_notion購読_設定が揃っていなければapiを呼ばず設定不足の失敗を持つ() {
    // Given
    let (notion, api) = notion_fixture();
    notion.refresh_tasks("/repo", 20, None, &Default::default());
    notion.refresh_label_options("/repo");
    let calls = (
        api.query_calls.load(Ordering::SeqCst),
        api.label_calls.load(Ordering::SeqCst),
    );
    notion.delete_config("/repo").unwrap();
    // When
    notion.refresh_tasks("/repo", 20, None, &Default::default());
    notion.refresh_label_options("/repo");
    // Then
    let tasks = notion
        .cached_tasks("/repo", 20, None, &Default::default())
        .unwrap();
    let labels = notion.cached_label_options("/repo").unwrap();
    assert_eq!(tasks.value, Some(page(0..1, false, None)));
    assert_eq!(labels.value, Some(label_options()));
    assert_eq!(tasks.error, Some(NotionUsecaseError::ConfigNotFound));
    assert_eq!(labels.error, Some(NotionUsecaseError::ConfigNotFound));
    assert_eq!(
        (
            api.query_calls.load(Ordering::SeqCst),
            api.label_calls.load(Ordering::SeqCst)
        ),
        calls
    );
}

#[test]
fn test_notion購読_設定が戻れば次の取り直しで失敗が消える() {
    // Given
    let (notion, _) = notion_fixture();
    notion.delete_config("/repo").unwrap();
    notion.refresh_tasks("/repo", 20, None, &Default::default());
    notion.refresh_label_options("/repo");
    // When
    notion.save_config("/repo".into(), config()).unwrap();
    notion.refresh_tasks("/repo", 20, None, &Default::default());
    notion.refresh_label_options("/repo");
    // Then
    assert!(notion
        .cached_tasks("/repo", 20, None, &Default::default())
        .unwrap()
        .error
        .is_none());
    assert!(notion
        .cached_label_options("/repo")
        .unwrap()
        .error
        .is_none());
}

#[test]
fn test_notion購読_解放した対象の結果は読めない() {
    // Given
    let (notion, _) = notion_fixture();
    notion.refresh_tasks("/repo", 20, None, &Default::default());
    notion.refresh_label_options("/repo");
    // When
    notion.release_tasks("/repo", 20, None, &Default::default());
    notion.release_label_options("/repo");
    // Then
    assert!(notion
        .cached_tasks("/repo", 20, None, &Default::default())
        .is_none());
    assert!(notion.cached_label_options("/repo").is_none());
}

#[test]
fn test_notion購読_別の設定を保存すると新しい設定で取り直した値になる() {
    // Given
    let (notion, api) = notion_fixture();
    let a = config();
    let mut b = config();
    b.database_id = "db-2".into();
    b.api_token = "new-token".into();
    b.property_mapping.title = "New title".into();
    let mut new_labels = label_options();
    new_labels[0].options = vec!["New".into()];
    *api.database_pages.lock().unwrap() = HashMap::from([
        (a.database_id.clone(), page(0..1, false, None)),
        (b.database_id.clone(), page(10..12, false, None)),
    ]);
    *api.database_labels.lock().unwrap() = HashMap::from([
        (a.database_id.clone(), label_options()),
        (b.database_id.clone(), new_labels.clone()),
    ]);
    notion.save_config("/repo".into(), a).unwrap();
    notion.refresh_tasks("/repo", 20, None, &Default::default());
    notion.refresh_label_options("/repo");
    let previous_tasks = notion
        .cached_tasks("/repo", 20, None, &Default::default())
        .unwrap();
    let previous_labels = notion.cached_label_options("/repo").unwrap();
    // When
    notion.save_config("/repo".into(), b.clone()).unwrap();
    notion.refresh_tasks("/repo", 20, None, &Default::default());
    notion.refresh_label_options("/repo");
    // Then
    assert_eq!(&api.configs.lock().unwrap()[2..], &[b.clone(), b]);
    assert_eq!(previous_tasks.value, Some(page(0..1, false, None)));
    assert_eq!(previous_labels.value, Some(label_options()));
    assert_eq!(
        notion
            .cached_tasks("/repo", 20, None, &Default::default())
            .unwrap()
            .value,
        Some(page(10..12, false, None))
    );
    assert_eq!(
        notion.cached_label_options("/repo").unwrap().value,
        Some(new_labels)
    );
}

#[test]
fn test_notion設定_保存で対象repoの購読へ通知する() {
    // Given
    let subscriptions = crate::test_support::state_subscription::test_subscriptions();
    let mut changes = subscriptions.changes();
    let (notion, _) = notion_fixture();
    let notion = notion.with_state_publisher(subscriptions);
    // When
    notion.save_config("/repo".into(), config()).unwrap();
    // Then
    assert_eq!(
        changes.try_recv().unwrap(),
        crate::usecase::state_subscription::StateChangeSource::AppConfig
    );
    assert_eq!(
        changes.try_recv().unwrap(),
        crate::usecase::state_subscription::StateChangeSource::NotionConfig("/repo".into())
    );
}

#[test]
fn test_notion設定_削除で対象repoの購読へ通知する() {
    // Given
    let subscriptions = crate::test_support::state_subscription::test_subscriptions();
    let mut changes = subscriptions.changes();
    let (notion, _) = notion_fixture();
    let notion = notion.with_state_publisher(subscriptions);
    // When
    notion.delete_config("/repo").unwrap();
    // Then
    assert_eq!(
        changes.try_recv().unwrap(),
        crate::usecase::state_subscription::StateChangeSource::AppConfig
    );
    assert_eq!(
        changes.try_recv().unwrap(),
        crate::usecase::state_subscription::StateChangeSource::NotionConfig("/repo".into())
    );
}

fn blocked_fetch(
    notion: Arc<NotionUsecase>,
    api: &FakeNotionApiGateway,
) -> (std::thread::JoinHandle<()>, std::sync::mpsc::Sender<()>) {
    let (entered, wait) = std::sync::mpsc::channel();
    let (resume, paused) = std::sync::mpsc::channel();
    *api.gate.lock().unwrap() = Some((entered, paused));
    let worker =
        std::thread::spawn(move || notion.refresh_tasks("/repo", 20, None, &Default::default()));
    wait.recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    (worker, resume)
}

#[test]
fn test_notion購読_取得を待つ間も別の対象の読み取りと解放を待たせない() {
    // Given
    let (notion, api) = notion_fixture();
    let notion = Arc::new(notion);
    notion.refresh_label_options("/repo");
    let (worker, resume) = blocked_fetch(notion.clone(), &api);
    let (completed, wait) = std::sync::mpsc::channel();
    // When
    let reader = std::thread::spawn(move || {
        let result = notion.cached_label_options("/repo");
        notion.release_label_options("/repo");
        let released = notion.cached_label_options("/repo");
        completed.send((result, released)).unwrap();
    });
    let result = wait.recv_timeout(std::time::Duration::from_secs(1));
    resume.send(()).unwrap();
    worker.join().unwrap();
    reader.join().unwrap();
    // Then
    let (value, released) = result.unwrap();
    assert_eq!(value.unwrap().value, Some(label_options()));
    assert!(released.is_none());
}

#[test]
fn test_notion購読_取得中に同じ対象を解放したら結果を書き戻さない() {
    // Given
    let (notion, api) = notion_fixture();
    let notion = Arc::new(notion);
    let (worker, resume) = blocked_fetch(notion.clone(), &api);
    let (completed, wait) = std::sync::mpsc::channel();
    let releasing = notion.clone();
    // When
    let release = std::thread::spawn(move || {
        releasing.release_tasks("/repo", 20, None, &Default::default());
        completed.send(()).unwrap();
    });
    let result = wait.recv_timeout(std::time::Duration::from_secs(1));
    resume.send(()).unwrap();
    worker.join().unwrap();
    release.join().unwrap();
    // Then
    result.unwrap();
    assert!(notion
        .cached_tasks("/repo", 20, None, &Default::default())
        .is_none());
}
