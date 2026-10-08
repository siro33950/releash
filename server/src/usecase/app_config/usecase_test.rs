use super::*;
use crate::domain::app_config::repository::ConfigUpdate;
use crate::domain::app_config::value_objects::{AppConfigDocument, AppSettings, TelemetryConfig};
use crate::domain::app_config::AppConfigError;
use crate::usecase::app_config::query_service::WorkflowConfigDto;

struct Config {
    document: parking_lot::Mutex<AppConfigDocument>,
    fail: bool,
}
impl ConfigRepository for Config {
    fn load(&self) -> Result<AppConfigDocument, AppConfigError> {
        Ok(self.document.lock().clone())
    }
    fn save(&self, _: AppConfigDocument) -> Result<(), AppConfigError> {
        panic!("settings must use atomic update");
    }
    fn update(&self, update: ConfigUpdate) -> Result<(), AppConfigError> {
        if self.fail {
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
            },
            workflow: WorkflowConfig {
                approval_auto_approve: false,
            },
            telemetry: TelemetryConfig {
                crash_reporting: false,
                performance_telemetry: false,
            },
        }),
        fail,
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
