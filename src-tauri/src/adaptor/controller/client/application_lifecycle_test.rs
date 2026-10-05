use crate::adaptor::controller::client::application_lifecycle::request_application_quit_shared;
use crate::adaptor::presenter::application_lifecycle_v1::{
    ApplicationQuitIntentDtoV1, ApplicationQuitOutcomeDtoV1, ApplicationQuitRequestDtoV1,
};

#[tokio::test]
async fn test_終了要求_終了と再起動のコードを記録なしでdaemonへ渡す() {
    for intent in [
        ApplicationQuitIntentDtoV1::Exit { code: -7 },
        ApplicationQuitIntentDtoV1::Restart { code: -7 },
    ] {
        // Given
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let daemon =
            crate::usecase::daemon::DaemonUsecase(crate::adaptor::gateway::daemon::serving());
        // When
        let outcome = request_application_quit_shared(
            &daemon,
            &sender,
            ApplicationQuitRequestDtoV1 { intent },
        )
        .await;
        // Then
        assert!(matches!(outcome, Ok(ApplicationQuitOutcomeDtoV1::Accepted)));
        assert_eq!(receiver.try_recv().unwrap(), -7);
        assert!(receiver.try_recv().is_err());
    }
}

#[tokio::test]
async fn test_終了要求_受信先がない場合は受付成功にしない() {
    // Given
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    drop(receiver);
    // When / Then
    assert!(request_application_quit_shared(
        &crate::usecase::daemon::DaemonUsecase(crate::adaptor::gateway::daemon::serving()),
        &sender,
        ApplicationQuitRequestDtoV1 {
            intent: ApplicationQuitIntentDtoV1::Exit { code: 0 }
        },
    )
    .await
    .is_err());
}

#[tokio::test]
async fn test_終了要求_受信channelが閉じても重複は成功し最初のcodeだけを送る() {
    // Given
    let daemon = crate::usecase::daemon::DaemonUsecase(crate::adaptor::gateway::daemon::serving());
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    // When
    request_application_quit_shared(
        &daemon,
        &sender,
        ApplicationQuitRequestDtoV1 {
            intent: ApplicationQuitIntentDtoV1::Exit { code: 23 },
        },
    )
    .await
    .unwrap();
    assert_eq!(receiver.try_recv().unwrap(), 23);
    receiver.close();
    // Then
    for code in 0..100 {
        assert!(matches!(
            request_application_quit_shared(
                &daemon,
                &sender,
                ApplicationQuitRequestDtoV1 {
                    intent: ApplicationQuitIntentDtoV1::Restart { code }
                }
            )
            .await,
            Ok(ApplicationQuitOutcomeDtoV1::Accepted)
        ));
    }
    assert_eq!(receiver.len(), 0);
}

#[tokio::test]
async fn test_終了要求_commandの同期が止まっても受理して終了を通知する() {
    // Given
    let repository = crate::adaptor::gateway::daemon::serving();
    let admission = repository.admission().await;
    let daemon = crate::usecase::daemon::DaemonUsecase(repository.clone());
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    // When
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        request_application_quit_shared(
            &daemon,
            &sender,
            ApplicationQuitRequestDtoV1 {
                intent: ApplicationQuitIntentDtoV1::Exit { code: 23 },
            },
        ),
    )
    .await
    .unwrap()
    .unwrap();
    // Then
    assert!(matches!(outcome, ApplicationQuitOutcomeDtoV1::Accepted));
    assert_eq!(receiver.try_recv().unwrap(), 23);
    assert!(!admission.admits(crate::domain::daemon::DaemonRequest::Operation));
}
