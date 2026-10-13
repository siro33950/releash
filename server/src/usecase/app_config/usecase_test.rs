use super::*;
use crate::domain::app_config::repository::ConfigUpdate;
use crate::domain::app_config::value_objects::{AppConfigDocument, AppSettings, TelemetryConfig};
use crate::domain::app_config::AppConfigError;
use crate::usecase::app_config::query_service::WorkflowConfigDto;

struct Config {
    document: parking_lot::Mutex<AppConfigDocument>,
    fail: std::sync::atomic::AtomicBool,
}
impl ConfigRepository for Config {
    fn load(&self) -> Result<AppConfigDocument, AppConfigError> {
        Ok(self.document.lock().clone())
    }
    fn save(&self, _: AppConfigDocument) -> Result<(), AppConfigError> {
        panic!("settings must use atomic update");
    }
    fn update(&self, update: ConfigUpdate) -> Result<(), AppConfigError> {
        if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(AppConfigError::Repository("save failed".into()));
        }
        update(&mut self.document.lock())
    }
}
impl WorkflowConfigQueryService for Config {
    fn get_workflow_config(&self) -> Result<WorkflowConfigDto, AppConfigError> {
        Ok(WorkflowConfigDto {
            approval_auto_approve: self.document.lock().workflow.approval_auto_approve,
        })
    }
}
fn repository(fail: bool) -> Arc<Config> {
    Arc::new(Config {
        document: parking_lot::Mutex::new(AppConfigDocument {
            app: AppSettings {
                close_to_tray: true,
                auto_launch: false,
                start_minimized: false,
                last_root_path: "/repo".into(),
                last_repo_paths: vec!["/repo".into()],
                external_editor: "code".into(),
                terminal_startup_command: String::new(),
            },
            workflow: WorkflowConfig {
                approval_auto_approve: false,
            },
            telemetry: TelemetryConfig {
                crash_reporting: false,
                performance_telemetry: false,
            },
        }),
        fail: fail.into(),
    })
}

#[test]
fn test_設定保存_別のclientからの一般設定保存でも登録希望を保持する() {
    // Given
    let repository = repository(false);
    let login = AppConfigUsecase::new(repository.clone(), repository.clone());
    let other_client = AppConfigUsecase::new(repository.clone(), repository.clone());
    let original = repository.load().unwrap();
    for requested in [true, false] {
        // When
        login.update_login_item_preference(requested).unwrap();
        other_client.update_app_settings(false, true).unwrap();
        // Then
        let mut expected = original.clone();
        expected.app.auto_launch = requested;
        expected.app.close_to_tray = false;
        expected.app.start_minimized = true;
        assert_eq!(repository.load().unwrap(), expected);
    }
    // When
    other_client.update_app_settings(true, false).unwrap();
    login.update_login_item_preference(true).unwrap();
    // Then
    let mut expected = original;
    expected.app.auto_launch = true;
    assert_eq!(repository.load().unwrap(), expected);
}

#[test]
fn test_設定保存_一般設定と登録希望の保存失敗を呼び出し元へ返す() {
    // Given
    let repository = repository(true);
    let original = repository.load().unwrap();
    let usecase = AppConfigUsecase::new(repository.clone(), repository.clone());
    // When
    let app_error = usecase.update_app_settings(false, true).unwrap_err();
    let login_error = usecase.update_login_item_preference(true).unwrap_err();
    let stored = repository.load().unwrap();
    // Then
    assert_eq!(app_error.to_string(), "save failed");
    assert_eq!(login_error.to_string(), "save failed");
    assert_eq!(stored, original);
}

#[test]
fn test_workflow設定_入力を保存し購読用出力で読み取る() {
    // Given
    let repository = repository(false);
    let usecase = AppConfigUsecase::new(repository.clone(), repository.clone());

    // When
    usecase
        .update_workflow_config(WorkflowConfigInput {
            approval_auto_approve: true,
        })
        .unwrap();

    // Then
    assert!(repository.load().unwrap().workflow.approval_auto_approve);
    assert!(usecase.get_workflow_config().unwrap().approval_auto_approve);
}

