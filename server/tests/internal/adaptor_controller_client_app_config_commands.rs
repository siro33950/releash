use releashd::test_support::integration::settings::update_workflow_config_shared;
use releashd::test_support::integration::settings::AppConfig;
use releashd::test_support::integration::settings::AppConfigUsecase;
use releashd::test_support::integration::settings::ReleashConfig;
use std::sync::Arc;

#[tokio::test]
pub async fn test_workflow設定_転送入力をcontrollerで保存し購読用読み取りへ反映する() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    let config = Arc::new(AppConfig::new(
        ReleashConfig::default(),
        dir.path().join("config.toml"),
    ));
    let usecase = Arc::new(AppConfigUsecase::new(config.clone(), config));
    let input = releashd::test_support::integration::settings::WorkflowConfigInput::try_from(
        releashd::test_support::integration::wire::WorkflowSection {
            approval_auto_approve: Some(true),
        },
    )
    .unwrap();

    // When
    update_workflow_config_shared(&usecase, input)
        .await
        .unwrap();

    // Then
    assert!(usecase.get_workflow_config().unwrap().approval_auto_approve);
}
