use super::*;
use futures_util::FutureExt;

#[tokio::test]
async fn test_配信通知_状態を変えた操作だけ待機者を起こす() {
    // Given
    let runtime = StateSubscriptionRuntime::<u64>::new("boot".into());
    let unchanged = runtime.changed.notified();
    tokio::pin!(unchanged);
    unchanged.as_mut().enable();
    // When
    runtime.mutate(|_| ((), false));
    // Then
    assert!(unchanged.as_mut().now_or_never().is_none());

    // Given
    let changed = runtime.changed.notified();
    tokio::pin!(changed);
    changed.as_mut().enable();
    // When
    runtime
        .mutate(|state| (state.open("client".into()), true))
        .unwrap();
    // Then
    assert!(changed.as_mut().now_or_never().is_some());
}

#[tokio::test]
async fn test_配信通知_updateも状態が変わった場合だけ待機者を起こす() {
    // Given
    let runtime = StateSubscriptionRuntime::<u64>::new("boot".into());
    let unchanged = runtime.changed.notified();
    tokio::pin!(unchanged);
    unchanged.as_mut().enable();

    // When
    runtime.update(|_| Ok(false)).unwrap();

    // Then
    assert!(unchanged.as_mut().now_or_never().is_none());

    // Given
    let changed = runtime.changed.notified();
    tokio::pin!(changed);
    changed.as_mut().enable();

    // When
    runtime
        .update(|state| {
            state
                .register("target".into(), 0, Delivery::Full)
                .map(|_| true)
        })
        .unwrap();

    // Then
    assert!(changed.as_mut().now_or_never().is_some());

    let repeated = runtime.changed.notified();
    tokio::pin!(repeated);
    repeated.as_mut().enable();
    runtime
        .update(|state| state.publish("target", 0, None))
        .unwrap();
    assert!(repeated.as_mut().now_or_never().is_none());
}
fn registry() -> Subscriptions<u64> {
    let mut state = Subscriptions::new("boot".into());
    state
        .register("workspaces".into(), 0, Delivery::Full)
        .unwrap();
    state
        .register("providers".into(), 0, Delivery::Delta)
        .unwrap();
    state.open("client".into()).unwrap();
    state
}

#[test]
fn test_再開_変更直後のbookmarkと送り待ち量が対応する() {
    // Given
    let mut state = registry();
    let target = "terminal:3:pty";
    let before = Version {
        epoch: "runtime".into(),
        sequence: 0,
    };
    state.register_delta(target, before.clone(), 100).unwrap();
    state.set_delta_snapshot(target, before.clone(), 0).unwrap();
    state
        .publish_delta(
            target,
            Version {
                sequence: 1,
                ..before.clone()
            },
            1,
            8,
            true,
        )
        .unwrap();
    // When
    state.start("client", target, Some(&before)).unwrap();
    // Then
    assert_eq!(state.pending_amount("client", target), 8);
    assert_eq!(state.clients["client"].subscriptions[target].sizes.len(), 2);
    assert!(matches!(
        state.next("client"),
        Some((
            _,
            Event::Change(Version { sequence: 1, .. }, Delivery::Delta, _)
        ))
    ));
    assert_eq!(state.pending_amount("client", target), 0);
    assert!(matches!(
        state.next("client"),
        Some((_, Event::Bookmark(Version { sequence: 1, .. })))
    ));
    assert!(state.next("client").is_none());
}

#[tokio::test(start_paused = true)]
async fn test_定期印_別対象の更新が続いても無通信の購読へ送る() {
    // Given
    let runtime = StateSubscriptionRuntime::new("boot".into());
    runtime
        .state
        .lock()
        .register("idle".into(), 0_u64, Delivery::Full)
        .unwrap();
    runtime
        .state
        .lock()
        .register("busy".into(), 0_u64, Delivery::Full)
        .unwrap();
    runtime.state.lock().open("client".into()).unwrap();
    runtime.state.lock().start("client", "idle", None).unwrap();
    let mut stream = Box::pin(runtime.stream("client".into(), (), |_, _| {}));
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Ready)
    ));
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, _)))
    ));
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Item(_, Event::Bookmark(_)))
    ));
    let waiting = tokio::spawn(async move { stream.next().await });
    tokio::task::yield_now().await;
    // When
    for value in 1..=9 {
        tokio::time::advance(std::time::Duration::from_secs(1)).await;
        runtime
            .update(|state| state.publish("busy", value, None))
            .unwrap();
        tokio::task::yield_now().await;
    }
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    // Then
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_millis(100), waiting)
            .await
            .unwrap()
            .unwrap(),
        Some(StateSubscriptionEvent::Item(target, Event::Bookmark(_))) if target == "idle"
    ));
}

