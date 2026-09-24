use super::*;

fn registry() -> Subscriptions<u64> {
    let mut state = Subscriptions::new("boot".into());
    state.register("a".into(), 0, Delivery::Full).unwrap();
    state.register("b".into(), 0, Delivery::Delta).unwrap();
    state.open("client".into()).unwrap();
    state
}

#[test]
fn test_購読_初期状態と区切りの後に変更が届く() {
    // Given
    let mut state = registry();
    // When
    state.start("client", "a", None).unwrap();
    state.publish("a", 1, None).unwrap();
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
    state.start("client", "a", None).unwrap();
    state.start("client", "a", None).unwrap();
    // Then
    assert_eq!(state.clients["client"].subscriptions.len(), 1);
    assert_eq!(
        state.start("client", "missing", None),
        Err(SubscriptionError::UnknownTarget)
    );
    state.stop("client", "a").unwrap();
    assert!(state.next("client").is_none());
    state.publish("a", 1, None).unwrap();
    assert!(state.next("client").is_none());
    state.start("client", "b", None).unwrap();
    state.close("client");
    assert_eq!(
        state.start("client", "a", None),
        Err(SubscriptionError::StreamEnded)
    );
}

#[test]
fn test_再開_履歴内と古い版と別起動と未来の版() {
    // Given
    let mut state = registry();
    state.publish("a", 1, None).unwrap();
    let version = Version {
        epoch: "boot".into(),
        sequence: 0,
    };
    // When
    state.start("client", "a", Some(&version)).unwrap();
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
        state.stop("client", "a").unwrap();
        state.start("client", "a", Some(&version)).unwrap();
        assert!(
            matches!(state.next("client"), Some((_, Event::Snapshot(_, value))) if *value == 1)
        );
        assert!(matches!(
            state.next("client"),
            Some((_, Event::Bookmark(_)))
        ));
    }
    for n in 2..=70 {
        state.publish("a", n, None).unwrap();
    }
    state.stop("client", "a").unwrap();
    state
        .start(
            "client",
            "a",
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
    state.start("client", "a", None).unwrap();
    state.start("client", "b", None).unwrap();
    for _ in 0..4 {
        state.next("client").unwrap();
    }
    // When
    for n in 1..=70 {
        state.publish("a", n, None).unwrap();
    }
    state.publish("b", 10, Some(2)).unwrap();
    // Then
    assert!(
        matches!(state.next("client"), Some((id, Event::Snapshot(Version {sequence:70,..}, value))) if id == "a" && *value == 70)
    );
    assert!(
        matches!(state.next("client"), Some((id, Event::Change(_, Delivery::Delta, value))) if id == "b" && *value == 2)
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
        let id = n.to_string();
        state.register(id.clone(), n, Delivery::Full).unwrap();
        state.start("client", &id, None).unwrap();
    }
    state.publish("a", 5, None).unwrap();
    state.close("client");
    state.open("client".into()).unwrap();
    state.start("client", "a", None).unwrap();
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
    state.start("client", "a", None).unwrap();
    state.next("client").unwrap();
    // When
    for n in 1..=64 {
        state.publish("a", n, None).unwrap();
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
    state.stop("client", "a").unwrap();
    state
        .start(
            "client",
            "a",
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
    state.publish("a", 64, None).unwrap();
    assert!(state.next("client").is_none());
}
