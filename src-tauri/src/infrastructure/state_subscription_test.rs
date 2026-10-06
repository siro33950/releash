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
    state
        .reserve("client", target, target)
        .and_then(|()| {
            state.activate(target, Some(&before)).inspect_err(|_| {
                let _ = state.stop("client", target);
            })
        })
        .unwrap();
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
    {
        let mut state = runtime.state.lock();
        state
            .reserve("client", "idle", "idle")
            .and_then(|()| state.activate("idle", None))
    }
    .unwrap();
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
async fn test_定期印_snapshot待ちの購読だけでもstreamに送る() {
    // Given
    let runtime = StateSubscriptionRuntime::<u64>::new("boot".into());
    runtime.state.lock().open("client".into()).unwrap();
    runtime
        .state
        .lock()
        .register_delta(
            "waiting",
            Version {
                epoch: "boot".into(),
                sequence: 0,
            },
            100,
        )
        .unwrap();
    {
        let mut state = runtime.state.lock();
        state
            .reserve("client", "waiting", "waiting")
            .and_then(|()| state.activate("waiting", None))
    }
    .unwrap();
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
        waiting.await.unwrap(),
        Some(StateSubscriptionEvent::Bookmark)
    ));
}

#[tokio::test(start_paused = true)]
async fn test_定期印_変更が送り待ちでもstreamへ間隔どおり送る() {
    // Given
    let runtime = StateSubscriptionRuntime::new("boot".into());
    runtime
        .state
        .lock()
        .register("target".into(), 0_u64, Delivery::Full)
        .unwrap();
    runtime.state.lock().open("client".into()).unwrap();
    {
        let mut state = runtime.state.lock();
        state
            .reserve("client", "target", "target")
            .and_then(|()| state.activate("target", None))
    }
    .unwrap();
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
    runtime.state.lock().publish("target", 1, None).unwrap();
    // When
    tokio::time::advance(std::time::Duration::from_secs(10)).await;
    // Then
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Bookmark)
    ));
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Item(_, Event::Change(_, _, _)))
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
fn test_定期印_版付き合図を積めたか返す() {
    // Given
    let mut state = registry();
    // When / Then
    assert!(!state.bookmark("client"));
    assert!(!state.bookmark("unknown"));
    state
        .reserve("client", "workspaces", "workspaces")
        .and_then(|()| {
            state.activate("workspaces", None).inspect_err(|_| {
                let _ = state.stop("client", "workspaces");
            })
        })
        .unwrap();
    assert!(!state.bookmark("client"));
    state.next("client");
    state.next("client");
    assert!(state.bookmark("client"));
}

#[test]
fn test_購読_初期状態と区切りの後に変更が届く() {
    // Given
    let mut state = registry();
    // When
    state
        .reserve("client", "workspaces", "workspaces")
        .and_then(|()| {
            state.activate("workspaces", None).inspect_err(|_| {
                let _ = state.stop("client", "workspaces");
            })
        })
        .unwrap();
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
    state
        .reserve("client", "workspaces", "workspaces")
        .and_then(|()| {
            state.activate("workspaces", None).inspect_err(|_| {
                let _ = state.stop("client", "workspaces");
            })
        })
        .unwrap();
    assert_eq!(
        state
            .reserve("client", "workspaces", "workspaces")
            .and_then(|()| state.activate("workspaces", None).inspect_err(|_| {
                let _ = state.stop("client", "workspaces");
            })),
        Err(SubscriptionError::AlreadyExists)
    );
    // Then
    assert_eq!(state.clients["client"].subscriptions.len(), 1);
    assert_eq!(
        state
            .reserve("client", "missing", "missing")
            .and_then(|()| state.activate("missing", None).inspect_err(|_| {
                let _ = state.stop("client", "missing");
            })),
        Err(SubscriptionError::UnknownTarget)
    );
    state.stop("client", "workspaces").unwrap();
    assert!(state.next("client").is_none());
    state.publish("workspaces", 1, None).unwrap();
    assert!(state.next("client").is_none());
    state
        .reserve("client", "providers", "providers")
        .and_then(|()| {
            state.activate("providers", None).inspect_err(|_| {
                let _ = state.stop("client", "providers");
            })
        })
        .unwrap();
    state.close("client");
    assert_eq!(
        state
            .reserve("client", "workspaces", "workspaces")
            .and_then(|()| state.activate("workspaces", None).inspect_err(|_| {
                let _ = state.stop("client", "workspaces");
            })),
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
    state
        .reserve("client", "workspaces", "workspaces")
        .and_then(|()| {
            state
                .activate("workspaces", Some(&version))
                .inspect_err(|_| {
                    let _ = state.stop("client", "workspaces");
                })
        })
        .unwrap();
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
        state
            .reserve("client", "workspaces", "workspaces")
            .and_then(|()| {
                state
                    .activate("workspaces", Some(&version))
                    .inspect_err(|_| {
                        let _ = state.stop("client", "workspaces");
                    })
            })
            .unwrap();
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
        .reserve("client", "workspaces", "workspaces")
        .and_then(|()| {
            state
                .activate(
                    "workspaces",
                    Some(&Version {
                        epoch: "boot".into(),
                        sequence: 0,
                    }),
                )
                .inspect_err(|_| {
                    let _ = state.stop("client", "workspaces");
                })
        })
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
    state
        .reserve("client", "workspaces", "workspaces")
        .and_then(|()| {
            state.activate("workspaces", None).inspect_err(|_| {
                let _ = state.stop("client", "workspaces");
            })
        })
        .unwrap();
    state
        .reserve("client", "providers", "providers")
        .and_then(|()| {
            state.activate("providers", None).inspect_err(|_| {
                let _ = state.stop("client", "providers");
            })
        })
        .unwrap();
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
        state
            .reserve("client", &id, &id)
            .and_then(|()| {
                state.activate(&id, None).inspect_err(|_| {
                    let _ = state.stop("client", &id);
                })
            })
            .unwrap();
    }
    state.publish("workspaces", 5, None).unwrap();
    state.close("client");
    state.open("client".into()).unwrap();
    state
        .reserve("client", "workspaces", "workspaces")
        .and_then(|()| {
            state.activate("workspaces", None).inspect_err(|_| {
                let _ = state.stop("client", "workspaces");
            })
        })
        .unwrap();
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
    state
        .reserve("client", "workspaces", "workspaces")
        .and_then(|()| {
            state.activate("workspaces", None).inspect_err(|_| {
                let _ = state.stop("client", "workspaces");
            })
        })
        .unwrap();
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
        .reserve("client", "workspaces", "workspaces")
        .and_then(|()| {
            state
                .activate(
                    "workspaces",
                    Some(&Version {
                        epoch: "boot".into(),
                        sequence: 64,
                    }),
                )
                .inspect_err(|_| {
                    let _ = state.stop("client", "workspaces");
                })
        })
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
    state
        .reserve("client", "workspaces", "workspaces")
        .and_then(|()| {
            state.activate("workspaces", None).inspect_err(|_| {
                let _ = state.stop("client", "workspaces");
            })
        })
        .unwrap();
    state
        .reserve("other", "other-workspaces", "workspaces")
        .and_then(|()| {
            state.activate("other-workspaces", None).inspect_err(|_| {
                let _ = state.stop("other", "other-workspaces");
            })
        })
        .unwrap();
    state.stop("client", "workspaces").unwrap();
    // Then
    assert!(state.active_targets().contains("workspaces"));
    state.close("other");
    assert!(!state.active_targets().contains("workspaces"));
}