#[tokio::test(start_paused = true)]
async fn test_定期印_購読の無いstreamにも間隔ごとに送る() {
    // Given
    let runtime = StateSubscriptionRuntime::<u64>::new("boot".into());
    runtime.state.lock().open("client".into()).unwrap();
    let mut stream = Box::pin(runtime.stream("client".into(), (), |_, _| {}));
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Ready)
    ));
    let waiting = tokio::spawn(async move { stream.next().await });
    tokio::task::yield_now().await;
    // When
    tokio::time::advance(std::time::Duration::from_secs(10)).await;
    // Then
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_millis(100), waiting)
            .await
            .unwrap()
            .unwrap(),
        Some(StateSubscriptionEvent::Bookmark)
    ));
}

#[test]
fn test_定期印_購読の有無を返す() {
    // Given
    let mut state = registry();
    // When / Then
    assert!(!state.bookmark("client"));
    assert!(!state.bookmark("unknown"));
    state.start("client", "workspaces", None).unwrap();
    assert!(state.bookmark("client"));
}

#[test]
fn test_購読_初期状態と区切りの後に変更が届く() {
    // Given
    let mut state = registry();
    // When
    state.start("client", "workspaces", None).unwrap();
    state.publish("workspaces", 1, None).unwrap();
    // Then
    assert!(
        matches!(state.next("client"), Some((_, Event::Snapshot(Version {sequence: 0, ..}, value))) if *value == 0)
    );
    assert!(matches!(
        state.next("client"),
        Some((_, Event::Bookmark(Version { sequence: 0, .. })))
    ));
    assert!(
        matches!(state.next("client"), Some((_, Event::Change(Version {sequence: 1, ..}, Delivery::Full, value))) if *value == 1)
    );
    assert!(state.next("client").is_none());
}

#[test]
fn test_購読_重複と停止と存在しない対象() {
    // Given
    let mut state = registry();
    // When
    state.start("client", "workspaces", None).unwrap();
    state.start("client", "workspaces", None).unwrap();
    // Then
    assert_eq!(state.clients["client"].subscriptions.len(), 1);
    assert_eq!(
        state.start("client", "missing", None),
        Err(SubscriptionError::UnknownTarget)
    );
    state.stop("client", "workspaces").unwrap();
    assert!(state.next("client").is_none());
    state.publish("workspaces", 1, None).unwrap();
    assert!(state.next("client").is_none());
    state.start("client", "providers", None).unwrap();
    state.close("client");
    assert_eq!(
        state.start("client", "workspaces", None),
        Err(SubscriptionError::StreamEnded)
    );
}

#[test]
fn test_再開_履歴内と古い版と別起動と未来の版() {
    // Given
    let mut state = registry();
    state.publish("workspaces", 1, None).unwrap();
    let version = Version {
        epoch: "boot".into(),
        sequence: 0,
    };
    // When
    state.start("client", "workspaces", Some(&version)).unwrap();
    // Then
    assert!(matches!(state.next("client"), Some((_, Event::Change(_, _, value))) if *value == 1));
    for version in [
        Version {
            epoch: "old".into(),
            sequence: 0,
        },
        Version {
            epoch: "boot".into(),
            sequence: 100,
        },
    ] {
        state.stop("client", "workspaces").unwrap();
        state.start("client", "workspaces", Some(&version)).unwrap();
        assert!(
            matches!(state.next("client"), Some((_, Event::Snapshot(_, value))) if *value == 1)
        );
        assert!(matches!(
            state.next("client"),
            Some((_, Event::Bookmark(_)))
        ));
    }
    for n in 2..=70 {
        state.publish("workspaces", n, None).unwrap();
    }
    state.stop("client", "workspaces").unwrap();
    state
        .start(
            "client",
            "workspaces",
            Some(&Version {
                epoch: "boot".into(),
                sequence: 0,
            }),
        )
        .unwrap();
    assert!(matches!(
        state.next("client"),
        Some((_, Event::Snapshot(Version { sequence: 70, .. }, _)))
    ));
}

