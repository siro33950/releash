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
    let target = SubscriptionTarget::ReleashBase("/repo".into()).to_string();
    let payload = crate::adaptor::presenter::state_subscription_wire::payload(
        &StateValue::TerminalStartupCommand(String::new()),
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
            .reserve_delivery("absent", "subscription", &target)
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

    let target = SubscriptionTarget::TerminalStartupCommand.to_string();
    presenter
        .publish_initial(
            &SubscriptionTarget::TerminalStartupCommand,
            StateValue::TerminalStartupCommand(String::new()),
        )
        .unwrap();
    let delivery = presenter
        .reserve_delivery("client", "subscription", &target)
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
            .reserve_delivery("client", "subscription", &target)
            .err(),
        Some(SubscriptionError::AlreadyExists)
    );
    assert!(!flag.0.load(Ordering::SeqCst));
}

#[test]
fn test_ディレクトリ差分_変更と削除だけを送る() {
    use crate::adaptor::presenter::client::{
        state_payload::Value, DirectoryEntry, DirectorySnapshot, StatePayload,
    };
    // Given
    let old = DirectoryEntry {
        path: "old".into(),
        name: "old".into(),
        directory: false,
        ignored: false,
        status: "none".into(),
    };
    let same = DirectoryEntry {
        path: "same".into(),
        ..old.clone()
    };
    let new = DirectoryEntry {
        path: "new".into(),
        ..old.clone()
    };
    let value = |entries: Vec<DirectoryEntry>| {
        PublishedState::from(StatePayload {
            value: Some(Value::Directory(DirectorySnapshot {
                order: entries.iter().map(|entry| entry.path.clone()).collect(),
                entries,
                removed: Vec::new(),
                delta: false,
            })),
        })
    };
    let previous = value(vec![old, same.clone()]);
    // When
    let delta = directory_delta(Some(&previous), &value(vec![same, new.clone()])).unwrap();
    // Then
    let PublishedState::Value(delta) = delta else {
        panic!("expected value");
    };
    let Some(Value::Directory(delta)) = delta.value else {
        panic!("expected directory");
    };
    assert!(delta.delta);
    assert_eq!(delta.order, vec!["same", "new"]);
    assert_eq!(delta.entries, vec![new]);
    assert_eq!(delta.removed, vec!["old"]);
}
