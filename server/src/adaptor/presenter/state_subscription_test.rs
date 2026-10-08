use super::*;
use crate::test_support::state_subscription::WakeFlag;
use crate::usecase::state_subscription::StateSubscriptionDelivery;
use crate::usecase::state_subscription::StateSubscriptionUsecase;
use futures_util::StreamExt;

#[test]
fn test_配信失敗_usecaseの失敗分類へ意味を保って変換する() {
    use crate::infrastructure::state_subscription::SubscriptionError as DeliveryError;
    use crate::usecase::state_subscription::SubscriptionError;

    let cases = [
        (DeliveryError::InvalidId, SubscriptionError::InvalidId),
        (
            DeliveryError::AlreadyExists,
            SubscriptionError::AlreadyExists,
        ),
        (DeliveryError::StreamEnded, SubscriptionError::StreamEnded),
        (
            DeliveryError::UnknownTarget,
            SubscriptionError::UnknownTarget,
        ),
        (
            DeliveryError::VersionExhausted,
            SubscriptionError::VersionExhausted,
        ),
        (
            DeliveryError::SnapshotRequired,
            SubscriptionError::SnapshotRequired,
        ),
    ];
    for (source, expected) in cases {
        assert_eq!(SubscriptionError::from(source), expected);
    }
}

#[tokio::test]
async fn test_購読開始失敗_対象削除を待機中streamへ通知する() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Waker};

    // Given
    let presenter = Arc::new(StateSubscriptionPresenter::new());
    let target = SubscriptionTarget::BranchBase("/repo".into(), "main".into()).to_string();
    let payload = crate::adaptor::presenter::state_subscription_wire::payload(
        &StateValue::RepositoryPaths(vec![]),
    )
    .unwrap();
    presenter
        .runtime
        .update(|state| {
            state
                .register(
                    target.clone(),
                    PublishedState::from(payload),
                    Delivery::Full,
                )
                .map(|_| true)
        })
        .unwrap();
    let usecase = StateSubscriptionUsecase::new_with_output(
        presenter.clone(),
        crate::test_support::state_subscription::read_driver(),
    );
    let mut stream = Box::pin(
        crate::test_support::state_subscription::deps(usecase, presenter.clone())
            .stream("waiting".into())
            .unwrap(),
    );
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Ready)
    ));
    let flag = Arc::new(WakeFlag(AtomicBool::new(false)));
    let waker = Waker::from(flag.clone());
    assert!(stream
        .as_mut()
        .poll_next(&mut Context::from_waker(&waker))
        .is_pending());

    // When
    assert_eq!(
        presenter
            .reserve_delivery("absent", "subscription", &target, None)
            .err(),
        Some(SubscriptionError::StreamEnded)
    );

    // Then
    assert!(!presenter.runtime.inspect(|state| state.registered(&target)));
    assert!(flag.0.load(Ordering::SeqCst));
}

#[tokio::test]
async fn test_購読再開始_状態不変なら通知せず初回開始だけ通知する() {
    use crate::infrastructure::state_subscription::Event;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Waker};

    let presenter = Arc::new(StateSubscriptionPresenter::new());
    let usecase = StateSubscriptionUsecase::new_with_output(
        presenter.clone(),
        crate::test_support::state_subscription::read_driver(),
    );
    let mut stream = Box::pin(
        crate::test_support::state_subscription::deps(usecase, presenter.clone())
            .stream("client".into())
            .unwrap(),
    );
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Ready)
    ));
    let flag = Arc::new(WakeFlag(AtomicBool::new(false)));
    let waker = Waker::from(flag.clone());
    assert!(stream
        .as_mut()
        .poll_next(&mut Context::from_waker(&waker))
        .is_pending());

    let target = SubscriptionTarget::RepositoryPaths.to_string();
    presenter
        .publish_initial(
            &SubscriptionTarget::RepositoryPaths,
            StateValue::RepositoryPaths(vec![]),
        )
        .unwrap();
    let delivery = presenter
        .reserve_delivery("client", "subscription", &target, None)
        .unwrap();
    delivery.start().unwrap();
    assert!(flag.0.swap(false, Ordering::SeqCst));
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, _)))
    ));
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Item(_, Event::Bookmark(_)))
    ));
    assert!(stream
        .as_mut()
        .poll_next(&mut Context::from_waker(&waker))
        .is_pending());
    flag.0.store(false, Ordering::SeqCst);

    assert_eq!(
        presenter
            .reserve_delivery("client", "subscription", &target, None)
            .err(),
        Some(SubscriptionError::AlreadyExists)
    );
    assert!(!flag.0.load(Ordering::SeqCst));
}

#[tokio::test]
async fn test_購読失敗_つなぎ直した購読へ保持済みのfailure事象を送り直す() {
    // Given
    let presenter = Arc::new(StateSubscriptionPresenter::new());
    let usecase = StateSubscriptionUsecase::new_with_output(
        presenter.clone(),
        crate::test_support::state_subscription::read_driver(),
    );
    presenter
        .publish_failure(
            &SubscriptionTarget::RepositoryPaths,
            StateReadError::from_error(SubscriptionError::SnapshotRequired),
        )
        .unwrap();
    let mut initial = Box::pin(
        crate::test_support::state_subscription::deps(usecase.clone(), presenter.clone())
            .stream("initial".into())
            .unwrap(),
    );
    initial.next().await;
    let delivery = presenter
        .reserve_delivery(
            "initial",
            "initial",
            &SubscriptionTarget::RepositoryPaths.to_string(),
            None,
        )
        .unwrap();
    delivery.start().unwrap();
    initial.next().await;
    drop(initial);
    // When
    let mut replay = Box::pin(
        crate::test_support::state_subscription::deps(usecase, presenter.clone())
            .stream("replay".into())
            .unwrap(),
    );
    replay.next().await;
    let delivery = presenter
        .reserve_delivery(
            "replay",
            "replay",
            &SubscriptionTarget::RepositoryPaths.to_string(),
            None,
        )
        .unwrap();
    delivery.start().unwrap();
    let event = replay.next().await.unwrap();
    let event = crate::adaptor::presenter::state_subscription_wire::event(event).unwrap();
    let wire: crate::adaptor::presenter::client::StateSubscriptionEvent =
        crate::adaptor::presenter::connect_wire::to_wire(&event).unwrap();
    // Then
    assert!(matches!(
        wire.event,
        Some(crate::adaptor::presenter::client::state_subscription_event::Event::Failure(_))
    ));
}