#[test]
fn test_購読開始_切断で解放したsnapshotは再取得前に再開しない() {
    let mut state = registry();
    state
        .reserve("client", "workspaces", "workspaces")
        .and_then(|()| {
            state.activate("workspaces", None).inspect_err(|_| {
                let _ = state.stop("client", "workspaces");
            })
        })
        .unwrap();
    state.close("client");
    state.release_inactive_snapshots();
    state.open("next".into()).unwrap();
    assert_eq!(
        state
            .reserve("next", "workspaces", "workspaces")
            .and_then(|()| state.activate("workspaces", None).inspect_err(|_| {
                let _ = state.stop("next", "workspaces");
            })),
        Err(SubscriptionError::UnknownTarget)
    );
    state.publish("workspaces", 1, None).unwrap();
    state
        .reserve("next", "workspaces", "workspaces")
        .and_then(|()| {
            state.activate("workspaces", None).inspect_err(|_| {
                let _ = state.stop("next", "workspaces");
            })
        })
        .unwrap();
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
        state.reserve("closed", target, target).and_then(|()| state
            .activate(target, None)
            .inspect_err(|_| {
                let _ = state.stop("closed", target);
            })),
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
    state
        .reserve("client", target, target)
        .and_then(|()| {
            state.activate(target, None).inspect_err(|_| {
                let _ = state.stop("client", target);
            })
        })
        .unwrap();
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
    state
        .reserve("client", raw, raw)
        .and_then(|()| {
            state.activate(raw, None).inspect_err(|_| {
                let _ = state.stop("client", raw);
            })
        })
        .unwrap();
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
        state
            .reserve("client", raw, raw)
            .and_then(|()| {
                state.activate(raw, None).inspect_err(|_| {
                    let _ = state.stop("client", raw);
                })
            })
            .unwrap();
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
    state
        .reserve("client", target, target)
        .and_then(|()| {
            state.activate(target, None).inspect_err(|_| {
                let _ = state.stop("client", target);
            })
        })
        .unwrap();

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
    state
        .reserve("other", "providers", "providers")
        .and_then(|()| {
            state.activate("providers", None).inspect_err(|_| {
                let _ = state.stop("other", "providers");
            })
        })
        .unwrap();
    state
        .reserve("client", "workspaces", "workspaces")
        .and_then(|()| {
            state.activate("workspaces", None).inspect_err(|_| {
                let _ = state.stop("client", "workspaces");
            })
        })
        .unwrap();
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
    state
        .reserve("client", "providers", "providers")
        .and_then(|()| {
            state.activate("providers", None).inspect_err(|_| {
                let _ = state.stop("client", "providers");
            })
        })
        .unwrap();
    state
        .register("workspaces".into(), vec![], Delivery::Full)
        .unwrap();
    state
        .reserve("client", "workspaces", "workspaces")
        .and_then(|()| {
            state.activate("workspaces", None).inspect_err(|_| {
                let _ = state.stop("client", "workspaces");
            })
        })
        .unwrap();
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
    state
        .reserve("client", "workspaces", "workspaces")
        .and_then(|()| {
            state.activate("workspaces", None).inspect_err(|_| {
                let _ = state.stop("client", "workspaces");
            })
        })
        .unwrap();
    let (_, initial) = state.next("client").unwrap();
    let version = initial.version().clone();
    state.close("client");
    state.release_inactive_snapshots();
    // When
    state.publish("workspaces", 1, None).unwrap();
    assert_eq!(
        state
            .reserve("closed", "workspaces", "workspaces")
            .and_then(|()| state.activate("workspaces", None).inspect_err(|_| {
                let _ = state.stop("closed", "workspaces");
            })),
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
    state
        .reserve("next", "workspaces", "workspaces")
        .and_then(|()| {
            state
                .activate("workspaces", Some(&version))
                .inspect_err(|_| {
                    let _ = state.stop("next", "workspaces");
                })
        })
        .unwrap();
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
    state
        .reserve("client", target, target)
        .and_then(|()| {
            state.activate(target, None).inspect_err(|_| {
                let _ = state.stop("client", target);
            })
        })
        .unwrap();
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
    state
        .reserve("client", target, target)
        .and_then(|()| {
            state
                .activate(target, Some(&version(119)))
                .inspect_err(|_| {
                    let _ = state.stop("client", target);
                })
        })
        .unwrap();
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
        .reserve("client", target, target)
        .and_then(|()| {
            state
                .activate(
                    target,
                    Some(&Version {
                        epoch: "old".into(),
                        sequence: 0,
                    }),
                )
                .inspect_err(|_| {
                    let _ = state.stop("client", target);
                })
        })
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
    state
        .reserve("client", target, target)
        .and_then(|()| {
            state.activate(target, None).inspect_err(|_| {
                let _ = state.stop("client", target);
            })
        })
        .unwrap();
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
    {
        let mut state = runtime.state.lock();
        state
            .reserve("client", "target", "target")
            .and_then(|()| state.activate("target", None))
    }
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
    assert!(stream.next().now_or_never().is_none());
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
    state
        .reserve("client", target, target)
        .and_then(|()| {
            state.activate(target, None).inspect_err(|_| {
                let _ = state.stop("client", target);
            })
        })
        .unwrap();
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
    state
        .reserve("client", target, target)
        .and_then(|()| {
            state.activate(target, None).inspect_err(|_| {
                let _ = state.stop("client", target);
            })
        })
        .unwrap();
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
    state
        .reserve("client", target, target)
        .and_then(|()| {
            state.activate(target, Some(&version(1))).inspect_err(|_| {
                let _ = state.stop("client", target);
            })
        })
        .unwrap();
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
    state
        .reserve("client", target, target)
        .and_then(|()| {
            state.activate(target, None).inspect_err(|_| {
                let _ = state.stop("client", target);
            })
        })
        .unwrap();
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
        state
            .reserve(client, client, target)
            .and_then(|()| {
                state.activate(client, None).inspect_err(|_| {
                    let _ = state.stop(client, client);
                })
            })
            .unwrap();
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
    state
        .reserve("client", target, target)
        .and_then(|()| {
            state.activate(target, None).inspect_err(|_| {
                let _ = state.stop("client", target);
            })
        })
        .unwrap();
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
    assert!(state
        .lookup(target)
        .is_some_and(|(client, actual_target)| client == "client" && actual_target == target));
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
        state
            .reserve("client", target, target)
            .and_then(|()| {
                state.activate(target, None).inspect_err(|_| {
                    let _ = state.stop("client", target);
                })
            })
            .unwrap();
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
        assert!(state
            .lookup(target)
            .is_some_and(|(client, actual_target)| client == "client" && actual_target == target));
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
        assert!(!state
            .lookup(target)
            .is_some_and(|(client, actual_target)| client == "client" && actual_target == target));
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
    state
        .reserve("client", target, target)
        .and_then(|()| {
            state.activate(target, None).inspect_err(|_| {
                let _ = state.stop("client", target);
            })
        })
        .unwrap();
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
    assert!(state
        .lookup(target)
        .is_some_and(|(client, actual_target)| client == "client" && actual_target == target));
    state.stop("client", target).unwrap();
    assert!(!state
        .lookup(target)
        .is_some_and(|(client, actual_target)| client == "client" && actual_target == target));
}

#[test]
fn test_差分の番号判定_重複を捨て同版を積み古い非更新で復元を要求する() {
    // Given
    let mut state = Subscriptions::new("boot".into());
    let version = Version {
        epoch: "epoch".into(),
        sequence: 4,
    };
    state.register_delta("delta", version.clone(), 100).unwrap();
    state
        .set_delta_snapshot("delta", version.clone(), 0)
        .unwrap();
    state.open("client".into()).unwrap();
    state
        .reserve("client", "delta", "delta")
        .and_then(|()| {
            state.activate("delta", None).inspect_err(|_| {
                let _ = state.stop("client", "delta");
            })
        })
        .unwrap();
    // When / Then
    for sequence in [3, 4] {
        assert_eq!(
            state.apply_delta("delta", sequence, 1, 1, true),
            Ok(DeltaPublication::Discarded)
        );
        assert_eq!(state.current_version("delta"), Some(version.clone()));
    }
    assert_eq!(
        state.apply_delta("delta", 4, 2, 0, false),
        Ok(DeltaPublication::Published)
    );
    assert_eq!(
        state.apply_delta("delta", 3, 3, 0, false),
        Ok(DeltaPublication::SnapshotRequired(true))
    );
    assert_eq!(
        state.apply_delta("delta", 3, 3, 0, false),
        Ok(DeltaPublication::SnapshotRequired(false))
    );
    assert_eq!(state.snapshot_requests("client"), vec!["delta"]);
    assert_eq!(
        state.apply_delta("delta", 5, 4, 1, true),
        Ok(DeltaPublication::Published)
    );
    assert_eq!(state.current_version("delta").unwrap().sequence, 5);
    assert_eq!(
        state.apply_delta("absent", 5, 5, 1, true),
        Ok(DeltaPublication::Discarded)
    );
}

#[test]
fn test_対象単位の停止_他の開始途中の状態と差分履歴を保持する() {
    // Given
    let mut state = Subscriptions::new("boot".into());
    state
        .register("starting".into(), 10, Delivery::Full)
        .unwrap();
    let version = Version {
        epoch: "epoch".into(),
        sequence: 0,
    };
    state.register_delta("delta", version.clone(), 100).unwrap();
    state.set_delta_snapshot("delta", version, 0).unwrap();
    state.open("client".into()).unwrap();
    state
        .reserve("client", "delta", "delta")
        .and_then(|()| {
            state.activate("delta", None).inspect_err(|_| {
                let _ = state.stop("client", "delta");
            })
        })
        .unwrap();
    state.apply_delta("delta", 1, 1, 1, true).unwrap();
    let version = state.current_version("delta").unwrap();
    state
        .set_delta_snapshot("delta", version.clone(), 1)
        .unwrap();
    // When
    assert!(state
        .stop("client", "delta")
        .map(
            |stopped| state.release_inactive_snapshots_except(&std::collections::HashSet::from([
                "starting".into()
            ])) || stopped
        )
        .unwrap());
    // Then
    assert!(state
        .reserve("client", "starting", "starting")
        .and_then(|()| state.activate("starting", None).inspect_err(|_| {
            let _ = state.stop("client", "starting");
        }))
        .is_ok());
    assert!(!state.awaiting_snapshot("client", "delta"));
    assert!(state
        .reserve("client", "delta", "delta")
        .and_then(|()| state
            .activate(
                "delta",
                Some(&Version {
                    epoch: version.epoch,
                    sequence: 0
                })
            )
            .inspect_err(|_| {
                let _ = state.stop("client", "delta");
            }))
        .is_ok());
    assert!(
        matches!(state.next("client"), Some((target, Event::Snapshot(_, _))) if target == "starting")
    );
    assert!(!state.awaiting_snapshot("client", "delta"));
    assert!(
        matches!(state.next("client"), Some((target, Event::Change(_, Delivery::Delta, _))) if target == "delta")
    );
}

async fn assert_unregister_epoch(registered: bool, matching: bool, subscribed: bool) {
    // Given
    let runtime = StateSubscriptionRuntime::<u64>::new("boot".into());
    runtime
        .update(|state| {
            if registered {
                state.register_delta(
                    "target",
                    Version {
                        epoch: "current".into(),
                        sequence: 0,
                    },
                    100,
                )?;
                if subscribed {
                    state.open("client".into())?;
                    state.reserve("client", "target", "target").and_then(|()| {
                        state.activate("target", None).inspect_err(|_| {
                            let _ = state.stop("client", "target");
                        })
                    })?;
                }
            }
            Ok(true)
        })
        .unwrap();
    let changed = runtime.changed.notified();
    tokio::pin!(changed);
    changed.as_mut().enable();
    // When
    let result = runtime
        .mutate(|state| state.unregister_epoch("target", if matching { "current" } else { "old" }));
    // Then
    let removed = registered && matching;
    assert_eq!(result, removed && subscribed);
    assert_eq!(changed.as_mut().now_or_never().is_some(), removed);
    assert_eq!(
        runtime.inspect(|state| state.registered("target")),
        registered && !removed
    );
}

#[tokio::test]
async fn test_版指定の登録解除_対象が無ければ通知しない() {
    assert_unregister_epoch(false, true, false).await;
}

#[tokio::test]
async fn test_版指定の登録解除_epochが違えば通知せず保持する() {
    assert_unregister_epoch(true, false, true).await;
}

#[tokio::test]
async fn test_版指定の登録解除_一致して購読者ありならtrueを返し通知する() {
    assert_unregister_epoch(true, true, true).await;
}

#[tokio::test]
async fn test_版指定の登録解除_一致して購読者なしならfalseを返し通知する() {
    assert_unregister_epoch(true, true, false).await;
}

#[test]
fn test_購読識別子_全clientで重複を拒み停止と切断後は再利用できる() {
    // Given
    let mut state = registry();
    state.open("other".into()).unwrap();
    state
        .reserve("client", "x", "workspaces")
        .and_then(|()| {
            state.activate("x", None).inspect_err(|_| {
                let _ = state.stop("client", "x");
            })
        })
        .unwrap();
    state
        .reserve("client", "y", "workspaces")
        .and_then(|()| {
            state.activate("y", None).inspect_err(|_| {
                let _ = state.stop("client", "y");
            })
        })
        .unwrap();
    // When / Then
    for client in ["client", "other"] {
        assert_eq!(
            state.reserve(client, "x", "providers"),
            Err(SubscriptionError::AlreadyExists)
        );
    }
    assert_eq!(
        state.lookup("x"),
        Some(("client".into(), "workspaces".into()))
    );
    assert!(matches!(state.next("client"), Some((id, Event::Snapshot(_, _))) if id == "x"));
    assert!(matches!(state.next("client"), Some((id, Event::Snapshot(_, _))) if id == "y"));
    state.stop("client", "x").unwrap();
    state.next("client");
    state.publish("workspaces", 1, None).unwrap();
    assert!(
        matches!(state.next("client"), Some((id, Event::Change(_, _, value))) if id == "y" && *value == 1)
    );
    assert!(!state.stop("missing", "never-started").unwrap());
    state
        .reserve("other", "x", "providers")
        .and_then(|()| {
            state.activate("x", None).inspect_err(|_| {
                let _ = state.stop("other", "x");
            })
        })
        .unwrap();
    state.close("other");
    state
        .reserve("client", "x", "providers")
        .and_then(|()| {
            state.activate("x", None).inspect_err(|_| {
                let _ = state.stop("client", "x");
            })
        })
        .unwrap();
}

#[test]
fn test_購読識別子_同時の予約は一つだけ受理する() {
    // Given
    let runtime = StateSubscriptionRuntime::<u64>::new("boot".into());
    runtime
        .update(|state| {
            state.open("a".into())?;
            state.open("b".into())?;
            Ok(false)
        })
        .unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    // When
    let results = std::thread::scope(|scope| {
        let threads: Vec<_> = ["a", "b"]
            .into_iter()
            .map(|client| {
                let runtime = runtime.clone();
                let barrier = barrier.clone();
                scope.spawn(move || {
                    barrier.wait();
                    runtime.update(|state| state.reserve(client, "x", "target").map(|_| false))
                })
            })
            .collect();
        threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>()
    });
    // Then
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| **result == Err(SubscriptionError::AlreadyExists))
            .count(),
        1
    );
}

#[test]
fn test_差分配信_同じclientの二つの識別子へ送り片方の停止後も続ける() {
    // Given
    let mut state = registry();
    let version = Version {
        epoch: "terminal".into(),
        sequence: 0,
    };
    state
        .register_delta("terminal", version.clone(), 100)
        .unwrap();
    state
        .set_delta_snapshot("terminal", version.clone(), 0)
        .unwrap();
    for id in ["x", "y"] {
        state
            .reserve("client", id, "terminal")
            .and_then(|()| {
                state.activate(id, None).inspect_err(|_| {
                    let _ = state.stop("client", id);
                })
            })
            .unwrap();
    }
    for _ in 0..4 {
        state.next("client").unwrap();
    }
    // When
    state
        .publish_delta(
            "terminal",
            Version {
                sequence: 1,
                ..version.clone()
            },
            1,
            5,
            true,
        )
        .unwrap();
    // Then
    for id in ["x", "y"] {
        assert!(
            matches!(state.next("client"), Some((delivered, Event::Change(_, Delivery::Delta, value))) if delivered == id && *value == 1)
        );
    }
    state
        .stop("client", "x")
        .map(|stopped| {
            state.release_inactive_snapshots_except(&std::collections::HashSet::new()) || stopped
        })
        .unwrap();
    state
        .publish_delta(
            "terminal",
            Version {
                sequence: 2,
                ..version
            },
            2,
            5,
            true,
        )
        .unwrap();
    assert!(
        matches!(state.next("client"), Some((id, Event::Change(_, Delivery::Delta, value))) if id == "y" && *value == 2)
    );
    assert_eq!(state.pending_amount("client", "y"), 0);
    assert!(state.has_subscribers("terminal"));
}

mod driver_tests {
    use super::*;
    use crate::test_support::state_subscription::*;

    #[test]
    fn test_テスト駆動_runtime外でも受け側を保持し送信を受理する() {
        // Given
        let sender = driver::<u8>(|| panic!("runtime外では駆動を起こさない"));
        // When / Then
        assert!(sender.send(1).is_ok());
        assert!(!read_driver().is_closed());
        assert!(!pending_read_driver().is_closed());
        assert!(!terminal_driver().is_closed());
        assert!(!repository_driver().is_closed());
    }

    #[tokio::test]
    async fn test_テスト駆動_runtime内では渡された駆動を起こす() {
        // Given
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        // When
        let sender = driver(|| sender);
        sender.send(1_u8).unwrap();
        // Then
        assert_eq!(receiver.recv().await, Some(1));
    }

    mod scenarios {
        use super::shared_test_helpers::{start_read, stop_read};
        use crate::usecase::state_subscription::*;
        use futures_util::{Stream, StreamExt};
        use parking_lot::Mutex;
        use std::sync::Arc;

        use crate::infrastructure::state_subscription::Version;
        use crate::test_support::state_subscription::WakeFlag;

        use crate::test_support::state_subscription::same;
        use crate::test_support::state_subscription::{Delivery, Event, StateSubscriptionEvent};
        use crate::usecase::state_subscription::{
            StateReadError, StateReadFailure, StateSubscriptionRead,
        };
        const BOOKMARK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

