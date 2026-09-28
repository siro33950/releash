use super::*;
use crate::test_support::state_subscription::WakeFlag;

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

#[test]
fn test_terminal登録_nulを含む識別子を失敗として返す() {
    // Given
    let presenter = StateSubscriptionPresenter::new(vec![]);

    // When
    let path = presenter.initialize(&crate::test_support::state_subscription::registration(
        "session", "/re\0po", None, 1, 0,
    ));
    let session = presenter.initialize(&crate::test_support::state_subscription::registration(
        "session",
        "/repo",
        Some("ses\0sion"),
        1,
        0,
    ));

    // Then
    assert!(path.is_err());
    assert!(session.is_err());
    assert_eq!(presenter.test_runtime().test_terminal_route_count(), 0);
}

#[tokio::test]
async fn test_古いterminal寸法_初回の復元要求だけ待機者へ通知する() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Waker};

    // Given
    let presenter = Arc::new(StateSubscriptionPresenter::new(vec![]));
    presenter
        .initialize(&crate::test_support::state_subscription::registration(
            "session", "/repo", None, 1, 2,
        ))
        .unwrap();
    let usecase = StateSubscriptionUsecase::new_with_output(
        presenter.clone(),
        presenter.change_sender(),
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    let mut stream = Box::pin(presenter.stream(usecase, "waiting".into()).unwrap());
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
    let old_resize = TerminalSurfaceOutputEvent::Resize {
        session_key: "session".into(),
        cols: 80,
        rows: 24,
        sequence: 1,
    };

    // When
    TerminalSurfaceStateSink::publish(presenter.as_ref(), old_resize.clone());
    // Then
    assert!(flag.0.swap(false, Ordering::SeqCst));
    assert!(stream
        .as_mut()
        .poll_next(&mut Context::from_waker(&waker))
        .is_pending());
    TerminalSurfaceStateSink::publish(presenter.as_ref(), old_resize);
    assert!(!flag.0.load(Ordering::SeqCst));
    TerminalSurfaceStateSink::publish(
        presenter.as_ref(),
        TerminalSurfaceOutputEvent::Exit {
            session_key: "session".into(),
            runtime_generation: 1,
            exit_code: Some(0),
            sequence: 1,
        },
    );
    assert!(!flag.0.load(Ordering::SeqCst));
}

#[tokio::test]
async fn test_購読開始失敗_対象削除を待機中streamへ通知する() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Waker};

    // Given
    let presenter = Arc::new(StateSubscriptionPresenter::new(vec![]));
    let target = SubscriptionTarget::BranchBase("/repo".into(), "main".into()).to_string();
    let payload = crate::adaptor::presenter::state_subscription_wire::payload(
        &StateValue::RepositoryPaths(vec![]),
    )
    .unwrap();
    presenter
        .runtime
        .update(|state| {
            state
                .register(target.clone(), payload, Delivery::Full)
                .map(|_| true)
        })
        .unwrap();
    let usecase = StateSubscriptionUsecase::new_with_output(
        presenter.clone(),
        presenter.change_sender(),
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    let mut stream = Box::pin(presenter.stream(usecase, "waiting".into()).unwrap());
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
        presenter.start("absent", &target, None),
        Err(SubscriptionError::StreamEnded)
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

    let presenter = Arc::new(StateSubscriptionPresenter::new(vec![]));
    let usecase = StateSubscriptionUsecase::new_with_output(
        presenter.clone(),
        presenter.change_sender(),
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    let mut stream = Box::pin(presenter.stream(usecase, "client".into()).unwrap());
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
    presenter.start("client", &target, None).unwrap();
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

    presenter.start("client", &target, None).unwrap();
    assert!(!flag.0.load(Ordering::SeqCst));
}