#[test]
fn test_送り待ち_溢れた購読のみ再開し他の対象は続く() {
    // Given
    let mut state = registry();
    state.start("client", "workspaces", None).unwrap();
    state.start("client", "providers", None).unwrap();
    for _ in 0..4 {
        state.next("client").unwrap();
    }
    // When
    for n in 1..=70 {
        state.publish("workspaces", n, None).unwrap();
    }
    state.publish("providers", 10, Some(2)).unwrap();
    // Then
    assert!(
        matches!(state.next("client"), Some((id, Event::Snapshot(Version {sequence:70,..}, value))) if id == "workspaces" && *value == 70)
    );
    assert!(
        matches!(state.next("client"), Some((id, Event::Change(_, Delivery::Delta, value))) if id == "providers" && *value == 2)
    );
    assert!(matches!(
        state.next("client"),
        Some((_, Event::Bookmark(Version { sequence: 70, .. })))
    ));
    state.bookmark("client");
    assert!(matches!(
        state.next("client"),
        Some((_, Event::Bookmark(_)))
    ));
}

#[test]
fn test_購読数_上限がなく版は購読し直しても戻らない() {
    // Given
    let mut state = registry();
    // When
    for n in 0..100 {
        let id = format!("branches-{n}");
        state.register(id.clone(), n, Delivery::Full).unwrap();
        state.start("client", &id, None).unwrap();
    }
    state.publish("workspaces", 5, None).unwrap();
    state.close("client");
    state.open("client".into()).unwrap();
    state.start("client", "workspaces", None).unwrap();
    // Then
    assert!(matches!(
        state.next("client"),
        Some((_, Event::Snapshot(Version { sequence: 1, .. }, _)))
    ));
    assert_eq!(
        state.open("client".into()),
        Err(SubscriptionError::AlreadyExists)
    );
    assert_eq!(state.open("".into()), Err(SubscriptionError::InvalidId));
}

#[test]
fn test_再開_送り待ちが溢れても保持した版以降だけを再生する() {
    // Given
    let mut state = registry();
    state.start("client", "workspaces", None).unwrap();
    state.next("client").unwrap();
    assert!(matches!(
        state.next("client"),
        Some((_, Event::Bookmark(_)))
    ));
    // When
    for n in 1..=64 {
        state.publish("workspaces", n, None).unwrap();
    }
    // Then
    for n in 1..=64 {
        assert!(
            matches!(state.next("client"), Some((_, Event::Change(Version {sequence,..}, _, _))) if sequence == n)
        );
    }
    assert!(state.next("client").is_none());
    state.bookmark("client");
    assert!(matches!(
        state.next("client"),
        Some((_, Event::Bookmark(Version { sequence: 64, .. })))
    ));
    state.stop("client", "workspaces").unwrap();
    state
        .start(
            "client",
            "workspaces",
            Some(&Version {
                epoch: "boot".into(),
                sequence: 64,
            }),
        )
        .unwrap();
    assert!(matches!(
        state.next("client"),
        Some((_, Event::Bookmark(_)))
    ));
    assert!(state.next("client").is_none());
    state.bookmark("client");
    assert!(matches!(
        state.next("client"),
        Some((_, Event::Bookmark(_)))
    ));
    state.publish("workspaces", 64, None).unwrap();
    assert!(state.next("client").is_none());
}

#[test]
fn test_共有購読_最後のclient終了時に対象がinactiveになる() {
    // Given
    let mut state = registry();
    state.open("other".into()).unwrap();
    // When
    state.start("client", "workspaces", None).unwrap();
    state.start("other", "workspaces", None).unwrap();
    state.stop("client", "workspaces").unwrap();
    // Then
    assert!(state.active_targets().contains("workspaces"));
    state.close("other");
    assert!(!state.active_targets().contains("workspaces"));
}

#[test]
fn test_購読開始_切断で解放したsnapshotは再取得前に再開しない() {
    let mut state = registry();
    state.start("client", "workspaces", None).unwrap();
    state.close("client");
    state.release_inactive_snapshots();
    state.open("next".into()).unwrap();
    assert_eq!(
        state.start("next", "workspaces", None),
        Err(SubscriptionError::UnknownTarget)
    );
    state.publish("workspaces", 1, None).unwrap();
    state.start("next", "workspaces", None).unwrap();
    assert!(matches!(state.next("next"), Some((_, Event::Snapshot(_, value))) if *value == 1));
}