        #[tokio::test(start_paused = true)]
        async fn test_購読_配信と定期印と終了時の解放() {
            // Given
            let usecase = StateSubscriptionUsecase::new(
                vec!["/repo".into()],
                crate::test_support::state_subscription::read_driver(),
            );
            let mut stream = Box::pin(usecase.open("client".into()).unwrap());
            assert!(matches!(
                stream.next().await,
                Some(StateSubscriptionEvent::Ready)
            ));
            // When
            start_read(
                &usecase,
                "client",
                &SubscriptionTarget::RepositoryPaths.to_string(),
                None,
            )
            .await
            .unwrap();
            // Then
            assert!(
                matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) if same(&value, StateValue::RepositoryPaths(vec!["/repo".into()])))
            );
            assert!(matches!(
                stream.next().await,
                Some(StateSubscriptionEvent::Item(_, Event::Bookmark(_)))
            ));
            usecase.test_set_repository_paths(vec!["/next".into()]);
            usecase.notify(StateChangeSource::Repositories);
            assert!(matches!(
                stream.next().await,
                Some(StateSubscriptionEvent::Item(
                    _,
                    Event::Change(Version { sequence: 1, .. }, Delivery::Full, _)
                ))
            ));
            tokio::time::advance(BOOKMARK_INTERVAL).await;
            assert!(matches!(
                stream.next().await,
                Some(StateSubscriptionEvent::Item(
                    _,
                    Event::Bookmark(Version { sequence: 1, .. })
                ))
            ));
            drop(stream);
            assert!(
                matches!(start_read(&usecase, "client", &SubscriptionTarget::RepositoryPaths.to_string(), None).await,
        Err(StateReadError { source: StateReadFailure::Subscription(error), .. }) if *error == SubscriptionError::StreamEnded)
            );
            assert!(usecase.open("client".into()).is_ok());
        }

        #[tokio::test(start_paused = true)]
        async fn test_購読_開始と配信と停止が待機中streamを起こす() {
            use std::sync::atomic::{AtomicBool, Ordering};
            use std::task::{Context, Poll, Waker};

            // Given
            let usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            );
            let mut stream = Box::pin(usecase.open("waiting".into()).unwrap());
            assert!(matches!(
                stream.next().await,
                Some(StateSubscriptionEvent::Ready)
            ));
            let flag = Arc::new(WakeFlag(AtomicBool::new(false)));
            let waker = Waker::from(flag.clone());
            let mut cx = Context::from_waker(&waker);
            assert!(stream.as_mut().poll_next(&mut cx).is_pending());
            flag.0.store(false, Ordering::SeqCst);

            assert!(
                matches!(start_read(&usecase, "waiting", "missing", None).await,
        Err(StateReadError { source: StateReadFailure::Subscription(error), .. }) if *error == SubscriptionError::UnknownTarget)
            );
            assert_eq!(
                usecase.publisher().publish(
                    &SubscriptionTarget::BranchBase("/missing".into(), "branch".into()),
                    StateValue::RepositoryPaths(vec![]),
                    None
                ),
                Err(SubscriptionError::UnknownTarget)
            );
            assert!(!flag.0.load(Ordering::SeqCst));

            // When
            start_read(
                &usecase,
                "waiting",
                &SubscriptionTarget::RepositoryPaths.to_string(),
                None,
            )
            .await
            .unwrap();

            // Then
            assert!(flag.0.swap(false, Ordering::SeqCst));
            assert!(matches!(
                stream.as_mut().poll_next(&mut cx),
                Poll::Ready(Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, _))))
            ));
            assert!(matches!(
                stream.as_mut().poll_next(&mut cx),
                Poll::Ready(Some(StateSubscriptionEvent::Item(_, Event::Bookmark(_))))
            ));
            while let Poll::Ready(Some(event)) = stream.as_mut().poll_next(&mut cx) {
                assert!(matches!(
                    event,
                    StateSubscriptionEvent::Item(_, Event::Bookmark(_))
                ));
            }
            flag.0.store(false, Ordering::SeqCst);

            // When
            usecase.test_set_repository_paths(vec!["/next".into()]);
            usecase.notify(StateChangeSource::Repositories);
            tokio::task::yield_now().await;

            // Then
            assert!(flag.0.load(Ordering::SeqCst));
            assert!(
                matches!(stream.as_mut().poll_next(&mut cx), Poll::Ready(Some(StateSubscriptionEvent::Item(_, Event::Change(Version { sequence: 1, .. }, Delivery::Full, value)))) if same(&value, StateValue::RepositoryPaths(vec!["/next".into()])))
            );

            assert!(stream.as_mut().poll_next(&mut cx).is_pending());
            flag.0.store(false, Ordering::SeqCst);
            assert_eq!(
                stop_read(
                    &usecase,
                    "missing",
                    &SubscriptionTarget::RepositoryPaths.to_string()
                )
                .await,
                Ok(())
            );
            assert!(!flag.0.load(Ordering::SeqCst));

            // When
            stop_read(
                &usecase,
                "waiting",
                &SubscriptionTarget::RepositoryPaths.to_string(),
            )
            .await
            .unwrap();

            // Then
            assert!(flag.0.swap(false, Ordering::SeqCst));
            assert!(stream.as_mut().poll_next(&mut cx).is_pending());
            usecase.notify(StateChangeSource::Repositories);
            assert!(stream.as_mut().poll_next(&mut cx).is_pending());
            tokio::time::advance(BOOKMARK_INTERVAL).await;
            assert!(matches!(
                stream.as_mut().poll_next(&mut cx),
                Poll::Ready(Some(StateSubscriptionEvent::Bookmark))
            ));
            assert!(stream.as_mut().poll_next(&mut cx).is_pending());
        }

        struct FakeReads {
            value: Mutex<String>,
            calls: std::sync::atomic::AtomicUsize,
        }
        #[async_trait::async_trait]
        impl StateSubscriptionRead for FakeReads {
            async fn read(
                &self,
                target: &crate::usecase::state_subscription::SubscriptionTarget,
            ) -> Result<StateValue, StateReadError> {
                self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(match target {
                    crate::usecase::state_subscription::SubscriptionTarget::ReleashBase(_) => {
                        StateValue::ReleashBase(Some(self.value.lock().clone()))
                    }
                    crate::usecase::state_subscription::SubscriptionTarget::WorkflowSource(_) => {
                        StateValue::WorkflowSource(Some(self.value.lock().clone()))
                    }
                    crate::usecase::state_subscription::SubscriptionTarget::SessionHistory(
                        _,
                        _,
                    ) => StateValue::SessionHistory(
                        crate::usecase::agent_session::AgentSessionHistoryPageDto {
                            items: vec![],
                            has_more: false,
                        },
                    ),
                    _ => StateValue::Issues(crate::usecase::fetched::Fetched::ready(vec![])),
                })
            }
            fn repositories(&self) -> Vec<String> {
                vec![]
            }
            fn workflows_dir(&self) -> String {
                "/workflows".into()
            }
        }

        #[tokio::test]
        async fn test_引数付き購読_対象の変更だけを読み直して配信し終了でworkerを解放する() {
            use crate::usecase::state_subscription::{StateChangeSource, SubscriptionTarget};
            // Given
            let reads = Arc::new(FakeReads {
                value: Mutex::new("node-1".into()),
                calls: Default::default(),
            });
            let usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            )
            .with_reads(reads.clone(), None, vec![], String::new());
            let mut stream = Box::pin(usecase.open("client".into()).unwrap());
            stream.next().await;
            let target = SubscriptionTarget::ReleashBase("/repo".into()).to_string();
            // When
            start_read(&usecase, "client", &target, None).await.unwrap();
            // Then
            assert!(
                matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) if same(&value, StateValue::ReleashBase(Some("node-1".into()))))
            );
            stream.next().await;
            *reads.value.lock() = "node-2".into();
            usecase.notify(StateChangeSource::Repository(vec!["/repo".into()]));
            assert!(
                matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Change(_, _, value))) if same(&value, StateValue::ReleashBase(Some("node-2".into()))))
            );
            drop(stream);
            assert_eq!(usecase.test_worker_count(), 0);
            assert!(usecase
                .test_presenter()
                .unwrap()
                .test_runtime()
                .inspect(|state| state.active_targets().is_empty()));
        }

        #[tokio::test(start_paused = true)]
        async fn test_外部情報_購読者がいる間だけcache_ttlで取得する() {
            use crate::usecase::state_subscription::SubscriptionTarget;
            use std::sync::atomic::Ordering;
            // Given
            let reads = Arc::new(FakeReads {
                value: Mutex::new(String::new()),
                calls: Default::default(),
            });
            let usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            )
            .with_reads(reads.clone(), None, vec![], String::new());
            let stream = usecase.open("client".into()).unwrap();
            let target = SubscriptionTarget::Issues("/repo".into()).to_string();
            start_read(&usecase, "client", &target, None).await.unwrap();
            tokio::task::yield_now().await;
            let initial = reads.calls.load(Ordering::SeqCst);
            // When
            tokio::time::advance(
                crate::domain::git_host::CacheTtl::EXTERNAL_INFORMATION.duration(),
            )
            .await;
            tokio::task::yield_now().await;
            // Then
            assert!(reads.calls.load(Ordering::SeqCst) > initial);
            drop(stream);
            let stopped = reads.calls.load(Ordering::SeqCst);
            tokio::time::advance(std::time::Duration::from_secs(60)).await;
            tokio::task::yield_now().await;
            assert_eq!(reads.calls.load(Ordering::SeqCst), stopped);
        }

        #[tokio::test]
        async fn test_履歴購読_件数違いと別clientが監視を共有し最後の終了で解放する() {
            use crate::usecase::state_subscription::SubscriptionTarget;
            let files = Arc::new(
                crate::usecase::watcher::watcher_test_helpers::SubscriptionFiles::default(),
            );
            let usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            )
            .with_reads(
                Arc::new(FakeReads {
                    value: Mutex::new(String::new()),
                    calls: Default::default(),
                }),
                Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
                    None,
                    files.clone(),
                ))),
                vec!["/claude".into(), "/codex".into()],
                String::new(),
            );
            let first = usecase.open("first".into()).unwrap();
            let second = usecase.open("second".into()).unwrap();
            let target = SubscriptionTarget::SessionHistory("/repo".into(), 20).to_string();
            start_read(&usecase, "first", &target, None).await.unwrap();
            start_read(&usecase, "second", &target, None).await.unwrap();
            assert_eq!(files.active.lock().unwrap().len(), 2);
            assert_eq!(usecase.test_worker_count(), 1);
            stop_read(&usecase, "first", &target).await.unwrap();
            let expanded = SubscriptionTarget::SessionHistory("/repo".into(), 40).to_string();
            start_read(&usecase, "first", &expanded, None)
                .await
                .unwrap();
            assert_eq!(files.active.lock().unwrap().len(), 2);
            drop(second);
            assert_eq!(files.active.lock().unwrap().len(), 2);
            drop(first);
            assert!(files.active.lock().unwrap().is_empty());
            assert_eq!(usecase.test_worker_count(), 0);
        }

        #[tokio::test]
        async fn test_automation購読_置き場の監視を共有し最後の終了で解放する() {
            use crate::usecase::state_subscription::{StateChangeSource, WatchRequirement};
            // Given
            let files = Arc::new(
                crate::usecase::watcher::watcher_test_helpers::SubscriptionFiles::default(),
            );
            let usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            )
            .with_reads(
                Arc::new(FakeReads {
                    value: Mutex::new(String::new()),
                    calls: Default::default(),
                }),
                Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
                    None,
                    files.clone(),
                ))),
                vec![],
                String::new(),
            );
            let first = usecase.open("first".into()).unwrap();
            let second = usecase.open("second".into()).unwrap();
            // When
            start_read(&usecase, "first", "workflows", None)
                .await
                .unwrap();
            start_read(&usecase, "second", "diagnostics", None)
                .await
                .unwrap();
            // Then
            let requirement = WatchRequirement::Files(
                "/workflows".into(),
                StateChangeSource::WorkflowDefinitions,
            );
            assert_eq!(
                usecase.test_watches().keys().collect::<Vec<_>>(),
                vec![&requirement]
            );
            assert_eq!(files.active.lock().unwrap().len(), 1);
            assert_eq!(usecase.test_worker_count(), 2);
            drop(first);
            assert_eq!(files.active.lock().unwrap().len(), 1);
            assert_eq!(usecase.test_worker_count(), 1);
            drop(second);
            assert!(files.active.lock().unwrap().is_empty());
            assert!(usecase.test_watches().is_empty());
            assert_eq!(usecase.test_worker_count(), 0);
        }

        struct CapturingFiles {
            on_change: Mutex<Option<crate::domain::repository::file_watcher::WatchChangeHandler>>,
        }
        impl crate::domain::repository::file_watcher::FileWatchGateway for CapturingFiles {
            fn start_tree(
                &self,
                _: &str,
                on_change: crate::domain::repository::file_watcher::WatchChangeHandler,
            ) -> Result<u64, String> {
                *self.on_change.lock() = Some(on_change);
                Ok(1)
            }
            fn stop(&self, _: u64) -> Result<(), String> {
                Ok(())
            }
        }

        #[tokio::test]
        async fn test_automation購読_置き場のファイル変化で読み直して配信する() {
            // Given
            let files = Arc::new(CapturingFiles {
                on_change: Mutex::new(None),
            });
            let reads = Arc::new(FakeReads {
                value: Mutex::new("first".into()),
                calls: Default::default(),
            });
            let usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            )
            .with_reads(
                reads.clone(),
                Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
                    None,
                    files.clone(),
                ))),
                vec![],
                String::new(),
            );
            let mut stream = Box::pin(usecase.open("client".into()).unwrap());
            stream.next().await;
            let target = crate::usecase::state_subscription::SubscriptionTarget::WorkflowSource(
                "dev".into(),
            )
            .to_string();
            start_read(&usecase, "client", &target, None).await.unwrap();
            assert!(matches!(
                stream.next().await,
                Some(StateSubscriptionEvent::Item(_, Event::Snapshot(..)))
            ));
            let on_change = files.on_change.lock().clone().unwrap();
            // When
            *reads.value.lock() = "second".into();
            on_change(Ok(()));
            // Then
            let value = tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    if let Some(StateSubscriptionEvent::Item(id, Event::Change(_, _, value))) =
                        stream.next().await
                    {
                        assert_eq!(id, format!("client:{target}"));
                        break value;
                    }
                }
            })
            .await
            .unwrap();
            assert!(same(
                &value,
                &StateValue::WorkflowSource(Some("second".into()))
            ));
        }

        #[tokio::test]
        async fn test_監視開始失敗_失敗を購読へ届け再開で張り直す() {
            use crate::usecase::state_subscription::SubscriptionTarget;
            let files = Arc::new(
                crate::usecase::watcher::watcher_test_helpers::SubscriptionFiles::default(),
            );
            let usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            )
            .with_reads(
                Arc::new(FakeReads {
                    value: Mutex::new(String::new()),
                    calls: Default::default(),
                }),
                Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
                    None,
                    files.clone(),
                ))),
                vec!["/missing".into()],
                String::new(),
            );
            let mut stream = Box::pin(usecase.open("client".into()).unwrap());
            stream.next().await;
            let target = SubscriptionTarget::SessionHistory("/repo".into(), 20).to_string();
            start_read(&usecase, "client", &target, None).await.unwrap();
            assert!(matches!(stream.next().await,
        Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value)))
            if matches!(value.as_ref(), crate::adaptor::presenter::state_subscription::PublishedState::Failure(error)
                if error.message.contains("missing path"))));
            assert!(usecase
                .test_presenter()
                .unwrap()
                .test_runtime()
                .inspect(|state| !state.active_targets().is_empty()));
            stop_read(&usecase, "client", &target).await.unwrap();
            let usecase = usecase.with_reads(
                Arc::new(FakeReads {
                    value: Mutex::new(String::new()),
                    calls: Default::default(),
                }),
                Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
                    None,
                    files.clone(),
                ))),
                vec!["/history".into()],
                String::new(),
            );
            start_read(&usecase, "client", &target, None).await.unwrap();
            assert_eq!(files.active.lock().unwrap().len(), 1);
        }

        struct BlockedReads {
            entered: tokio::sync::Notify,
            release: tokio::sync::Notify,
        }
        #[async_trait::async_trait]
        impl StateSubscriptionRead for BlockedReads {
            async fn read(
                &self,
                _: &crate::usecase::state_subscription::SubscriptionTarget,
            ) -> Result<StateValue, StateReadError> {
                self.entered.notify_one();
                self.release.notified().await;
                Ok(StateValue::SessionHistory(
                    crate::usecase::agent_session::AgentSessionHistoryPageDto {
                        items: vec![],
                        has_more: false,
                    },
                ))
            }
            fn repositories(&self) -> Vec<String> {
                vec![]
            }
        }
        #[tokio::test]
        async fn test_購読停止_初回読取中の停止要求でも監視とworkerを残さない() {
            use crate::usecase::state_subscription::SubscriptionTarget;
            let reads = Arc::new(BlockedReads {
                entered: Default::default(),
                release: Default::default(),
            });
            let files = Arc::new(
                crate::usecase::watcher::watcher_test_helpers::SubscriptionFiles::default(),
            );
            let usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            )
            .with_reads(
                reads.clone(),
                Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
                    None,
                    files.clone(),
                ))),
                vec!["/history".into()],
                String::new(),
            );
            let _stream = usecase.open("client".into()).unwrap();
            let target = SubscriptionTarget::SessionHistory("/repo".into(), 20).to_string();
            let started = tokio::spawn({
                let usecase = usecase.clone();
                let target = target.clone();
                async move { start_read(&usecase, "client", &target, None).await }
            });
            reads.entered.notified().await;
            let stopped = tokio::spawn({
                let usecase = usecase.clone();
                async move { stop_read(&usecase, "client", &target).await }
            });
            tokio::task::yield_now().await;
            assert!(!stopped.is_finished());
            reads.release.notify_one();
            started.await.unwrap().unwrap();
            stopped.await.unwrap().unwrap();
            assert!(files.active.lock().unwrap().is_empty());
            assert_eq!(usecase.test_worker_count(), 0);
            assert!(usecase
                .test_presenter()
                .unwrap()
                .test_runtime()
                .inspect(|state| state.active_targets().is_empty()));
        }

        struct NullableReads;
        #[async_trait::async_trait]
        impl StateSubscriptionRead for NullableReads {
            async fn read(
                &self,
                target: &crate::usecase::state_subscription::SubscriptionTarget,
            ) -> Result<StateValue, StateReadError> {
                Ok(match target {
                    crate::usecase::state_subscription::SubscriptionTarget::AgentSession(_) => {
                        StateValue::AgentSession(None)
                    }
                    crate::usecase::state_subscription::SubscriptionTarget::NodeDetail(_, _) => {
                        StateValue::NodeDetail(None)
                    }
                    _ => panic!("unexpected target"),
                })
            }
            fn repositories(&self) -> Vec<String> {
                vec![]
            }
        }

        #[tokio::test]
        async fn test_不在対象_初回からnullable_snapshotとして配信する() {
            use crate::usecase::state_subscription::SubscriptionTarget;
            let usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            )
            .with_reads(Arc::new(NullableReads), None, vec![], String::new());
            let mut stream = Box::pin(usecase.open("client".into()).unwrap());
            stream.next().await;
            for (target, expected) in [
                (
                    SubscriptionTarget::AgentSession("missing".into()),
                    StateValue::AgentSession(None),
                ),
                (
                    SubscriptionTarget::NodeDetail("/repo".into(), "missing".into()),
                    StateValue::NodeDetail(None),
                ),
            ] {
                start_read(&usecase, "client", &target.to_string(), None)
                    .await
                    .unwrap();
                assert!(
                    matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) if same(&value, expected))
                );
                assert!(matches!(
                    stream.next().await,
                    Some(StateSubscriptionEvent::Item(_, Event::Bookmark(_)))
                ));
            }
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn test_購読開始と切断_同じ対象の最終購読者が切断しても再開できる() {
            use crate::usecase::state_subscription::SubscriptionTarget;
            let usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            )
            .with_reads(Arc::new(NullableReads), None, vec![], String::new());
            let target = SubscriptionTarget::AgentSession("missing".into()).to_string();
            for index in 0..100 {
                let old_id = format!("old-{index}");
                let next_id = format!("next-{index}");
                let old = usecase.open(old_id.clone()).unwrap();
                start_read(&usecase, &old_id, &target, None).await.unwrap();
                let mut next = Box::pin(usecase.open(next_id.clone()).unwrap());
                next.next().await;
                let barrier = Arc::new(tokio::sync::Barrier::new(2));
                let closed = tokio::spawn({
                    let barrier = barrier.clone();
                    async move {
                        barrier.wait().await;
                        drop(old);
                    }
                });
                barrier.wait().await;
                start_read(&usecase, &next_id, &target, None).await.unwrap();
                closed.await.unwrap();
                assert!(
                    matches!(next.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) if same(&value, StateValue::AgentSession(None)))
                );
                drop(next);
                assert_eq!(usecase.test_worker_count(), 0);
            }
        }

        #[tokio::test(start_paused = true)]
        async fn test_初回読取_保持中の版から再開して変更だけ届ける() {
            let usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            )
            .with_reads(
                Arc::new(FakeReads {
                    value: Mutex::new("after".into()),
                    calls: Default::default(),
                }),
                None,
                vec![],
                String::new(),
            );
            let target = SubscriptionTarget::ReleashBase("/repo".into()).to_string();
            let presenter = usecase.test_presenter().unwrap();
            let before = crate::test_support::state_subscription::payload(
                &StateValue::ReleashBase(Some("before".into())),
            )
            .unwrap();
            let after = crate::test_support::state_subscription::payload(&StateValue::ReleashBase(
                Some("after".into()),
            ))
            .unwrap();
            let version = presenter.test_runtime().mutate(|state| {
                state
                    .register(target.clone(), before, Delivery::Full)
                    .unwrap();
                let version = state.current_version(&target).unwrap();
                state.publish(&target, after, None).unwrap();
                (version, true)
            });
            let mut stream = Box::pin(usecase.open("client".into()).unwrap());
            assert!(matches!(
                stream.next().await,
                Some(StateSubscriptionEvent::Ready)
            ));

            tokio::time::advance(BOOKMARK_INTERVAL - std::time::Duration::from_millis(1)).await;

            start_read(
                &usecase,
                "client",
                &target,
                Some((&version.epoch, version.sequence)),
            )
            .await
            .unwrap();

            assert!(matches!(
                stream.next().await,
                Some(StateSubscriptionEvent::Item(_, Event::Change(next, Delivery::Full, value)))
                    if next.sequence == version.sequence + 1
                        && same(&value, StateValue::ReleashBase(Some("after".into())))
            ));
            assert!(matches!(
                stream.next().await,
                Some(StateSubscriptionEvent::Item(_, Event::Bookmark(next)))
                    if next.sequence == version.sequence + 1
            ));
            tokio::time::advance(std::time::Duration::from_millis(1)).await;
            assert!(matches!(
                stream.next().await,
                Some(StateSubscriptionEvent::Item(_, Event::Bookmark(next)))
                    if next.sequence == version.sequence + 1
            ));
        }

        #[derive(Default)]
        struct ExternalReads {
            issues: std::sync::atomic::AtomicU64,
            prs: std::sync::atomic::AtomicU64,
        }
        impl ExternalReads {
            fn value(
                &self,
                target: &crate::usecase::state_subscription::SubscriptionTarget,
            ) -> StateValue {
                use crate::usecase::fetched::Fetched;
                use crate::usecase::workspace_tree::{
                    WorkspaceList, WorkspaceListRepository, WorkspaceListWorktree,
                };
                use std::sync::atomic::Ordering;
                if matches!(
                    target,
                    crate::usecase::state_subscription::SubscriptionTarget::Issues(_)
                ) {
                    return StateValue::Issues(Fetched::ready(vec![
                        crate::domain::git_host::IssueInfo {
                            number: self.issues.load(Ordering::SeqCst),
                            title: "issue".into(),
                            state: "open".into(),
                            url: String::new(),
                            author: crate::domain::git_host::PrAuthor {
                                login: "author".into(),
                            },
                            created_at: String::new(),
                            updated_at: String::new(),
                            labels: vec![],
                            assignees: vec![],
                            body: String::new(),
                            milestone: None,
                        },
                    ]));
                }
                StateValue::Workspaces(WorkspaceList {
                    repositories: vec![WorkspaceListRepository {
                        path: "/repo".into(),
                        worktrees: Fetched::ready(vec![WorkspaceListWorktree {
                            worktree: crate::domain::repository::Worktree {
                                name: "main".into(),
                                path: "/repo".into(),
                                branch: "main".into(),
                                is_main: true,
                                is_locked: false,
                                is_merged: false,
                            },
                            deleting: false,
                            dirty_count: Fetched::ready(0),
                            pull_request_error: None,
                            pull_request_loaded: true,
                            merged: false,
                            pull_request: Some(crate::domain::git_host::PrInfo {
                                number: self.prs.load(Ordering::SeqCst),
                                url: String::new(),
                            }),
                            tree: Fetched::default(),
                        }]),
                    }],
                })
            }
        }
        #[async_trait::async_trait]
        impl StateSubscriptionRead for ExternalReads {
            async fn read(
                &self,
                target: &crate::usecase::state_subscription::SubscriptionTarget,
            ) -> Result<StateValue, StateReadError> {
                Ok(self.value(target))
            }
            async fn refresh_external(
                &self,
                target: &crate::usecase::state_subscription::SubscriptionTarget,
            ) -> Result<(), StateReadError> {
                let count = if matches!(
                    target,
                    crate::usecase::state_subscription::SubscriptionTarget::Issues(_)
                ) {
                    &self.issues
                } else {
                    &self.prs
                };
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
            fn repositories(&self) -> Vec<String> {
                vec!["/repo".into()]
            }
        }

        #[tokio::test(start_paused = true)]
        async fn test_外部情報ttl_issueとprを取得し更新値を配信して停止後は取得しない() {
            use crate::usecase::state_subscription::SubscriptionTarget;
            use std::sync::atomic::Ordering;
            for target in [
                SubscriptionTarget::Issues("/repo".into()),
                SubscriptionTarget::Workspaces,
            ] {
                let reads = Arc::new(ExternalReads::default());
                let usecase = StateSubscriptionUsecase::new(
                    vec![],
                    crate::test_support::state_subscription::read_driver(),
                )
                .with_reads(reads.clone(), None, vec![], String::new());
                let mut stream = Box::pin(usecase.open("client".into()).unwrap());
                stream.next().await;
                start_read(&usecase, "client", &target.to_string(), None)
                    .await
                    .unwrap();
                let count = if matches!(target, SubscriptionTarget::Issues(_)) {
                    &reads.issues
                } else {
                    &reads.prs
                };
                assert_eq!(count.load(Ordering::SeqCst), 1);
                assert!(
                    matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) if same(&value, reads.value(&target)))
                );
                stream.next().await;
                tokio::task::yield_now().await;
                tokio::time::advance(std::time::Duration::from_secs(29)).await;
                tokio::task::yield_now().await;
                assert_eq!(count.load(Ordering::SeqCst), 1);
                tokio::time::advance(std::time::Duration::from_secs(1)).await;
                tokio::task::yield_now().await;
                assert_eq!(count.load(Ordering::SeqCst), 2);
                let changed = tokio::time::timeout(std::time::Duration::from_secs(1), async {
                    loop {
                        if let Some(StateSubscriptionEvent::Item(
                            _,
                            Event::Change(_, Delivery::Full, value),
                        )) = stream.next().await
                        {
                            break value;
                        }
                    }
                })
                .await
                .unwrap();
                assert!(same(&changed, reads.value(&target)));
                drop(stream);
                tokio::time::advance(std::time::Duration::from_secs(60)).await;
                tokio::task::yield_now().await;
                assert_eq!(count.load(Ordering::SeqCst), 2);
            }
        }

        #[tokio::test]
        async fn test_初回読取中の切断_開始失敗後に対象の鍵もworkerも残さない() {
            use crate::usecase::state_subscription::SubscriptionTarget;
            // Given
            let reads = Arc::new(BlockedReads {
                entered: Default::default(),
                release: Default::default(),
            });
            let usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            )
            .with_reads(reads.clone(), None, vec![], String::new());
            let stream = usecase.open("client".into()).unwrap();
            let target = SubscriptionTarget::SessionHistory("/repo".into(), 20);
            let started = tokio::spawn({
                let usecase = usecase.clone();
                let raw = target.to_string();
                async move { start_read(&usecase, "client", &raw, None).await }
            });
            reads.entered.notified().await;
            // When
            drop(stream);
            reads.release.notify_one();
            // Then
            let error = started.await.unwrap().unwrap_err();
            assert!(
                matches!(error.source, StateReadFailure::Subscription(source) if *source == SubscriptionError::StreamEnded)
            );
            assert_eq!(error.message, SubscriptionError::StreamEnded.to_string());
            assert!(!usecase
                .test_presenter()
                .unwrap()
                .test_runtime()
                .inspect(|state| state.registered(&target.to_string())));
            assert!(!usecase.test_presenter().unwrap().test_runtime().inspect(
                |state| state.registered(&SubscriptionTarget::RepositoryPaths.to_string())
            ));
            assert_eq!(usecase.test_worker_count(), 0);
        }

        struct DisconnectingFiles {
            usecase: StateSubscriptionUsecase,
        }
        impl crate::domain::repository::file_watcher::FileWatchGateway for DisconnectingFiles {
            fn start_tree(
                &self,
                _: &str,
                _: crate::domain::repository::file_watcher::WatchChangeHandler,
            ) -> Result<u64, String> {
                self.usecase.close_client("client");
                Ok(1)
            }
            fn stop(&self, _: u64) -> Result<(), String> {
                Ok(())
            }
        }

        #[tokio::test]
        async fn test_snapshot登録後の切断_開始失敗で対象の鍵を解放する() {
            use crate::usecase::state_subscription::SubscriptionTarget;
            // Given
            let mut usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            );
            let files = Arc::new(DisconnectingFiles {
                usecase: usecase.clone(),
            });
            usecase = usecase.with_reads(
                Arc::new(FakeReads {
                    value: Mutex::new(String::new()),
                    calls: Default::default(),
                }),
                Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
                    None, files,
                ))),
                vec!["/history".into()],
                String::new(),
            );
            let _stream = usecase.open("client".into()).unwrap();
            let target = SubscriptionTarget::SessionHistory("/repo".into(), 20);
            // When
            let error = start_read(&usecase, "client", &target.to_string(), None)
                .await
                .unwrap_err();
            // Then
            assert_eq!(error.message, "StreamEnded");
            assert!(!usecase
                .test_presenter()
                .unwrap()
                .test_runtime()
                .inspect(|state| state.registered(&target.to_string())));
            assert!(!usecase.test_presenter().unwrap().test_runtime().inspect(
                |state| state.registered(&SubscriptionTarget::RepositoryPaths.to_string())
            ));
            assert_eq!(usecase.test_worker_count(), 0);
        }
    }

    mod terminal_scenarios {
        use crate::usecase::state_subscription::*;
        use crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem;
        use futures_util::StreamExt;
        use std::sync::Arc;

        use crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::TerminalSurfaceRuntimeGatewayFor;
        use crate::adaptor::presenter::terminal_event_hub::TerminalSurfaceEventHub;
        use crate::domain::terminal_surface::entities::TerminalSurface;
        use crate::domain::terminal_surface::gateway::TerminalSurfaceGateway;
        use crate::infrastructure::state_subscription::Version;
        use crate::test_support::state_subscription::{
            terminal_item, Delivery, Event, StateSubscriptionEvent,
        };
        use crate::usecase::state_subscription::SubscriptionTarget;
        use crate::usecase::terminal_surface::output::TerminalSurfaceEventSink;
        use crate::usecase::terminal_surface::output::TerminalSurfaceOutputControl;
        use crate::usecase::terminal_surface::output::TerminalSurfaceOutputEvent;
        use crate::usecase::terminal_surface::output::TerminalSurfaceStateSink;

        use crate::domain::terminal_surface::TerminalSurfaceOwner;
        use crate::domain::workspace_tree::WorkspaceIdentity;

        struct BlockingResetOutput {
            hub: Arc<TerminalSurfaceEventHub>,
            started: std::sync::mpsc::Sender<()>,
            release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
        }

        impl TerminalSurfaceOutputControl for BlockingResetOutput {
            fn set_state_sink(
                &self,
                sink: Arc<dyn TerminalSurfaceStateSink>,
            ) -> Result<(), crate::usecase::terminal_surface::error::UsecaseError> {
                self.hub.set_state_sink(sink)
            }

            fn initialize(
                &self,
                registration: crate::usecase::terminal_surface::output::TerminalRegistration,
            ) -> Result<(), crate::usecase::terminal_surface::error::UsecaseError> {
                self.hub.initialize(registration)
            }

            fn subscribe_output(&self, session_key: &str, client: &str, units: usize) {
                if client == "stopping" && units == 0 {
                    self.started.send(()).unwrap();
                    self.release
                        .lock()
                        .unwrap()
                        .recv_timeout(std::time::Duration::from_secs(5))
                        .unwrap();
                }
                self.hub.subscribe_output(session_key, client, units);
            }

            fn unsubscribe_output(&self, session_key: &str, client: &str) {
                self.hub.unsubscribe_output(session_key, client);
            }

            fn processed_output(&self, session_key: &str, client: &str, units: usize) {
                self.hub.processed_output(session_key, client, units);
            }
        }

        fn fixture() -> (
            crate::test_support::state_subscription::TerminalSubscriptions,
            Arc<TerminalSurfaceRuntimeGatewayFor>,
            Arc<TerminalSurfaceEventHub>,
            TerminalSurface,
        ) {
            let (terminal, gateway, hub, surface) =
                crate::test_support::state_subscription::terminal_application_fixture();
            let subscriptions = StateSubscriptionUsecase::new(
                vec!["/repo".into()],
                crate::test_support::state_subscription::read_driver(),
            );
            let subscriptions = subscriptions.with_terminal(terminal);
            (subscriptions, gateway, hub, surface)
        }

        #[tokio::test]
        async fn test_terminal購読_同じstreamでsnapshot差分と区切りを届ける() {
            // Given
            let (subscriptions, gateway, hub, mut surface) = fixture();
            let raw = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
            let stream = subscriptions.open("client".into()).unwrap();
            tokio::pin!(stream);
            assert!(matches!(
                stream.next().await,
                Some(StateSubscriptionEvent::Ready)
            ));
            let before = gateway.snapshot_materialization_count();
            // When
            subscriptions
                .deps()
                .start_subscription(
                    "client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(&raw).unwrap(),
                    "input",
                    None,
                )
                .await
                .unwrap();
            subscriptions
                .usecase
                .deps()
                .start_subscription(
                    "client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(
                        "repository-paths",
                    )
                    .unwrap(),
                    &format!("{}:{}", "client", "repository-paths"),
                    None,
                )
                .await
                .unwrap();
            // Then
            let mut terminal_snapshot = false;
            let mut paths_snapshot = false;
            let mut terminal_bookmark = false;
            let mut paths_bookmark = false;
            for _ in 0..4 {
                match tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                    .await
                    .unwrap()
                {
                    Some(StateSubscriptionEvent::Item(
                        target,
                        Event::Snapshot(Version { sequence: 0, .. }, value),
                    )) if target == "input" => {
                        terminal_snapshot = matches!(
                            terminal_item(&value),
                            crate::adaptor::presenter::client::terminal_event::Item::Snapshot(_)
                        );
                    }
                    Some(StateSubscriptionEvent::Item(target, Event::Snapshot(_, _)))
                        if target == "client:repository-paths" =>
                    {
                        paths_snapshot = true
                    }
                    Some(StateSubscriptionEvent::Item(target, Event::Bookmark(_)))
                        if target == "input" =>
                    {
                        terminal_bookmark = true
                    }
                    Some(StateSubscriptionEvent::Item(target, Event::Bookmark(_)))
                        if target == "client:repository-paths" =>
                    {
                        paths_bookmark = true
                    }
                    event => panic!("unexpected subscription event: {}", event.is_some()),
                }
            }
            assert!(terminal_snapshot && paths_snapshot && terminal_bookmark && paths_bookmark);
            assert_eq!(gateway.snapshot_materialization_count(), before + 1);
            for sequence in 1..=100 {
                surface
                    .record_output(surface.runtime_generation, std::time::Instant::now())
                    .unwrap();
                gateway.insert_surface(surface.clone());
                hub.initialize(crate::test_support::state_subscription::registration(
                    &surface.session_key,
                    "/repo",
                    None,
                    1,
                    sequence,
                ))
                .unwrap();
                hub.publish(TerminalSurfaceOutputEvent::Output {
                    session_key: surface.session_key.clone(),
                    data: "🙂".into(),
                    sequence,
                });
            }
            assert_eq!(gateway.snapshot_materialization_count(), before + 1);
            for sequence in 1..=100 {
                assert!(
                    matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Change(version, Delivery::Delta, value))) if version.sequence == sequence && matches!(terminal_item(&value), crate::adaptor::presenter::client::terminal_event::Item::Output(_)))
                );
            }
        }

        #[tokio::test]
        async fn test_terminal再開_履歴内ならsnapshotを作らず再起動後は作る() {
            let (subscriptions, gateway, hub, mut surface) = fixture();
            let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
            let stream = subscriptions.open("client".into()).unwrap();
            tokio::pin!(stream);
            stream.next().await;
            subscriptions
                .deps()
                .start_subscription(
                    "client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                        .unwrap(),
                    "input",
                    None,
                )
                .await
                .unwrap();
            let Some(StateSubscriptionEvent::Item(_, Event::Snapshot(version, _))) =
                stream.next().await
            else {
                panic!("snapshot");
            };
            stream.next().await;
            let before = gateway.snapshot_materialization_count();
            stop_terminal(
                &subscriptions,
                "client",
                &crate::usecase::state_subscription::SubscriptionTarget::parse(&target).unwrap(),
            )
            .unwrap();
            surface
                .record_output(surface.runtime_generation, std::time::Instant::now())
                .unwrap();
            gateway.insert_surface(surface.clone());
            hub.initialize(crate::test_support::state_subscription::registration(
                &surface.session_key,
                "/repo",
                None,
                1,
                1,
            ))
            .unwrap();
            hub.publish(TerminalSurfaceOutputEvent::Output {
                session_key: surface.session_key.clone(),
                data: "next".into(),
                sequence: 1,
            });
            subscriptions
                .deps()
                .start_subscription(
                    "client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                        .unwrap(),
                    "input-2",
                    Some((&version.epoch, version.sequence)),
                )
                .await
                .unwrap();
            assert_eq!(gateway.snapshot_materialization_count(), before);
            assert!(
                matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Change(v, Delivery::Delta, _))) if v.sequence == 1)
            );
            stream.next().await;
            stop_terminal(
                &subscriptions,
                "client",
                &crate::usecase::state_subscription::SubscriptionTarget::parse(&target).unwrap(),
            )
            .unwrap();
            let recreated = TerminalSurface::new(2, surface.owner.clone(), None);
            gateway.remove_surface(surface.runtime_generation.value());
            gateway.insert_surface(recreated.clone());
            hub.initialize(crate::test_support::state_subscription::registration(
                &recreated.session_key,
                "/repo",
                None,
                2,
                0,
            ))
            .unwrap();
            subscriptions
                .test_presenter()
                .unwrap()
                .initialize(&crate::test_support::state_subscription::registration(
                    &recreated.session_key,
                    "/repo",
                    None,
                    recreated.runtime_generation.value(),
                    recreated.latest_sequence(),
                ))
                .unwrap();
            subscriptions
                .deps()
                .start_subscription(
                    "client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                        .unwrap(),
                    "input-3",
                    Some((&version.epoch, version.sequence)),
                )
                .await
                .unwrap();
            assert!(
                matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(v, _))) if v.epoch != version.epoch)
            );
            assert_eq!(gateway.snapshot_materialization_count(), before + 1);
        }

        #[tokio::test]
        async fn test_terminal再開不能_配信開始直後の出力を現在snapshotで回復する() {
            // Given
            let (subscriptions, gateway, hub, mut surface) = fixture();
            let target = SubscriptionTarget::Terminal(surface.owner.clone());
            let mut stream = Box::pin(subscriptions.open("client".into()).unwrap());
            stream.next().await;
            let cursor = Some(("old-epoch", 0));
            subscriptions
                .deps()
                .start_subscription("client", &target, "input", cursor)
                .await
                .unwrap();
            assert!(!hub.test_subscribed(&surface.session_key, "client"));
            assert_eq!(
                hub.test_pending_amount(&surface.session_key, "client"),
                None
            );

            // When
            surface
                .record_output(surface.runtime_generation, std::time::Instant::now())
                .unwrap();
            gateway.insert_surface(surface.clone());
            hub.initialize(crate::test_support::state_subscription::registration(
                &surface.session_key,
                "/repo",
                None,
                1,
                1,
            ))
            .unwrap();
            hub.publish(TerminalSurfaceOutputEvent::Output {
                session_key: surface.session_key.clone(),
                data: "next".into(),
                sequence: 1,
            });
            assert_eq!(
                hub.test_pending_amount(&surface.session_key, "client"),
                None
            );
            // Then
            assert!(matches!(
                tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                    .await
                    .unwrap(),
                Some(StateSubscriptionEvent::Item(_, Event::Snapshot(version, _)))
                    if version.sequence == 1
            ));
            assert!(hub.test_subscribed(&surface.session_key, "client"));
            assert_eq!(
                hub.test_pending_amount(&surface.session_key, "client"),
                Some(0)
            );
            hub.publish(TerminalSurfaceOutputEvent::Output {
                session_key: surface.session_key.clone(),
                data: "x".repeat(100_001).into(),
                sequence: 2,
            });
            assert_eq!(
                hub.test_pending_amount(&surface.session_key, "client"),
                Some(100_001)
            );
        }

        #[tokio::test]
        async fn test_terminal購読_件数上限がなく停止と切断で流量を解放する() {
            let (subscriptions, gateway, hub, surface) = fixture();
            let stream = subscriptions.open("client".into()).unwrap();
            for index in 0..20 {
                let owner = TerminalSurfaceOwner::workspace(WorkspaceIdentity::new(format!(
                    "/repo-{index}"
                )))
                .unwrap();
                let surface = TerminalSurface::new(index + 2, owner.clone(), None);
                gateway.insert_surface(surface.clone());
                hub.initialize(crate::test_support::state_subscription::registration(
                    &surface.session_key,
                    &format!("/repo-{index}"),
                    None,
                    surface.runtime_generation.value(),
                    surface.latest_sequence(),
                ))
                .unwrap();
                subscriptions
                    .deps()
                    .start_subscription(
                        "client",
                        &crate::usecase::state_subscription::SubscriptionTarget::parse(
                            &SubscriptionTarget::Terminal(owner).to_string(),
                        )
                        .unwrap(),
                        &format!("input-{index}"),
                        None,
                    )
                    .await
                    .unwrap();
            }
            assert_eq!(
                subscriptions
                    .test_presenter()
                    .unwrap()
                    .test_runtime()
                    .inspect(|state| state.active_targets().len()),
                20
            );
            drop(stream);
            assert!(subscriptions
                .test_presenter()
                .unwrap()
                .test_runtime()
                .inspect(|state| state.active_targets().is_empty()));
            assert!(crate::test_support::state_subscription::terminal_processed(
                &subscriptions,
                "client",
                &SubscriptionTarget::Terminal(surface.owner).to_string(),
                5000
            )
            .is_err());
        }

        #[tokio::test]
        async fn test_terminal復元_同じ対象の全clientの流量停止を解放する() {
            let (subscriptions, _, hub, surface) = fixture();
            let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
            let mut first = Box::pin(subscriptions.open("first".into()).unwrap());
            let mut second = Box::pin(subscriptions.open("second".into()).unwrap());
            first.next().await;
            second.next().await;
            for client in ["first", "second"] {
                subscriptions
                    .deps()
                    .start_subscription(
                        client,
                        &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                            .unwrap(),
                        client,
                        None,
                    )
                    .await
                    .unwrap();
            }
            first.next().await;
            first.next().await;
            second.next().await;
            second.next().await;
            for client in ["first", "second"] {
                hub.subscribe_output(
                    &surface.session_key,
                    client,
                    crate::infrastructure::terminal::output_flow_control::OUTPUT_HIGH_WATERMARK + 1,
                );
            }
            subscriptions
                .test_presenter()
                .unwrap()
                .test_runtime()
                .update(|state| state.require_delta_snapshot(&target))
                .unwrap();

            assert!(matches!(
                tokio::time::timeout(std::time::Duration::from_secs(2), first.next())
                    .await
                    .unwrap(),
                Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, _)))
            ));
            let (sender, receiver) = std::sync::mpsc::channel();
            let hub_for_wait = hub.clone();
            let session_key = surface.session_key.clone();
            let waiter = std::thread::spawn(move || {
                hub_for_wait.wait_output(&session_key);
                sender.send(()).unwrap();
            });
            let resumed = receiver
                .recv_timeout(std::time::Duration::from_secs(1))
                .is_ok();
            hub.unsubscribe_output(&surface.session_key, "first");
            hub.unsubscribe_output(&surface.session_key, "second");
            waiter.join().unwrap();
            assert!(resumed);
        }

        #[tokio::test]
        async fn test_snapshot作成中_別terminalのsnapshotと出力とexecutorを止めない() {
            // Given
            use crate::usecase::terminal_surface::test_helpers_io::FakePtyGateway;
            let mut gateway = FakePtyGateway::new();
            let first = TerminalSurface::new(
                1,
                TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/first")).unwrap(),
                None,
            );
            let second = TerminalSurface::new(
                2,
                TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/second")).unwrap(),
                None,
            );
            gateway.additional_surfaces = vec![first.clone(), second.clone()];
            let gateway = Arc::new(gateway);
            let hub = Arc::new(TerminalSurfaceEventHub::new());
            hub.initialize(crate::test_support::state_subscription::registration(
                &first.session_key,
                "/first",
                None,
                1,
                0,
            ))
            .unwrap();
            hub.initialize(crate::test_support::state_subscription::registration(
                &second.session_key,
                "/second",
                None,
                2,
                0,
            ))
            .unwrap();
            let terminal = Arc::new(
        crate::usecase::terminal_surface::application::TerminalSurfaceApplication::new(
            std::sync::Arc::new(crate::adaptor::gateway::telemetry::TelemetryGateway),
            gateway.clone(),
            Arc::new(crate::adaptor::gateway::terminal_surface::event_source::TerminalSurfaceEventSourceGateway::new(hub.event_sender())),
            hub.clone(),
        ),
    );
            let subscriptions = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            );
            let subscriptions = subscriptions.with_terminal(terminal.clone());
            let mut first_stream = Box::pin(subscriptions.open("first-client".into()).unwrap());
            let mut second_stream = Box::pin(subscriptions.open("second-client".into()).unwrap());
            first_stream.next().await;
            second_stream.next().await;
            let (started, waiting) = std::sync::mpsc::channel();
            let (release, blocked) = std::sync::mpsc::channel();
            *gateway.snapshot_gate.lock() = Some((started, blocked));
            // When
            subscriptions
                .deps()
                .start_subscription(
                    "first-client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(
                        &SubscriptionTarget::Terminal(first.owner).to_string(),
                    )
                    .unwrap(),
                    "first-input",
                    None,
                )
                .await
                .unwrap();
            let request = tokio::spawn(async move { first_stream.next().await });
            tokio::task::spawn_blocking(move || {
                waiting
                    .recv_timeout(std::time::Duration::from_secs(1))
                    .unwrap()
            })
            .await
            .unwrap();
            assert!(gateway.snapshot_gate.try_lock().is_some());
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                subscriptions.deps().start_subscription(
                    "second-client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(
                        &SubscriptionTarget::Terminal(second.owner.clone()).to_string(),
                    )
                    .unwrap(),
                    "second-input",
                    None,
                ),
            )
            .await;
            // Then
            assert!(!request.is_finished());
            assert!(result.unwrap().is_ok());
            assert_eq!(
                subscriptions
                    .test_presenter()
                    .unwrap()
                    .test_runtime()
                    .inspect(|state| state.snapshot_requests("second-client")),
                vec![SubscriptionTarget::Terminal(second.owner.clone()).to_string()]
            );
            subscriptions.schedule_terminal_refresh(
                vec!["second-client".into()],
                SubscriptionTarget::Terminal(second.owner.clone()),
            );
            assert_eq!(subscriptions.test_worker_count(), 2);
            tokio::time::timeout(std::time::Duration::from_secs(1), second_stream.next())
                .await
                .expect("second snapshot");
            tokio::time::timeout(std::time::Duration::from_secs(1), second_stream.next())
                .await
                .expect("second bookmark");
            terminal.resize(&second.owner, 40, 120).unwrap();
            terminal
                .write_attached(&second.owner, "second-input", 0, "input")
                .unwrap();
            hub.publish(TerminalSurfaceOutputEvent::Output {
                session_key: second.session_key,
                data: "live".into(),
                sequence: 1,
            });
            assert!(matches!(
                tokio::time::timeout(std::time::Duration::from_secs(1), second_stream.next())
                    .await
                    .expect("second output"),
                Some(StateSubscriptionEvent::Item(
                    _,
                    Event::Change(_, Delivery::Delta, _)
                ))
            ));
            release.send(()).unwrap();
            assert!(matches!(
                request.await.unwrap(),
                Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, _)))
            ));
        }

        #[tokio::test]
        async fn test_snapshot作成中_同じterminalへ追加されたclientにもsnapshotを届ける() {
            // Given
            use crate::usecase::terminal_surface::test_helpers_io::FakePtyGateway;
            let mut gateway = FakePtyGateway::new();
            let surface = TerminalSurface::new(
                1,
                TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap(),
                None,
            );
            gateway.additional_surfaces = vec![surface.clone()];
            let gateway = Arc::new(gateway);
            let hub = Arc::new(TerminalSurfaceEventHub::new());
            hub.initialize(crate::test_support::state_subscription::registration(
                &surface.session_key,
                "/repo",
                None,
                1,
                0,
            ))
            .unwrap();
            let terminal = Arc::new(
        crate::usecase::terminal_surface::application::TerminalSurfaceApplication::new(
            Arc::new(crate::adaptor::gateway::telemetry::TelemetryGateway),
            gateway.clone(),
            Arc::new(crate::adaptor::gateway::terminal_surface::event_source::TerminalSurfaceEventSourceGateway::new(hub.event_sender())),
            hub,
        ),
    );
            let subscriptions = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            );
            let subscriptions = subscriptions.with_terminal(terminal);
            let target = SubscriptionTarget::Terminal(surface.owner);
            let mut first = Box::pin(subscriptions.open("first".into()).unwrap());
            let mut second = Box::pin(subscriptions.open("second".into()).unwrap());
            first.next().await;
            second.next().await;
            let (started, waiting) = std::sync::mpsc::channel();
            let (release, blocked) = std::sync::mpsc::channel();
            *gateway.snapshot_gate.lock() = Some((started, blocked));

            // When
            subscriptions
                .deps()
                .start_subscription("first", &target, "first-input", None)
                .await
                .unwrap();
            let first_event = tokio::spawn(async move { first.next().await });
            tokio::task::spawn_blocking(move || {
                waiting
                    .recv_timeout(std::time::Duration::from_secs(1))
                    .unwrap()
            })
            .await
            .unwrap();
            subscriptions
                .deps()
                .start_subscription("second", &target, "second-input", None)
                .await
                .unwrap();
            subscriptions.schedule_terminal_refresh(vec!["second".into()], target.clone());
            assert_eq!(subscriptions.test_worker_count(), 1);
            release.send(()).unwrap();

            // Then
            assert!(matches!(
                first_event.await.unwrap(),
                Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, _)))
            ));
            assert!(matches!(
                tokio::time::timeout(std::time::Duration::from_secs(1), second.next())
                    .await
                    .unwrap(),
                Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, _)))
            ));
        }

        #[tokio::test]
        async fn test_terminal復元_停止と切断の競合でも停止済みclientを再登録しない() {
            use crate::usecase::terminal_surface::test_helpers_io::FakePtyGateway;

            for close in [false, true] {
                // Given
                let surface = TerminalSurface::new(
                    1,
                    TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap(),
                    None,
                );
                let mut gateway = FakePtyGateway::new();
                gateway.additional_surfaces.push(surface.clone());
                let hub = Arc::new(TerminalSurfaceEventHub::with_flags(8, true));
                hub.initialize(crate::test_support::state_subscription::registration(
                    &surface.session_key,
                    "/repo",
                    None,
                    1,
                    0,
                ))
                .unwrap();
                let (started, waiting) = std::sync::mpsc::channel();
                let (release, blocked) = std::sync::mpsc::channel();
                let output = Arc::new(BlockingResetOutput {
                    hub: hub.clone(),
                    started,
                    release: std::sync::Mutex::new(blocked),
                });
                let terminal = Arc::new(crate::usecase::terminal_surface::application::TerminalSurfaceApplication::new(
            Arc::new(crate::adaptor::gateway::telemetry::TelemetryGateway),
            Arc::new(gateway),
            Arc::new(crate::adaptor::gateway::terminal_surface::event_source::TerminalSurfaceEventSourceGateway::new(hub.event_sender())),
            output,
        ));
                let subscriptions = StateSubscriptionUsecase::new(
                    vec![],
                    crate::test_support::state_subscription::read_driver(),
                );
                let subscriptions = subscriptions.with_terminal(terminal);
                let target = SubscriptionTarget::Terminal(surface.owner.clone());
                let _stopping = subscriptions.open("stopping".into()).unwrap();
                let _active = subscriptions.open("active".into()).unwrap();
                subscriptions
                    .deps()
                    .start_subscription("stopping", &target, "stopping-input", None)
                    .await
                    .unwrap();
                subscriptions
                    .deps()
                    .start_subscription("active", &target, "active-input", None)
                    .await
                    .unwrap();
                hub.subscribe_output(
                    &surface.session_key,
                    "active",
                    crate::infrastructure::terminal::output_flow_control::OUTPUT_HIGH_WATERMARK + 1,
                );

                // When
                subscriptions.schedule_terminal_refresh(
                    vec!["stopping".into(), "active".into()],
                    target.clone(),
                );
                tokio::task::spawn_blocking(move || {
                    waiting
                        .recv_timeout(std::time::Duration::from_secs(2))
                        .unwrap()
                })
                .await
                .unwrap();
                let stopping = subscriptions.clone();
                let stopped_target = target.clone();
                let (stop_started, started_stop) = tokio::sync::oneshot::channel();
                let mut stop = tokio::task::spawn_blocking(move || {
                    let _ = stop_started.send(());
                    if close {
                        stopping.close_client("stopping");
                    } else {
                        stop_terminal(&stopping, "stopping", &stopped_target).unwrap();
                    }
                });
                started_stop.await.unwrap();
                assert!(
                    tokio::time::timeout(std::time::Duration::from_millis(100), &mut stop)
                        .await
                        .is_err()
                );
                release.send(()).unwrap();
                stop.await.unwrap();
                tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    while subscriptions.test_worker_count() != 0 {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();

                // Then
                assert!(!hub.test_subscribed(&surface.session_key, "stopping"));
                assert!(hub.test_subscribed(&surface.session_key, "active"));
                let units =
                    crate::infrastructure::terminal::output_flow_control::OUTPUT_HIGH_WATERMARK + 1;
                hub.publish(TerminalSurfaceOutputEvent::Output {
                    session_key: surface.session_key.clone(),
                    data: "x".repeat(units).into(),
                    sequence: 1,
                });
                hub.processed_output(&surface.session_key, "active", units);
                let (resumed, wait) = std::sync::mpsc::channel();
                let session_key = surface.session_key.clone();
                let waiting_hub = hub.clone();
                let waiter = std::thread::spawn(move || {
                    waiting_hub.wait_output(&session_key);
                    resumed.send(()).unwrap();
                });
                let active_resumed = wait.recv_timeout(std::time::Duration::from_secs(2)).is_ok();
                hub.unsubscribe_output(&surface.session_key, "active");
                hub.unsubscribe_output(&surface.session_key, "stopping");
                waiter.join().unwrap();
                assert!(active_resumed);
            }
        }

        #[tokio::test]
        async fn test_terminal差分_出力の重複を除き同じ出力番号で寸法と終了を配信する() {
            // Given
            let (subscriptions, _, hub, surface) = fixture();
            let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
            let stream = subscriptions.open("client".into()).unwrap();
            tokio::pin!(stream);
            stream.next().await;
            subscriptions
                .deps()
                .start_subscription(
                    "client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                        .unwrap(),
                    "input",
                    None,
                )
                .await
                .unwrap();
            stream.next().await;
            stream.next().await;
            // When
            let output = TerminalSurfaceOutputEvent::Output {
                session_key: surface.session_key.clone(),
                data: "x".into(),
                sequence: 1,
            };
            hub.publish(output.clone());
            hub.publish(output);
            hub.publish(TerminalSurfaceOutputEvent::Resize {
                session_key: surface.session_key.clone(),
                cols: 120,
                rows: 30,
                sequence: 1,
            });
            hub.publish(TerminalSurfaceOutputEvent::Exit {
                session_key: surface.session_key.clone(),
                runtime_generation: 1,
                exit_code: Some(7),
                sequence: 1,
            });
            let mut actual = Vec::new();
            for _ in 0..3 {
                actual.push(stream.next().await.unwrap());
            }
            // Then
            for ((sequence, expected), event) in [
                (
                    1,
                    TerminalSurfaceStreamItem::Output {
                        session_key: surface.session_key.clone(),
                        data: "x".into(),
                        sequence: 1,
                    },
                ),
                (
                    1,
                    TerminalSurfaceStreamItem::Resize {
                        session_key: surface.session_key.clone(),
                        cols: 120,
                        rows: 30,
                        sequence: 1,
                    },
                ),
                (
                    1,
                    TerminalSurfaceStreamItem::Exit {
                        session_key: surface.session_key.clone(),
                        exit_code: Some(7),
                        sequence: 1,
                    },
                ),
            ]
            .into_iter()
            .zip(actual)
            {
                let StateSubscriptionEvent::Item(
                    actual_target,
                    Event::Change(version, Delivery::Delta, value),
                ) = event
                else {
                    panic!("terminal delta");
                };
                let expected_payload = crate::adaptor::presenter::state_subscription_wire::payload(
                    &StateValue::Terminal(expected.into()),
                )
                .unwrap();
                assert_eq!(actual_target, "input");
                assert_eq!(version.sequence, sequence);
                assert_eq!(*value, expected_payload.into());
            }
        }

        #[tokio::test]
        async fn test_terminal購読_処理報告単位以外を拒否する() {
            // Given
            let (subscriptions, _, _, surface) = fixture();
            let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
            let stream = subscriptions.open("client".into()).unwrap();
            tokio::pin!(stream);
            stream.next().await;
            subscriptions
                .deps()
                .start_subscription(
                    "client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                        .unwrap(),
                    "input",
                    None,
                )
                .await
                .unwrap();
            stream.next().await;
            stream.next().await;
            // When
            let result = crate::test_support::state_subscription::terminal_processed(
                &subscriptions,
                "client",
                &target,
                1,
            );
            // Then
            assert!(result.is_err());
        }
        #[tokio::test]
        async fn test_terminal購読_処理報告単位を受理する() {
            // Given
            let (subscriptions, _, _, surface) = fixture();
            let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
            let stream = subscriptions.open("client".into()).unwrap();
            tokio::pin!(stream);
            stream.next().await;
            subscriptions
                .deps()
                .start_subscription(
                    "client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                        .unwrap(),
                    "input",
                    None,
                )
                .await
                .unwrap();
            stream.next().await;
            stream.next().await;
            // When
            let result = crate::test_support::state_subscription::terminal_processed(
                &subscriptions,
                "client",
                &target,
                5000,
            );
            // Then
            assert!(result.is_ok());
        }
        #[tokio::test]
        async fn test_terminal購読_停止後は処理報告を拒否する() {
            // Given
            let (subscriptions, _, _, surface) = fixture();
            let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
            let stream = subscriptions.open("client".into()).unwrap();
            tokio::pin!(stream);
            stream.next().await;
            subscriptions
                .deps()
                .start_subscription(
                    "client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                        .unwrap(),
                    "input",
                    None,
                )
                .await
                .unwrap();
            stream.next().await;
            stream.next().await;
            stop_terminal(
                &subscriptions,
                "client",
                &crate::usecase::state_subscription::SubscriptionTarget::parse(&target).unwrap(),
            )
            .unwrap();
            // When
            let result = crate::test_support::state_subscription::terminal_processed(
                &subscriptions,
                "client",
                &target,
                5000,
            );
            // Then
            assert!(result.is_err());
        }
        #[tokio::test]
        async fn test_terminal購読_出力前の寸法変更と終了を版ゼロで届け再開する() {
            // Given
            let (subscriptions, _, hub, surface) = fixture();
            let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
            let stream = subscriptions.open("client".into()).unwrap();
            tokio::pin!(stream);
            stream.next().await;
            subscriptions
                .deps()
                .start_subscription(
                    "client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                        .unwrap(),
                    "input",
                    None,
                )
                .await
                .unwrap();
            let Some(StateSubscriptionEvent::Item(_, Event::Snapshot(version, _))) =
                stream.next().await
            else {
                panic!("snapshot");
            };
            assert_eq!(version.sequence, 0);
            stream.next().await;
            // When
            hub.publish(TerminalSurfaceOutputEvent::Resize {
                session_key: surface.session_key.clone(),
                cols: 120,
                rows: 30,
                sequence: 0,
            });
            hub.publish(TerminalSurfaceOutputEvent::Exit {
                session_key: surface.session_key.clone(),
                runtime_generation: 1,
                exit_code: Some(7),
                sequence: 0,
            });
            // Then
            for reconnect in [false, true] {
                if reconnect {
                    stop_terminal(
                        &subscriptions,
                        "client",
                        &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                            .unwrap(),
                    )
                    .unwrap();
                    subscriptions
                        .deps()
                        .start_subscription(
                            "client",
                            &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                                .unwrap(),
                            "input",
                            Some((&version.epoch, version.sequence)),
                        )
                        .await
                        .unwrap();
                }
                let Some(StateSubscriptionEvent::Item(
                    _,
                    Event::Change(resize_version, Delivery::Delta, resize),
                )) = stream.next().await
                else {
                    panic!("resize");
                };
                assert_eq!(resize_version, version);
                assert!(matches!(
                    terminal_item(&resize),
                    crate::adaptor::presenter::client::terminal_event::Item::Resize(
                        crate::adaptor::presenter::client::TerminalResize {
                            cols: 120,
                            rows: 30,
                            sequence: 0,
                            ..
                        }
                    )
                ));
                let Some(StateSubscriptionEvent::Item(
                    _,
                    Event::Change(exit_version, Delivery::Delta, exit),
                )) = stream.next().await
                else {
                    panic!("exit");
                };
                assert_eq!(exit_version, version);
                assert!(matches!(
                    terminal_item(&exit),
                    crate::adaptor::presenter::client::terminal_event::Item::Exit(
                        crate::adaptor::presenter::client::TerminalExit {
                            exit_code: Some(7),
                            sequence: 0,
                            ..
                        }
                    )
                ));
            }
        }

        #[tokio::test]
        async fn test_terminal購読_出力と寸法の逆転は古い寸法を捨てずsnapshotで復元する() {
            // Given
            let (subscriptions, gateway, hub, mut surface) = fixture();
            let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
            let stream = subscriptions.open("client".into()).unwrap();
            tokio::pin!(stream);
            stream.next().await;
            subscriptions
                .deps()
                .start_subscription(
                    "client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                        .unwrap(),
                    "input",
                    None,
                )
                .await
                .unwrap();
            stream.next().await;
            stream.next().await;
            surface
                .record_output(surface.runtime_generation, std::time::Instant::now())
                .unwrap();
            surface.checkpoint.cols = 120;
            surface.checkpoint.rows = 30;
            gateway.insert_surface(surface.clone());
            hub.initialize(crate::test_support::state_subscription::registration(
                &surface.session_key,
                "/repo",
                None,
                1,
                1,
            ))
            .unwrap();
            // When
            hub.publish(TerminalSurfaceOutputEvent::Output {
                session_key: surface.session_key.clone(),
                sequence: 1,
                data: "x".into(),
            });
            hub.publish(TerminalSurfaceOutputEvent::Resize {
                session_key: surface.session_key.clone(),
                sequence: 0,
                cols: 120,
                rows: 30,
            });
            // Then
            let Some(StateSubscriptionEvent::Item(_, Event::Snapshot(version, snapshot))) =
                stream.next().await
            else {
                panic!("resynchronized snapshot");
            };
            assert_eq!(version.sequence, 1);
            assert!(
                matches!(terminal_item(&snapshot), crate::adaptor::presenter::client::terminal_event::Item::Snapshot(value) if value.cols == 120 && value.rows == 30)
            );
        }

        #[tokio::test]
        async fn test_terminal削除_経路と履歴を解放し購読と入力は明示停止まで保つ() {
            // Given
            let (subscriptions, gateway, hub, surface) = fixture();
            let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
            let stream = subscriptions.open("client".into()).unwrap();
            tokio::pin!(stream);
            stream.next().await;
            // When
            for generation in 1..=10 {
                let surface = TerminalSurface::new(generation, surface.owner.clone(), None);
                gateway.insert_surface(surface.clone());
                hub.initialize(crate::test_support::state_subscription::registration(
                    &surface.session_key,
                    "/repo",
                    None,
                    generation,
                    0,
                ))
                .unwrap();
                subscriptions
                    .deps()
                    .start_subscription(
                        "client",
                        &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                            .unwrap(),
                        "input",
                        None,
                    )
                    .await
                    .unwrap();
                stream.next().await;
                stream.next().await;
                hub.publish(TerminalSurfaceOutputEvent::Output {
                    session_key: surface.session_key.clone(),
                    data: "retained".into(),
                    sequence: 1,
                });
                hub.publish(TerminalSurfaceOutputEvent::Exit {
                    session_key: surface.session_key.clone(),
                    runtime_generation: generation,
                    exit_code: Some(0),
                    sequence: 1,
                });
                gateway.remove_surface(generation).unwrap();
                // Then
                assert!(
                    subscriptions
                        .test_presenter()
                        .unwrap()
                        .test_runtime()
                        .inspect(|state| usize::from(state.registered(&target)))
                        == 0
                );
                let runtime = &subscriptions.test_presenter().unwrap().test_runtime();
                assert_eq!(runtime.inspect(|state| state.active_targets().len()), 1);
                assert!(runtime.inspect(|state| state.current_version(&target).is_none()));
                runtime.mutate(|state| (state.bookmark("client"), true));
                assert!(runtime.inspect(|state| state.snapshot_requests("client").is_empty()));
                assert!(
                    matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Change(_, _, value))) if matches!(terminal_item(&value), crate::adaptor::presenter::client::terminal_event::Item::Output(_)))
                );
                assert!(
                    matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Change(_, _, value))) if matches!(terminal_item(&value), crate::adaptor::presenter::client::terminal_event::Item::Exit(event) if event.exit_code == Some(0)))
                );
                assert!(subscriptions
                    .test_presenter()
                    .unwrap()
                    .test_runtime()
                    .inspect(|state| state
                        .lookup("input")
                        .is_some_and(|(client, raw)| client == "client" && raw == target)));
                assert_eq!(
                    subscriptions
                        .test_presenter()
                        .as_ref()
                        .unwrap()
                        .test_runtime()
                        .inspect(|state| state.active_targets().len()),
                    1
                );
                stop_terminal(
                    &subscriptions,
                    "client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                        .unwrap(),
                )
                .unwrap();
                assert!(subscriptions
                    .test_presenter()
                    .unwrap()
                    .test_runtime()
                    .inspect(|state| state.active_targets().is_empty()));
                assert!(subscriptions
                    .test_presenter()
                    .unwrap()
                    .test_runtime()
                    .inspect(|state| state.active_targets().is_empty()));
            }
        }

        #[tokio::test]
        async fn test_terminal購読開始_古いsummary取得後の再作成でepochを巻き戻さない() {
            // Given
            let (subscriptions, gateway, hub, surface) = fixture();
            let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
            let stream = subscriptions.open("client".into()).unwrap();
            tokio::pin!(stream);
            stream.next().await;
            let old_version = subscriptions
                .test_presenter()
                .unwrap()
                .test_runtime()
                .inspect(|state| state.current_version(&target))
                .unwrap();
            let recreated = TerminalSurface::new(2, surface.owner.clone(), None);
            let replacement = recreated.clone();
            let replacement_key = recreated.session_key.clone();
            let gateway_for_replacement = gateway.clone();
            let hub_for_replacement = hub.clone();
            *gateway.before_output_order.lock() = Some(Box::new(move || {
                gateway_for_replacement.remove_surface(1).unwrap();
                gateway_for_replacement.insert_surface(replacement);
                hub_for_replacement
                    .initialize(crate::test_support::state_subscription::registration(
                        &replacement_key,
                        "/repo",
                        None,
                        2,
                        0,
                    ))
                    .unwrap();
            }));
            // When
            subscriptions
                .deps()
                .start_subscription(
                    "client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                        .unwrap(),
                    "new-input",
                    Some((&old_version.epoch, old_version.sequence)),
                )
                .await
                .unwrap();
            // Then
            let Some(StateSubscriptionEvent::Item(_, Event::Snapshot(version, value))) =
                stream.next().await
            else {
                panic!("new runtime snapshot");
            };
            assert_ne!(version.epoch, old_version.epoch);
            assert_eq!(
                version,
                subscriptions
                    .test_presenter()
                    .unwrap()
                    .test_runtime()
                    .inspect(|state| state.current_version(&target))
                    .unwrap()
            );
            assert!(
                matches!(terminal_item(&value), crate::adaptor::presenter::client::terminal_event::Item::Snapshot(surface) if surface.session_key == recreated.session_key)
            );
            subscriptions.test_presenter().unwrap().remove(
                &crate::test_support::state_subscription::registration(
                    &surface.session_key,
                    "/repo",
                    None,
                    surface.runtime_generation.value(),
                    0,
                ),
            );
            assert_eq!(
                subscriptions
                    .test_presenter()
                    .unwrap()
                    .test_runtime()
                    .inspect(|state| state.current_version(&target)),
                Some(version)
            );
            assert_eq!(
                subscriptions
                    .test_presenter()
                    .unwrap()
                    .test_runtime()
                    .inspect(|state| usize::from(state.registered(&target))),
                1
            );
        }

        #[tokio::test]
        async fn test_terminal購読開始_出力順序区間内でsummary取得に失敗したら開始しない() {
            // Given
            let (subscriptions, gateway, hub, surface) = fixture();
            let target = SubscriptionTarget::Terminal(surface.owner.clone());
            let session_key = surface.session_key.clone();
            let _stream = subscriptions.open("client".into()).unwrap();
            let removed = gateway.clone();
            *gateway.during_output_order.lock() = Some(Box::new(move || {
                removed
                    .remove_surface(surface.runtime_generation.value())
                    .unwrap();
            }));

            // When
            let result = subscriptions
                .deps()
                .start_subscription("client", &target, "input", None)
                .await;

            // Then
            assert!(result.is_err());
            assert!(subscriptions
                .terminal
                .test_input_id("client", &target)
                .is_none());
            assert!(matches!(
                subscriptions
                    .terminal
                    .terminal_processed("input", 5000)
                    .unwrap_err()
                    .source,
                crate::usecase::state_subscription::StateReadFailure::TerminalSubscriptionEnded
            ));
            assert!(!hub.test_subscribed(&session_key, "client"));
            assert!(!subscriptions
                .test_presenter()
                .unwrap()
                .test_runtime()
                .inspect(|state| state
                    .lookup("input")
                    .is_some_and(|(client, raw)| client == "client" && raw == target.to_string())));
        }

        #[tokio::test]
        async fn test_terminal購読開始_途中でclientが切断したら出力登録を巻き戻す() {
            // Given
            let (subscriptions, gateway, hub, surface) = fixture();
            let target = SubscriptionTarget::Terminal(surface.owner.clone());
            let _stream = subscriptions.open("client".into()).unwrap();
            let disconnected = subscriptions.clone();
            *gateway.before_output_order.lock() =
                Some(Box::new(move || disconnected.close_client("client")));
            // When
            let result = subscriptions
                .deps()
                .start_subscription("client", &target, "input", None)
                .await;
            // Then
            assert!(
                matches!(result, Err(crate::usecase::state_subscription::StateReadError {
        source: crate::usecase::state_subscription::StateReadFailure::Subscription(error), ..
    }) if *error == crate::usecase::state_subscription::SubscriptionError::StreamEnded)
            );
            assert!(!hub.test_subscribed(&surface.session_key, "client"));
            assert!(subscriptions
                .terminal
                .test_input_id("client", &target)
                .is_none());
            assert!(matches!(
                subscriptions
                    .terminal
                    .terminal_processed("input", 5000)
                    .unwrap_err()
                    .source,
                crate::usecase::state_subscription::StateReadFailure::TerminalSubscriptionEnded
            ));
            assert!(!subscriptions
                .test_presenter()
                .unwrap()
                .test_runtime()
                .inspect(|state| state
                    .lookup("input")
                    .is_some_and(|(client, raw)| client == "client" && raw == target.to_string())));
        }

        #[tokio::test]
        async fn test_terminal再取得失敗_開始済み購読へ失敗を届ける() {
            // Given
            use crate::usecase::terminal_surface::test_helpers_io::FakePtyGateway;
            let (subscriptions, _, hub, surface) = fixture();
            let mut gateway = FakePtyGateway::new();
            gateway.surface = Some(surface.clone());
            let gateway = Arc::new(gateway);
            let terminal = Arc::new(crate::usecase::terminal_surface::application::TerminalSurfaceApplication::new(
        Arc::new(crate::adaptor::gateway::telemetry::TelemetryGateway),
        gateway.clone(),
        Arc::new(crate::adaptor::gateway::terminal_surface::event_source::TerminalSurfaceEventSourceGateway::new(hub.event_sender())),
        hub,
    ));
            let subscriptions = subscriptions.with_terminal(terminal);
            let target = SubscriptionTarget::Terminal(surface.owner.clone());
            let stream = subscriptions.open("client".into()).unwrap();
            tokio::pin!(stream);
            assert!(matches!(
                stream.next().await,
                Some(StateSubscriptionEvent::Ready)
            ));
            subscriptions
                .deps()
                .start_subscription(
                    "client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(
                        &target.to_string(),
                    )
                    .unwrap(),
                    "input",
                    None,
                )
                .await
                .unwrap();
            stream.next().await.unwrap();
            stream.next().await.unwrap();
            *gateway.snapshot_unavailable.lock() = true;
            // When
            subscriptions.schedule_terminal_refresh(vec!["client".into()], target.clone());
            let event = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                .await
                .unwrap()
                .unwrap();
            // Then
            assert!(
                matches!(event, StateSubscriptionEvent::Item(_, Event::Change(_, _, value)) if matches!(value.as_ref(), crate::adaptor::presenter::state_subscription::PublishedState::Failure(failure) if failure.message.contains("snapshot unavailable")))
            );
        }

        #[tokio::test]
        async fn test_terminal購読開始_捨てられた最初の状態を再要求から作り直して届ける() {
            // Given
            let (subscriptions, _, _, surface) = fixture();
            let target = SubscriptionTarget::Terminal(surface.owner.clone());
            let mut first = Box::pin(subscriptions.open("first".into()).unwrap());
            first.next().await.unwrap();
            subscriptions
                .deps()
                .start_subscription("first", &target, "first-input", None)
                .await
                .unwrap();
            let snapshot = tokio::time::timeout(std::time::Duration::from_secs(2), first.next())
                .await
                .unwrap()
                .unwrap();
            assert!(matches!(
                snapshot,
                StateSubscriptionEvent::Item(_, Event::Snapshot(_, _))
            ));
            subscriptions
                .terminal
                .stop_delivery(
                    "first",
                    &target,
                    "first-input",
                    &subscriptions
                        .usecase
                        .test_presenter()
                        .unwrap()
                        .delivery("first-input")
                        .unwrap()
                        .2,
                )
                .unwrap();
            let runtime = subscriptions.test_presenter().unwrap().test_runtime();
            let raw = target.to_string();
            assert!(runtime.inspect(|state| state.current_version(&raw).is_some()));
            assert!(runtime.inspect(|state| state.needs_snapshot(&raw, None).unwrap()));
            let mut second = Box::pin(subscriptions.open("second".into()).unwrap());
            second.next().await.unwrap();
            // When
            subscriptions
                .deps()
                .start_subscription("second", &target, "second-input", None)
                .await
                .unwrap();
            assert_eq!(
                runtime.inspect(|state| state.snapshot_requests("second")),
                vec![raw.clone()]
            );
            let event = tokio::time::timeout(std::time::Duration::from_secs(2), second.next())
                .await
                .unwrap()
                .unwrap();
            // Then
            assert!(
                matches!(event, StateSubscriptionEvent::Item(delivered, Event::Snapshot(_, value))
        if delivered == "second-input" && matches!(terminal_item(&value),
            crate::adaptor::presenter::client::terminal_event::Item::Snapshot(snapshot) if snapshot.session_key == surface.session_key))
            );
        }

        fn stop_terminal(
            subscriptions: &crate::test_support::state_subscription::TerminalSubscriptions,
            client: &str,
            target: &SubscriptionTarget,
        ) -> Result<(), crate::usecase::state_subscription::SubscriptionError> {
            let Some(id) = subscriptions.terminal.test_input_id(client, target) else {
                return Ok(());
            };
            let Some((_, _, delivery)) = subscriptions
                .usecase
                .test_presenter()
                .unwrap()
                .delivery(&id)
            else {
                return Ok(());
            };
            subscriptions
                .terminal
                .stop_delivery(client, target, &id, &delivery)
        }
    }
}
