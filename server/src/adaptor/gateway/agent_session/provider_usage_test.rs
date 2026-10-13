use super::*;

#[test]
fn test_使用率_read_modelはproviderごとの最後の値だけを持ち再生成で空になる() {
    // Given
    let model = ProviderUsageReadModel::default();
    let usage = |percent| ProviderUsage {
        provider: ProviderKind::Claude,
        windows: vec![UsageWindow::new(percent, 300, 123).unwrap()],
        plan: None,
    };
    // When
    model.replace(usage(20.0));
    model.replace(usage(30.0));
    // Then
    assert_eq!(model.snapshot().len(), 1);
    assert_eq!(model.snapshot()[0].windows[0].used_percent, 30.0);
    assert!(ProviderUsageReadModel::default().snapshot().is_empty());
}

#[test]
fn test_使用率保存_重複開始は開始時刻と既読を保ち終了後に削除する() {
    // Given
    let model = CodexUsageObservationStore::default();
    let start = std::time::UNIX_EPOCH;
    let record = CodexUsageRecord {
        id: "first".into(),
        timestamp: start,
        usage: ProviderUsage::claude(Vec::new(), None, None),
    };
    model.start_observation(CodexUsageObservation::new("session", start));
    assert!(model.accept_record("session", &record));
    // When
    model.start_observation(CodexUsageObservation::new(
        "session",
        start + std::time::Duration::from_secs(1),
    ));
    // Then
    assert!(!model.accept_record("session", &record));
    assert!(model.accept_record(
        "session",
        &CodexUsageRecord {
            id: "second".into(),
            ..record.clone()
        }
    ));
    model.retain_observations(&[], start);
    assert!(model.has_observation("session"));
    model.retain_observations(&[], start + std::time::Duration::from_secs(2));
    assert!(!model.has_observation("session"));
    assert!(!model.accept_record("session", &record));
}

#[test]
fn test_使用率日時_rfc3339の小数とoffsetを読み不正値とepoch前を拒否する() {
    // Given / When / Then
    let expected = std::time::UNIX_EPOCH + std::time::Duration::new(1, 123456789);
    assert_eq!(
        parse_timestamp("1970-01-01T01:00:01.123456789+01:00").unwrap(),
        expected
    );
    assert_eq!(
        parse_timestamp("1970-01-01T00:00:00Z").unwrap(),
        std::time::UNIX_EPOCH
    );
    for invalid in [
        "invalid",
        "2026-02-30T00:00:00Z",
        "1969-12-31T23:59:59.999Z",
    ] {
        assert!(matches!(parse_timestamp(invalid), Err(UsageError::Invalid)));
    }
}