#[test]
fn test_購読開始失敗_切断済みclientの対象を登録せず既存対象を保持する() {
    // Given
    let mut state = registry();
    state
        .register("repository-paths".into(), 0, Delivery::Full)
        .unwrap();
    let target = "branches";
    // When
    state.register(target.into(), 1, Delivery::Full).unwrap();
    assert_eq!(
        state.start("closed", target, None),
        Err(SubscriptionError::StreamEnded)
    );
    assert_eq!(
        state.ensure_active(target),
        Err(SubscriptionError::StreamEnded)
    );
    // Then
    assert!(!state.registered(target));
    assert!(state.registered("repository-paths"));
    state.register(target.into(), 1, Delivery::Full).unwrap();
    state.start("client", target, None).unwrap();
    assert!(state.registered(target));
}

#[test]
fn test_対象ごとの配信_変更値を購読clientへ届ける() {
    // Given
    let mut state = Subscriptions::new("boot".into());
    state.open("client".into()).unwrap();
    let raw = "branch-list";
    state
        .register(raw.into(), vec!["main".to_string()], Delivery::Full)
        .unwrap();
    state.start("client", raw, None).unwrap();
    assert!(
        matches!(state.next("client"), Some((_, Event::Snapshot(_, value))) if *value == ["main"])
    );
    state.next("client");
    // When
    state
        .publish(raw, vec!["main".to_string(), "develop".to_string()], None)
        .unwrap();
    // Then
    assert!(
        matches!(state.next("client"), Some((id, Event::Change(_, Delivery::Full, value))) if id == raw && *value == ["main", "develop"])
    );
    state.stop("client", raw).unwrap();
    assert!(!state.active_targets().contains(raw));
}

#[test]
fn test_複数対象の配信_指定した対象だけに変更値を届ける() {
    for raw in [
        "branch-base",
        "branch-status",
        "current-branch",
        "worktrees",
        "repository-root",
    ] {
        // Given
        let mut state = Subscriptions::new("boot".into());
        state.open("client".into()).unwrap();
        state
            .register(raw.into(), "before", Delivery::Full)
            .unwrap();
        state.start("client", raw, None).unwrap();
        assert!(
            matches!(state.next("client"), Some((id, Event::Snapshot(_, value))) if id == raw && *value == "before")
        );
        assert!(matches!(
            state.next("client"),
            Some((_, Event::Bookmark(_)))
        ));
        // When
        state.publish(raw, "after", None).unwrap();
        // Then
        assert!(
            matches!(state.next("client"), Some((id, Event::Change(version, Delivery::Full, value))) if id == raw && version.sequence == 1 && *value == "after")
        );
    }
}

#[test]
fn test_差分対象_最後のclient切断後もruntimeの登録を保持する() {
    let mut state = registry();
    let target = "terminal:3:pty";
    let version = Version {
        epoch: "runtime-1".into(),
        sequence: 0,
    };
    state.register_delta(target, version.clone(), 100).unwrap();
    state.set_delta_snapshot(target, version, 0).unwrap();
    state.start("client", target, None).unwrap();

    state.close("client");

    assert_eq!(
        state.ensure_active(target),
        Err(SubscriptionError::StreamEnded)
    );
    assert!(state.registered(target));
}

#[test]
fn test_購読開始確認_切断後は対象の鍵を解放し他の購読を保持する() {
    // Given
    let mut state = registry();
    state
        .register("repository-paths".into(), 0, Delivery::Full)
        .unwrap();
    state.open("other".into()).unwrap();
    state.start("other", "providers", None).unwrap();
    state.start("client", "workspaces", None).unwrap();
    // When
    state.close("client");
    state.release_inactive_snapshots();
    // Then
    assert_eq!(
        state.ensure_active("workspaces"),
        Err(SubscriptionError::StreamEnded)
    );
    assert!(!state.registered("workspaces"));
    assert_eq!(state.ensure_active("providers"), Ok(()));
    assert_eq!(
        state.ensure_active("repository-paths"),
        Err(SubscriptionError::StreamEnded)
    );
    assert!(!state.registered("repository-paths"));
}

