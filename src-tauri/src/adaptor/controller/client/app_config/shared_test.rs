use super::*;
use crate::test_support::state_subscription::StateReadsFixture;
use crate::usecase::state_subscription::SubscriptionTarget;

fn workflow_config_controller(
    app_config_usecase: Option<std::sync::Arc<crate::usecase::app_config::AppConfigUsecase>>,
) -> (
    crate::adaptor::controller::client::workflow::tests::WorkflowTestDependencies,
    ClientCommandDispatch,
) {
    let (mut app, _data, _store) =
        crate::adaptor::controller::client::workflow::tests::make_read_only_app();
    let deps = &mut app.client;
    if let Some(usecase) = app_config_usecase {
        deps.app_config_usecase = Some(usecase);
    }
    let mut controller = ClientCommandDispatch::new(crate::usecase::daemon::DaemonUsecase(
        crate::adaptor::gateway::daemon::serving(),
    ));
    register_shared(&mut controller, deps);
    (app, controller)
}

#[tokio::test]
async fn test_workflow設定_転送要求の保存を購読の読み取りに反映する() {
    // Given
    let fixture = StateReadsFixture::new();
    let (_app, controller) = workflow_config_controller(Some(std::sync::Arc::new(
        crate::usecase::app_config::AppConfigUsecase::new(
            fixture.config.clone(),
            fixture.config.clone(),
        ),
    )));

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
    let (_app, controller) = workflow_config_controller(None);

    // When
    let result = controller
        .dispatch(wire::command_request::Command::UpdateWorkflowConfig(
            wire::UpdateWorkflowConfigRequest { workflow: None },
        ))
        .await;

    // Then
    assert!(result.is_err());
}
