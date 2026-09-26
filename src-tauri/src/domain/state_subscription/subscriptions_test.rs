use super::*;

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
        let id = SubscriptionTarget::Branches(format!("/repo/{n}"), None).to_string();
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
    state.publish("workspaces", 64, None).unwrap();
    assert!(state.next("client").is_none());
}

#[test]
fn test_監視_共有する購読が全て終了したときだけ不要になる() {
    // Given
    let mut state = registry();
    state.open("other".into()).unwrap();
    let paths = vec!["/repo".into()];
    // When
    state.start("client", "workspaces", None).unwrap();
    state.start("other", "workspaces", None).unwrap();
    state.stop("client", "workspaces").unwrap();
    // Then
    assert_eq!(
        state.required_watches(&paths, &[]),
        [WatchRequirement::Git("/repo".into())].into()
    );
    state.close("other");
    assert!(state.required_watches(&paths, &[]).is_empty());
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
    let target = SubscriptionTarget::Branches("/repo".into(), None);
    // When
    assert_eq!(
        state.start_with_snapshot("closed", &target.to_string(), 1, None),
        Err(SubscriptionError::StreamEnded)
    );
    // Then
    assert!(!state.registered(&target));
    assert!(state.registered(&SubscriptionTarget::RepositoryPaths));
    state
        .start_with_snapshot("client", &target.to_string(), 1, None)
        .unwrap();
    assert!(state.registered(&target));
}

#[test]
fn test_branch一覧購読_repository変更を選び監視し変更値を配信する() {
    // Given
    let mut state = Subscriptions::new("boot".into());
    state.open("client".into()).unwrap();
    let target = SubscriptionTarget::Branches("/repo".into(), Some("feature".into()));
    let raw = target.to_string();
    state
        .start_with_snapshot("client", &raw, vec!["main".to_string()], None)
        .unwrap();
    assert!(
        matches!(state.next("client"), Some((_, Event::Snapshot(_, value))) if *value == ["main"])
    );
    state.next("client");
    // When
    let source = StateChangeSource::Repository(vec!["/repo".into()]);
    let selected: Vec<_> = state
        .active_targets()
        .into_iter()
        .filter(|target| target.affected_by(&source))
        .collect();
    // Then
    assert_eq!(selected, vec![target.clone()]);
    assert!(!target.affected_by(&StateChangeSource::Repository(vec!["/other".into()])));
    assert_eq!(
        state.required_watches(&[], &[]),
        [WatchRequirement::Git("/repo".into())].into()
    );
    state
        .publish(&raw, vec!["main".to_string(), "develop".to_string()], None)
        .unwrap();
    assert!(
        matches!(state.next("client"), Some((id, Event::Change(_, Delivery::Full, value))) if id == raw && *value == ["main", "develop"])
    );
    state.stop("client", &raw).unwrap();
    assert!(state.required_watches(&[], &[]).is_empty());
}

#[test]
fn test_repository購読_path一致の各対象だけに変更値を配信する() {
    for target in [
        SubscriptionTarget::BranchBase("/repo".into(), "feature".into()),
        SubscriptionTarget::BranchStatus("/repo".into()),
        SubscriptionTarget::CurrentBranch("/repo".into()),
        SubscriptionTarget::Worktrees("/repo".into()),
        SubscriptionTarget::RepositoryRoot("/repo".into()),
    ] {
        // Given
        let mut state = Subscriptions::new("boot".into());
        state.open("client".into()).unwrap();
        let raw = target.to_string();
        state
            .start_with_snapshot("client", &raw, "before", None)
            .unwrap();
        assert!(
            matches!(state.next("client"), Some((id, Event::Snapshot(_, value))) if id == raw && *value == "before")
        );
        assert!(matches!(
            state.next("client"),
            Some((_, Event::Bookmark(_)))
        ));
        // When / Then
        for path in ["/other", "/repo"] {
            let source = StateChangeSource::Repository(vec![path.into()]);
            let selected: Vec<_> = state
                .active_targets()
                .into_iter()
                .filter(|candidate| candidate.affected_by(&source))
                .collect();
            if path == "/other" {
                assert!(selected.is_empty(), "{target}");
            } else {
                assert_eq!(selected, vec![target.clone()]);
            }
            for candidate in selected {
                state
                    .publish(&candidate.to_string(), "after", None)
                    .unwrap();
            }
            if path == "/other" {
                assert!(state.next("client").is_none(), "{target}");
            } else {
                assert!(
                    matches!(state.next("client"), Some((id, Event::Change(version, Delivery::Full, value))) if id == raw && version.sequence == 1 && *value == "after"),
                    "{target}"
                );
            }
        }
    }
}

#[test]
fn test_購読開始確認_切断後は対象の鍵を解放し他の購読と起動時対象を保持する() {
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
        state.ensure_active(&SubscriptionTarget::Workspaces),
        Err(SubscriptionError::StreamEnded)
    );
    assert!(!state.registered(&SubscriptionTarget::Workspaces));
    assert_eq!(state.ensure_active(&SubscriptionTarget::Providers), Ok(()));
    assert_eq!(
        state.ensure_active(&SubscriptionTarget::RepositoryPaths),
        Err(SubscriptionError::StreamEnded)
    );
    assert!(state.registered(&SubscriptionTarget::RepositoryPaths));
}

#[test]
fn test_provider一覧購読_provider変更だけを選び同じ購読へ更新一覧を配信する() {
    // Given
    let mut state = Subscriptions::new("boot".into());
    state.open("client".into()).unwrap();
    state
        .start_with_snapshot("client", "providers", vec!["codex"], None)
        .unwrap();
    state
        .start_with_snapshot("client", "workspaces", vec![], None)
        .unwrap();
    for _ in 0..4 {
        state.next("client");
    }
    // When
    let selected: Vec<_> = state
        .active_targets()
        .into_iter()
        .filter(|target| target.affected_by(&StateChangeSource::Providers))
        .collect();
    // Then
    assert_eq!(selected, vec![SubscriptionTarget::Providers]);
    assert!(!SubscriptionTarget::Providers.affected_by(&StateChangeSource::ProviderHistory));
    for target in selected {
        state
            .publish(&target.to_string(), vec!["codex", "claude"], None)
            .unwrap();
    }
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
    assert_eq!(
        state.start_with_snapshot("closed", "workspaces", 1, None),
        Err(SubscriptionError::StreamEnded)
    );
    assert!(!state.registered(&SubscriptionTarget::Workspaces));
    state.open("next".into()).unwrap();
    state
        .start_with_snapshot("next", "workspaces", 2, Some(&version))
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
        state.ensure_active(&SubscriptionTarget::Workspaces),
        Err(SubscriptionError::VersionExhausted)
    );
    assert!(state.registered(&SubscriptionTarget::Workspaces));
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
    state.require_delta_snapshot(target).unwrap();
    // Then
    for client in ["client", "second"] {
        assert_eq!(state.snapshot_requests(client), vec![target]);
        assert!(state.next(client).is_none());
    }
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
        assert!(!state.registered(&SubscriptionTarget::parse(target).unwrap()));
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