#[test]
fn test_provider一覧購読_指定対象に更新一覧を配信する() {
    // Given
    let mut state = Subscriptions::new("boot".into());
    state.open("client".into()).unwrap();
    state
        .register("providers".into(), vec!["codex"], Delivery::Full)
        .unwrap();
    state.start("client", "providers", None).unwrap();
    state
        .register("workspaces".into(), vec![], Delivery::Full)
        .unwrap();
    state.start("client", "workspaces", None).unwrap();
    for _ in 0..4 {
        state.next("client");
    }
    // When
    state
        .publish("providers", vec!["codex", "claude"], None)
        .unwrap();
    // Then
    assert!(
        matches!(state.next("client"), Some((id, Event::Change(version, Delivery::Full, value))) if id == "providers" && version.sequence == 1 && *value == ["codex", "claude"])
    );
    assert!(state.next("client").is_none());
}

#[test]
fn test_開始失敗で解放した対象_再登録時は以前の版を再利用せずsnapshotを届ける() {
    // Given
    let mut state = registry();
    state.start("client", "workspaces", None).unwrap();
    let (_, initial) = state.next("client").unwrap();
    let version = initial.version().clone();
    state.close("client");
    state.release_inactive_snapshots();
    // When
    state.publish("workspaces", 1, None).unwrap();
    assert_eq!(
        state.start("closed", "workspaces", None),
        Err(SubscriptionError::StreamEnded)
    );
    assert_eq!(
        state.ensure_active("workspaces"),
        Err(SubscriptionError::StreamEnded)
    );
    assert!(!state.registered("workspaces"));
    state.open("next".into()).unwrap();
    state
        .register("workspaces".into(), 2, Delivery::Full)
        .unwrap();
    state.start("next", "workspaces", Some(&version)).unwrap();
    // Then
    assert!(
        matches!(state.next("next"), Some((_, Event::Snapshot(next, value))) if next.epoch != version.epoch && *value == 2)
    );
}

#[test]
fn test_対象の解放_世代番号が尽きたら版を再利用せずエラーにする() {
    // Given
    let mut state = registry();
    state.target_generation = u64::MAX;
    // When / Then
    assert_eq!(
        state.ensure_active("workspaces"),
        Err(SubscriptionError::VersionExhausted)
    );
    assert!(state.registered("workspaces"));
}

#[test]
fn test_差分購読_対象の版で再開し件数では溢れない() {
    // Given
    let mut state = registry();
    let target = "terminal:3:pty";
    let version = |sequence| Version {
        epoch: "runtime-1".into(),
        sequence,
    };
    state.register_delta(target, version(20), 100_000).unwrap();
    assert!(state.needs_snapshot(target, None).unwrap());
    state.set_delta_snapshot(target, version(20), 0).unwrap();
    state.start("client", target, None).unwrap();
    // When
    for sequence in 21..=120 {
        state
            .publish_delta(target, version(sequence), sequence, 1, true)
            .unwrap();
    }
    // Then
    assert!(matches!(state.next("client"), Some((_, Event::Snapshot(v, _))) if v.sequence == 20));
    assert!(matches!(
        state.next("client"),
        Some((_, Event::Bookmark(_)))
    ));
    for sequence in 21..=120 {
        assert!(
            matches!(state.next("client"), Some((_, Event::Change(v, Delivery::Delta, _))) if v.sequence == sequence)
        );
    }
    state.stop("client", target).unwrap();
    assert!(!state.needs_snapshot(target, Some(&version(119))).unwrap());
    state.start("client", target, Some(&version(119))).unwrap();
    assert!(matches!(state.next("client"), Some((_, Event::Change(v, _, _))) if v.sequence == 120));
}

#[test]
fn test_terminal開始_登録後の出力でsnapshotが消えても現在状態を待つ() {
    // Given
    let mut state = Subscriptions::new("boot".into());
    let target = "terminal:3:pty";
    let initial = Version {
        epoch: "runtime".into(),
        sequence: 0,
    };
    let current = Version {
        sequence: 1,
        ..initial.clone()
    };
    state.open("client".into()).unwrap();
    state.register_delta(target, initial.clone(), 100).unwrap();
    state.set_delta_snapshot(target, initial, 0).unwrap();
    state
        .publish_delta(target, current.clone(), 1, 1, true)
        .unwrap();

    // When
    state
        .start(
            "client",
            target,
            Some(&Version {
                epoch: "old".into(),
                sequence: 0,
            }),
        )
        .unwrap();

    // Then
    assert_eq!(state.snapshot_requests("client"), vec![target]);
    assert!(state.next("client").is_none());
    state
        .set_delta_snapshot(target, current.clone(), 2)
        .unwrap();
    assert!(matches!(
        state.next("client"),
        Some((_, Event::Snapshot(version, value))) if version == current && *value == 2
    ));
}

