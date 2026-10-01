use super::*;
use crate::adaptor::presenter::state_subscription::StateSubscriptionEvent;
use crate::test_support::state_subscription::WakeFlag;
use futures_util::{Stream, StreamExt};

#[test]
fn test_terminal登録_nulを含む識別子を失敗として返す() {
    // Given
    let presenter = TerminalSubscriptionPresenter::new(&StateSubscriptionPresenter::new());

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
    assert!(!presenter.test_runtime().inspect(|state| state.registered(
        &SubscriptionTarget::from_parts("terminal", &["/repo"])
            .unwrap()
            .to_string()
    )));
}

#[tokio::test]
async fn test_古いterminal寸法_初回の復元要求だけ待機者へ通知する() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Waker};

    // Given
    let generic = Arc::new(StateSubscriptionPresenter::new());
    let presenter = Arc::new(TerminalSubscriptionPresenter::new(&generic));
    presenter
        .initialize(&crate::test_support::state_subscription::registration(
            "session", "/repo", None, 1, 2,
        ))
        .unwrap();
    let usecase = crate::usecase::state_subscription::StateSubscriptionUsecase::new_with_output(
        generic.clone(),
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    let mut stream = Box::pin(
        crate::test_support::state_subscription::deps(usecase, generic)
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
    let old_resize = TerminalSurfaceOutputEvent::Resize {
        session_key: "session".into(),
        cols: 80,
        rows: 24,
        sequence: 1,
    };

    // When
    TerminalSurfaceStateSink::publish(
        presenter.as_ref(),
        &crate::test_support::state_subscription::registration("session", "/repo", None, 1, 2),
        old_resize.clone(),
    );
    // Then
    assert!(flag.0.swap(false, Ordering::SeqCst));
    assert!(stream
        .as_mut()
        .poll_next(&mut Context::from_waker(&waker))
        .is_pending());
    TerminalSurfaceStateSink::publish(
        presenter.as_ref(),
        &crate::test_support::state_subscription::registration("session", "/repo", None, 1, 2),
        old_resize,
    );
    assert!(!flag.0.load(Ordering::SeqCst));
    TerminalSurfaceStateSink::publish(
        presenter.as_ref(),
        &crate::test_support::state_subscription::registration("session", "/repo", None, 1, 2),
        TerminalSurfaceOutputEvent::Exit {
            session_key: "session".into(),
            runtime_generation: 1,
            exit_code: Some(0),
            sequence: 1,
        },
    );
    assert!(!flag.0.load(Ordering::SeqCst));
}

#[test]
fn test_terminal読取失敗_出力sequenceを進めない() {
    // Given
    let presenter = TerminalSubscriptionPresenter::new(&StateSubscriptionPresenter::new());
    let target = SubscriptionTarget::from_parts("terminal", &["/repo"]).unwrap();
    let version = Version {
        epoch: "terminal".into(),
        sequence: 4,
    };
    presenter
        .runtime
        .update(|state| state.register_delta(&target.to_string(), version.clone(), 1024))
        .unwrap();
    // When
    presenter
        .publish_failure(
            &target,
            StateReadError::from_error(SubscriptionError::SnapshotRequired),
        )
        .unwrap();
    // Then
    assert_eq!(
        presenter
            .runtime
            .inspect(|state| state.current_version(&target.to_string())),
        Some(version)
    );
}

#[test]
fn test_terminal読取失敗_失敗後の出力でsequenceを進める() {
    // Given
    let presenter = TerminalSubscriptionPresenter::new(&StateSubscriptionPresenter::new());
    let target = SubscriptionTarget::from_parts("terminal", &["/repo"]).unwrap();
    let version = Version {
        epoch: "terminal".into(),
        sequence: 4,
    };
    presenter
        .runtime
        .update(|state| state.register_delta(&target.to_string(), version.clone(), 1024))
        .unwrap();
    presenter
        .publish_failure(
            &target,
            StateReadError::from_error(SubscriptionError::SnapshotRequired),
        )
        .unwrap();
    // When
    let next = Version {
        epoch: "terminal".into(),
        sequence: 5,
    };
    let value = crate::adaptor::presenter::state_subscription_wire::payload(
        &StateValue::RepositoryPaths(vec![]),
    )
    .unwrap();
    presenter
        .runtime
        .update(|state| {
            state
                .publish_delta(
                    &target.to_string(),
                    next.clone(),
                    PublishedState::from(value),
                    1,
                    true,
                )
                .map(|_| true)
        })
        .unwrap();
    // Then
    assert_eq!(
        presenter
            .runtime
            .inspect(|state| state.current_version(&target.to_string())),
        Some(next)
    );
}
