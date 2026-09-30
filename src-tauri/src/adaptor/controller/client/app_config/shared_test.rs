use super::*;
use crate::test_support::state_subscription::StateReadsFixture;
use crate::usecase::application_startup::ApplicationStartupAuthority;
use crate::usecase::state_subscription::SubscriptionTarget;

#[tokio::test]
async fn test_workflow設定_登録済みcontrollerが転送要求を保存し欠落を拒否する() {
    // Given
    let fixture = StateReadsFixture::new();
    let usecase = std::sync::Arc::new(crate::usecase::app_config::AppConfigUsecase::new(
        fixture.config.clone(),
        fixture.config.clone(),
    ));
    let mut controller =
        ClientCommandDispatch::new(std::sync::Arc::new(ApplicationStartupAuthority::ready()));
    register_workflow_config(&mut controller, Some(usecase.clone()));

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
    let output = fixture
        .reads
        .read(&SubscriptionTarget::WorkflowConfig)
        .await
        .unwrap();
    let payload = crate::adaptor::presenter::state_subscription_wire::payload(&output).unwrap();
    let missing = controller
        .dispatch(wire::command_request::Command::UpdateWorkflowConfig(
            wire::UpdateWorkflowConfigRequest { workflow: None },
        ))
        .await;

    // Then
    assert!(saved.is_ok());
    assert!(matches!(
        payload.value,
        Some(wire::state_payload::Value::WorkflowConfig(wire::WorkflowSection {
            approval_auto_approve: Some(value),
        })) if bool::try_from(value).unwrap()
    ));
    assert!(missing.is_err());
}