#[test]
fn test_terminal開始_snapshot未確定の差分対象を登録してから作り直す() {
    // Given
    let mut state = registry();
    let target = "terminal:3:pty";
    let version = Version {
        epoch: "runtime".into(),
        sequence: 0,
    };
    state.register_delta(target, version.clone(), 100).unwrap();
    // When
    state.start("client", target, None).unwrap();
    // Then
    assert_eq!(state.snapshot_requests("client"), vec![target]);
    assert!(state.next("client").is_none());
    state
        .set_delta_snapshot(target, version.clone(), 7)
        .unwrap();
    assert!(
        matches!(state.next("client"), Some((_, Event::Snapshot(v, value))) if v == version && *value == 7)
    );
}

#[tokio::test(start_paused = true)]
async fn test_定期印_変更の配信で周期の起点をずらさない() {
    // Given
    let runtime = StateSubscriptionRuntime::new("boot".into());
    runtime
        .state
        .lock()
        .register("target".into(), 0_u64, Delivery::Full)
        .unwrap();
    runtime.state.lock().open("client".into()).unwrap();
    runtime
        .state
        .lock()
        .start("client", "target", None)
        .unwrap();
    let mut stream = Box::pin(runtime.stream("client".into(), (), |_, _| {}));
    stream.next().await;
    stream.next().await;
    stream.next().await;
    // When
    tokio::time::advance(std::time::Duration::from_secs(9)).await;
    runtime
        .update(|state| state.publish("target", 1, None))
        .unwrap();
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Item(_, Event::Change(_, _, _)))
    ));
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    // Then
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Item(_, Event::Bookmark(_)))
    ));
}

#[test]
fn test_差分購読_零単位の要素も一単位として数える() {
    // Given
    let mut state = registry();
    let target = "terminal:3:pty";
    let version = Version {
        epoch: "runtime-1".into(),
        sequence: 0,
    };
    state.register_delta(target, version.clone(), 100).unwrap();
    state
        .set_delta_snapshot(target, version.clone(), 0)
        .unwrap();
    state.start("client", target, None).unwrap();
    state.next("client");
    state.next("client");

    // When
    state
        .publish_delta(target, version.clone(), 1, 0, false)
        .unwrap();
    state
        .publish_delta(
            target,
            Version {
                sequence: 1,
                ..version
            },
            2,
            4,
            true,
        )
        .unwrap();

    // Then
    assert_eq!(state.pending_amount("client", target), 5);
    state.next("client");
    assert_eq!(state.pending_amount("client", target), 4);
    state.next("client");
    assert_eq!(state.pending_amount("client", target), 0);
}

#[test]
fn test_差分購読_量の超過と作り直しは現在状態を要求する() {
    // Given
    let mut state = registry();
    let target = "terminal:3:pty";
    let version = Version {
        epoch: "runtime-1".into(),
        sequence: 10,
    };
    state.register_delta(target, version.clone(), 100).unwrap();
    state
        .set_delta_snapshot(target, version.clone(), 0)
        .unwrap();
    state.start("client", target, None).unwrap();
    state.next("client");
    state.next("client");
    // When
    for sequence in 11..=80 {
        state
            .publish_delta(
                target,
                Version {
                    sequence,
                    ..version.clone()
                },
                sequence,
                2,
                true,
            )
            .unwrap();
    }
    // Then
    assert_eq!(state.snapshot_requests("client"), vec![target]);
    assert!(state.next("client").is_none());
    state
        .register_delta(
            target,
            Version {
                epoch: "runtime-2".into(),
                sequence: 10,
            },
            100,
        )
        .unwrap();
    assert!(state.needs_snapshot(target, Some(&version)).unwrap());
}

