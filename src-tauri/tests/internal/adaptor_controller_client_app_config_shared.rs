use crate::state_subscription_reads::Fixture as StateReadsFixture;
use releash_lib::test_support::integration::settings::register_shared;
use releash_lib::test_support::integration::subscriptions::SubscriptionTarget;
use releash_lib::test_support::integration::transport::ClientCommandDispatch;
use releash_lib::test_support::integration::wire;

fn workflow_config_controller(
    app_config_usecase: Option<
        std::sync::Arc<releash_lib::test_support::integration::settings::AppConfigUsecase>,
    >,
) -> (
    crate::adaptor_controller_client_workflow_mod::tests::WorkflowTestDependencies,
    ClientCommandDispatch,
) {
    let (mut app, _data, _store) =
        crate::adaptor_controller_client_workflow_mod::tests::make_read_only_app();
    let deps = &mut app.client;
    if let Some(usecase) = app_config_usecase {
        deps.app_config_usecase = Some(usecase);
    }
    let mut controller = ClientCommandDispatch::new(
        releash_lib::test_support::integration::daemon::DaemonUsecase::test_with_repository(
            releash_lib::test_support::integration::daemon::serving(),
        ),
    );
    register_shared(&mut controller, deps);
    (app, controller)
}

#[tokio::test]
pub async fn test_workflow設定_転送要求の保存を購読の読み取りに反映する() {
    // Given
    let fixture = StateReadsFixture::new();
    let (_app, controller) = workflow_config_controller(Some(std::sync::Arc::new(
        releash_lib::test_support::integration::settings::AppConfigUsecase::new(
            fixture.config.clone(),
            fixture.config.clone(),
        ),
    )));

    // When
    let saved = controller
        .dispatch(wire::command_request::Command::UpdateWorkflowConfig(
            wire::UpdateWorkflowConfigRequest {
                workflow: Some(wire::WorkflowSection {
                    approval_auto_approve: Some(true),
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
    let payload = releash_lib::test_support::integration::platform::payload(&output).unwrap();
    assert!(matches!(
        payload.value,
        Some(wire::state_payload::Value::WorkflowConfig(wire::WorkflowSection {
            approval_auto_approve: Some(value),
        })) if value
    ));
}

#[tokio::test]
pub async fn test_workflow設定_workflowが欠けた転送要求を拒否する() {
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