#[test]
fn test_terminal起動設定_保存した値を読み別設定の更新で保持する() {
    // Given
    let repository = repository(false);
    let usecase = AppConfigUsecase::new(repository.clone(), repository.clone());
    // When
    usecase
        .update_terminal_startup_command("printf ready".into())
        .unwrap();
    usecase.update_app_settings(false, true).unwrap();
    // Then
    assert_eq!(usecase.terminal_startup_command().unwrap(), "printf ready");
    assert!(usecase
        .update_terminal_startup_command("bad\0command".into())
        .is_err());
    assert_eq!(usecase.terminal_startup_command().unwrap(), "printf ready");
    let failing = repository_for_failure();
    assert!(failing
        .update_terminal_startup_command("new".into())
        .is_err());
}

fn repository_for_failure() -> AppConfigUsecase {
    let repository = repository(true);
    AppConfigUsecase::new(repository.clone(), repository)
}

struct TerminalSettingsReads(Arc<AppConfigUsecase>);

#[async_trait::async_trait]
impl crate::usecase::state_subscription::StateSubscriptionRead for TerminalSettingsReads {
    fn repositories(&self) -> Vec<String> {
        vec![]
    }
    async fn read(
        &self,
        target: &crate::usecase::state_subscription::SubscriptionTarget,
    ) -> Result<
        crate::usecase::state_subscription::StateValue,
        crate::usecase::state_subscription::StateReadError,
    > {
        assert_eq!(
            target,
            &crate::usecase::state_subscription::SubscriptionTarget::TerminalStartupCommand
        );
        self.0
            .terminal_startup_command()
            .map(crate::usecase::state_subscription::StateValue::TerminalStartupCommand)
            .map_err(crate::usecase::state_subscription::StateReadError::from_error)
    }
}

#[tokio::test]
async fn test_terminal起動設定_保存成功だけが購読へ通知され新値を配信する() {
    use crate::usecase::state_subscription::{
        StateSubscriptionUsecase, StateValue, SubscriptionTarget,
    };
    use crate::usecase::test_helpers::state_subscription::{FakeDelivery, RecordingOutput};
    // Given
    let repository = repository(false);
    let query = Arc::new(AppConfigUsecase::new(
        repository.clone(),
        repository.clone(),
    ));
    let output = Arc::new(RecordingOutput::default());
    let subscriptions = StateSubscriptionUsecase::new_with_output(
        output.clone(),
        crate::test_support::state_subscription::pending_read_driver(),
    )
    .with_reads(
        Arc::new(TerminalSettingsReads(query)),
        None,
        vec![],
        String::new(),
    );
    let usecase = AppConfigUsecase::new(repository.clone(), repository.clone())
        .with_state_publisher(subscriptions.clone());
    let target = SubscriptionTarget::TerminalStartupCommand;
    subscriptions.open_client("settings".into()).unwrap();
    subscriptions
        .start_subscription("settings", &target, &FakeDelivery)
        .await
        .unwrap();
    assert!(
        matches!(output.initial_values.lock().as_slice(), [StateValue::TerminalStartupCommand(value)] if value.is_empty())
    );
    // When
    usecase
        .update_terminal_startup_command("echo saved".into())
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), output.updated.notified())
        .await
        .unwrap();
    // Then
    let values = output.update_values.lock();
    assert!(
        matches!(values.as_slice(), [StateValue::TerminalStartupCommand(value)] if value == "echo saved")
    );
    let payload = crate::adaptor::presenter::state_subscription_wire::payload(&values[0]).unwrap();
    assert!(
        matches!(payload.value, Some(crate::adaptor::presenter::client::state_payload::Value::TerminalStartupCommand(value)) if value.value.as_deref() == Some("echo saved"))
    );
    drop(values);
    let mut changes = subscriptions.changes();
    repository
        .fail
        .store(true, std::sync::atomic::Ordering::SeqCst);
    // When
    assert!(usecase
        .update_terminal_startup_command("not saved".into())
        .is_err());
    // Then
    assert_eq!(usecase.terminal_startup_command().unwrap(), "echo saved");
    assert!(changes.try_recv().is_err());
    assert_eq!(output.update_values.lock().len(), 1);
    subscriptions
        .stop_subscription("settings", &target, &FakeDelivery)
        .await
        .unwrap();
}