#[test]
fn test_差分再開_同じ出力番号の変更を再送し出力は重複させない() {
    // Given
    let mut state = registry();
    let target = "terminal:3:pty";
    let version = |sequence| Version {
        epoch: "runtime".into(),
        sequence,
    };
    state.register_delta(target, version(0), 100_000).unwrap();
    state
        .publish_delta(target, version(1), 10, 10, true)
        .unwrap();
    state
        .publish_delta(target, version(1), 20, 0, false)
        .unwrap();
    state
        .publish_delta(target, version(1), 30, 0, false)
        .unwrap();
    state
        .publish_delta(target, version(2), 40, 10, true)
        .unwrap();
    // When
    state.start("client", target, Some(&version(1))).unwrap();
    // Then
    for (sequence, expected) in [(1, 20), (1, 30), (2, 40)] {
        assert!(
            matches!(state.next("client"), Some((_, Event::Change(v, Delivery::Delta, value))) if v.sequence == sequence && *value == expected)
        );
    }
    assert!(matches!(state.next("client"), Some((_, Event::Bookmark(v))) if v.sequence == 2));
    assert!(state.next("client").is_none());
    state.bookmark("client");
    assert!(matches!(state.next("client"), Some((_, Event::Bookmark(v))) if v.sequence == 2));
    assert!(state.next("client").is_none());
}

#[test]
fn test_差分再開_同番号の変更の履歴が欠けたら現在状態を要求する() {
    // Given
    let mut state = registry();
    let target = "terminal:3:pty";
    let version = Version {
        epoch: "runtime".into(),
        sequence: 0,
    };
    state
        .register_delta(target, version.clone(), 100_000)
        .unwrap();
    // When
    for value in 0..=RETAINED_CHANGES {
        state
            .publish_delta(target, version.clone(), value as u64, 0, false)
            .unwrap();
    }
    // Then
    assert!(state.needs_snapshot(target, Some(&version)).unwrap());
}

#[test]
fn test_差分購読_出力欠落後の同番号変更だけで再開せずsnapshotを要求する() {
    // Given
    let mut state = registry();
    let target = "terminal:3:pty";
    let version = |sequence| Version {
        epoch: "runtime".into(),
        sequence,
    };
    state.register_delta(target, version(0), 100_000).unwrap();
    state.set_delta_snapshot(target, version(0), 0).unwrap();
    state.start("client", target, None).unwrap();
    state.next("client");
    state.next("client");
    // When
    state
        .publish_delta(target, version(1), 20, 0, false)
        .unwrap();
    // Then
    assert_eq!(state.snapshot_requests("client"), vec![target]);
    assert!(state.next("client").is_none());
    assert!(state.needs_snapshot(target, Some(&version(0))).unwrap());
}

#[test]
fn test_差分復元要求_同じ対象の全購読を現在状態から再開する() {
    // Given
    let mut state = registry();
    let target = "terminal:3:pty";
    let version = Version {
        epoch: "runtime".into(),
        sequence: 1,
    };
    state
        .register_delta(target, version.clone(), 100_000)
        .unwrap();
    state
        .set_delta_snapshot(target, version.clone(), 10)
        .unwrap();
    state.open("second".into()).unwrap();
    for client in ["client", "second"] {
        state.start(client, target, None).unwrap();
        state.next(client);
        state.next(client);
    }
    // When
    assert!(state.require_delta_snapshot(target).unwrap());
    // Then
    for client in ["client", "second"] {
        assert_eq!(state.snapshot_requests(client), vec![target]);
        assert!(state.next(client).is_none());
    }
    let mut clients = state.snapshot_request_clients(target);
    clients.sort();
    assert_eq!(clients, ["client", "second"]);
    assert!(!state.require_delta_snapshot(target).unwrap());
    assert_eq!(state.snapshot_request_clients(target).len(), 2);
    state
        .set_delta_snapshot(target, version.clone(), 20)
        .unwrap();
    for client in ["client", "second"] {
        assert!(
            matches!(state.next(client), Some((_, Event::Snapshot(v, value))) if v == version && *value == 20)
        );
    }
}

