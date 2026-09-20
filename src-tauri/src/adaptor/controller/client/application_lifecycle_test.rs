use crate::adaptor::controller::client::application_lifecycle::request_application_quit_shared;
use crate::adaptor::gateway::application_lifecycle::DaemonProcessActionPort;
use crate::adaptor::protocol::application_lifecycle_v1::{
    ApplicationQuitIntentDtoV1, ApplicationQuitOutcomeDtoV1, ApplicationQuitRequestDtoV1,
};

#[test]
fn test_終了要求_終了と再起動のコードを記録なしでdaemonへ渡す() {
    for intent in [
        ApplicationQuitIntentDtoV1::Exit { code: -7 },
        ApplicationQuitIntentDtoV1::Restart { code: -7 },
    ] {
        // Given
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let port = DaemonProcessActionPort(sender);
        // When
        let outcome =
            request_application_quit_shared(&port, ApplicationQuitRequestDtoV1 { intent });
        // Then
        assert!(matches!(outcome, Ok(ApplicationQuitOutcomeDtoV1::Accepted)));
        assert_eq!(receiver.try_recv().unwrap(), -7);
        assert!(receiver.try_recv().is_err());
    }
}

#[test]
fn test_終了要求_受信先がない場合は受付成功にしない() {
    // Given
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    drop(receiver);
    // When / Then
    assert!(request_application_quit_shared(
        &DaemonProcessActionPort(sender),
        ApplicationQuitRequestDtoV1 {
            intent: ApplicationQuitIntentDtoV1::Exit { code: 0 }
        },
    )
    .is_err());
}
