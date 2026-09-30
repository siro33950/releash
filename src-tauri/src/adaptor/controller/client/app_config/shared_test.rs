use super::*;
use crate::test_support::state_subscription::StateReadsFixture;
use crate::usecase::application_startup::ApplicationStartupAuthority;
use crate::usecase::state_subscription::SubscriptionTarget;
use tauri::Manager;

#[tokio::test]
async fn test_workflow設定_転送要求の保存を購読の読み取りに反映する() {
    // Given
    let fixture = StateReadsFixture::new();
    let (app, _data, _store) =
        crate::adaptor::controller::client::workflow::tests::make_read_only_app();
    app.manage(std::sync::Arc::new(
        crate::infrastructure::file_watcher::FileWatcherManager::default(),
    ));
    let mut deps = crate::desktop_test_support::build_client_dependencies(app.handle());
    deps.app_config_usecase = Some(std::sync::Arc::new(
        crate::usecase::app_config::AppConfigUsecase::new(
            fixture.config.clone(),
            fixture.config.clone(),
        ),
    ));
    let mut controller =
        ClientCommandDispatch::new(std::sync::Arc::new(ApplicationStartupAuthority::ready()));
    register_shared(&mut controller, &deps);

    // When
    let saved = controller
        .dispatch(wire::command_request::Command::UpdateWorkflowConfig(
            wire::UpdateWorkflowConfigRequest {
                workflow: Some(wire::WorkflowSection {
                    approval_auto_approve: Some(true.into()),
                }),
            },
        ))
        .await;
    // Then
    assert!(saved.is_ok());
    let output = fixture
        .reads
        .read(&SubscriptionTarget::WorkflowConfig)
        .await
        .unwrap();
    let payload = crate::adaptor::presenter::state_subscription_wire::payload(&output).unwrap();
    assert!(matches!(
        payload.value,
        Some(wire::state_payload::Value::WorkflowConfig(wire::WorkflowSection {
            approval_auto_approve: Some(value),
        })) if bool::try_from(value).unwrap()
    ));
}

#[tokio::test]
async fn test_workflow設定_workflowが欠けた転送要求を拒否する() {
    // Given
    let (app, _data, _store) =
        crate::adaptor::controller::client::workflow::tests::make_read_only_app();
    app.manage(std::sync::Arc::new(
        crate::infrastructure::file_watcher::FileWatcherManager::default(),
    ));
    let deps = crate::desktop_test_support::build_client_dependencies(app.handle());
    let mut controller =
        ClientCommandDispatch::new(std::sync::Arc::new(ApplicationStartupAuthority::ready()));
    register_shared(&mut controller, &deps);

    // When
    let result = controller
        .dispatch(wire::command_request::Command::UpdateWorkflowConfig(
            wire::UpdateWorkflowConfigRequest { workflow: None },
        ))
        .await;

    // Then
    assert!(result.is_err());
}