#[test]
fn test_購読対象削除_差分履歴を解放し購読は明示停止まで保つ() {
    // Given
    let target = "terminal:5:/repo";
    let mut state = Subscriptions::new("boot".into());
    let version = Version {
        epoch: "runtime-1".into(),
        sequence: 0,
    };
    state
        .register_delta(target, version.clone(), 100_000)
        .unwrap();
    state
        .set_delta_snapshot(target, version.clone(), "snapshot")
        .unwrap();
    state.open("client".into()).unwrap();
    state.start("client", target, None).unwrap();
    state
        .publish_delta(
            target,
            Version {
                sequence: 1,
                ..version.clone()
            },
            "output",
            6,
            true,
        )
        .unwrap();
    // When
    state.unregister(target).unwrap();
    // Then
    assert!(state.targets.is_empty());
    state.bookmark("client");
    assert!(state.snapshot_requests("client").is_empty());
    assert!(matches!(
        state.next("client"),
        Some((_, Event::Snapshot(_, _)))
    ));
    assert!(matches!(
        state.next("client"),
        Some((_, Event::Bookmark(_)))
    ));
    assert!(
        matches!(state.next("client"), Some((_, Event::Change(_, Delivery::Delta, value))) if *value == "output")
    );
    assert!(state.is_subscribed("client", target));
    assert!(state.next("client").is_none());
    state
        .register_delta(
            target,
            Version {
                epoch: "runtime-2".into(),
                sequence: 0,
            },
            100_000,
        )
        .unwrap();
    assert!(state.needs_snapshot(target, Some(&version)).unwrap());
}

#[test]
fn test_購読対象再作成_送り待ちが無い購読と溢れた購読も再開する() {
    for overflow in [false, true] {
        // Given
        let mut state = registry();
        let target = "terminal:3:pty";
        let version = Version {
            epoch: "runtime".into(),
            sequence: 0,
        };
        state.register_delta(target, version.clone(), 1).unwrap();
        state
            .set_delta_snapshot(target, version.clone(), 0)
            .unwrap();
        state.start("client", target, None).unwrap();
        state.next("client");
        state.next("client");
        if overflow {
            state
                .publish_delta(
                    target,
                    Version {
                        sequence: 1,
                        ..version
                    },
                    1,
                    2,
                    true,
                )
                .unwrap();
        }
        // When
        state.unregister(target).unwrap();
        // Then
        assert!(!state.registered(target));
        assert!(state.is_subscribed("client", target));
        assert!(state.next("client").is_none());
        let new = Version {
            epoch: "new".into(),
            sequence: 0,
        };
        state.register_delta(target, new.clone(), 100).unwrap();
        assert_eq!(state.snapshot_requests("client"), vec![target]);
        state.set_delta_snapshot(target, new.clone(), 2).unwrap();
        assert_eq!(
            state.next("client"),
            Some((target.into(), Event::Snapshot(new.clone(), Arc::new(2))))
        );
        assert_eq!(
            state.next("client"),
            Some((target.into(), Event::Bookmark(new.clone())))
        );
        let output = Version { sequence: 1, ..new };
        state
            .publish_delta(target, output.clone(), 3, 1, true)
            .unwrap();
        assert_eq!(
            state.next("client"),
            Some((
                target.into(),
                Event::Change(output, Delivery::Delta, Arc::new(3))
            ))
        );
        state.stop("client", target).unwrap();
        assert!(!state.is_subscribed("client", target));
        assert!(state.clients["client"].order.is_empty());
    }
}

#[test]
fn test_差分対象再作成_旧世代の送り待ちを新世代snapshotより先に届ける() {
    // Given
    let mut state = registry();
    let target = "terminal:3:pty";
    let old = Version {
        epoch: "old".into(),
        sequence: 0,
    };
    let new = Version {
        epoch: "new".into(),
        sequence: 1,
    };
    state.register_delta(target, old.clone(), 100).unwrap();
    state.set_delta_snapshot(target, old.clone(), 0).unwrap();
    state.start("client", target, None).unwrap();
    state.next("client");
    state.next("client");
    state
        .publish_delta(target, old.clone(), 7, 0, false)
        .unwrap();
    // When
    state.unregister(target).unwrap();
    state
        .register_delta(
            target,
            Version {
                sequence: 0,
                ..new.clone()
            },
            100,
        )
        .unwrap();
    state
        .publish_delta(target, new.clone(), 8, 101, true)
        .unwrap();
    state.set_delta_snapshot(target, new.clone(), 9).unwrap();
    // Then
    assert_eq!(
        state.next("client"),
        Some((
            target.into(),
            Event::Change(old, Delivery::Delta, Arc::new(7))
        ))
    );
    assert_eq!(
        state.next("client"),
        Some((target.into(), Event::Snapshot(new.clone(), Arc::new(9))))
    );
    assert_eq!(
        state.next("client"),
        Some((target.into(), Event::Bookmark(new)))
    );
    assert!(state.is_subscribed("client", target));
    state.stop("client", target).unwrap();
    assert!(!state.is_subscribed("client", target));
}
