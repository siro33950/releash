use super::*;
use prost::Message;

#[test]
fn test_拒否_構造化エラーで段と理由を返す() {
    // Given
    let rejection = crate::common::concurrency::Rejection {
        level: "default",
        reason: crate::common::concurrency::RejectReason::TimedOut,
    };
    // When
    let error = crate::adaptor::presenter::connect::request_rejected(&rejection);
    // Then
    assert_eq!(error.code, connectrpc::ErrorCode::ResourceExhausted);
    assert_eq!(error.details.len(), 1);
    assert_eq!(error.details[0].type_url, "releash.client.v1.CommandError");
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(error.details[0].value.as_ref().unwrap())
        .unwrap();
    let detail = wire::CommandError::decode(bytes.as_slice()).unwrap();
    let Some(wire::command_error::Variant::Coded(detail)) = detail.variant else {
        panic!("coded error");
    };
    assert_eq!(detail.code.as_deref(), Some("CLIENT_REQUEST_LIMIT"));
    assert_eq!(
        detail.message.as_deref(),
        Some("default requests rejected: time-out")
    );
    assert_eq!(
        error.message.as_deref(),
        Some("default requests rejected: time-out")
    );
}

#[test]
fn test_状態購読配線_usecaseとcontrollerが同じ出力実装を参照する() {
    // Given
    let presenter =
        Arc::new(crate::adaptor::presenter::state_subscription::StateSubscriptionPresenter::new());
    let usecase = crate::usecase::state_subscription::StateSubscriptionUsecase::new_with_output(
        presenter.clone(),
        crate::test_support::state_subscription::read_driver(),
    );
    let deps = crate::test_support::state_subscription::deps(usecase, presenter);
    // When
    let output: Arc<dyn crate::usecase::state_subscription::StateSubscriptionOutput> =
        deps.presenter.clone();
    // Then
    assert!(Arc::ptr_eq(&deps.usecase.publisher(), &output));
}

#[tokio::test]
async fn test_状態stream開始_各段の失敗で既存clientを保持し先に開いたclientだけ戻す() {
    use crate::usecase::state_subscription::SubscriptionError;
    // Given
    for stage in 0..3 {
        let subscriptions = crate::usecase::state_subscription::StateSubscriptionUsecase::new(
            vec![],
            crate::test_support::state_subscription::read_driver(),
        );
        let deps = subscriptions.deps();
        match stage {
            0 => deps.usecase.open_client("client".into()).unwrap(),
            1 => deps.terminal.open_client("client".into()).unwrap(),
            _ => deps.presenter.open("client".into()).unwrap(),
        }
        // When
        assert!(matches!(
            deps.open_stream("client".into()),
            Err(SubscriptionError::AlreadyExists)
        ));
        // Then
        if stage == 0 {
            assert_eq!(
                deps.usecase.open_client("client".into()),
                Err(SubscriptionError::AlreadyExists)
            );
        } else {
            deps.usecase.open_client("client".into()).unwrap();
            deps.usecase.close_client("client");
        }
        if stage == 1 {
            assert_eq!(
                deps.terminal.open_client("client".into()),
                Err(SubscriptionError::AlreadyExists)
            );
        } else {
            deps.terminal.open_client("client".into()).unwrap();
            deps.terminal.close_client("client");
        }
        if stage == 2 {
            assert_eq!(
                deps.presenter.open("client".into()),
                Err(SubscriptionError::AlreadyExists)
            );
        } else {
            deps.presenter.open("client".into()).unwrap();
        }
    }
}
