use super::*;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::provider_usage::ProviderUsage;
#[test]
fn test_使用率観測_開始前は除外し開始時刻以降の異なる記録だけを採る() {
    // Given
    let start = std::time::UNIX_EPOCH + std::time::Duration::from_secs(100);
    let mut observation = CodexUsageObservation::new("session", start);
    let record = |id: &str, seconds| CodexUsageRecord {
        id: id.into(),
        timestamp: std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds),
        usage: ProviderUsage {
            provider: ProviderKind::Codex,
            windows: Vec::new(),
            plan: None,
        },
    };
    // When / Then
    assert!(!observation.accept(&record("old", 99)));
    assert!(observation.accept(&record("equal", 100)));
    assert!(observation.accept(&record("new", 101)));
    assert!(!observation.accept(&record("new", 101)));
}

#[test]
fn test_使用率観測_poll開始後の観測を古いsession一覧で削除しない() {
    // Given
    let start = std::time::UNIX_EPOCH + std::time::Duration::from_secs(100);
    let observation = CodexUsageObservation::new("session", start);
    // When / Then
    assert!(observation.retain_for_poll(&[], start - std::time::Duration::from_secs(1)));
    assert!(observation.retain_for_poll(&[], start));
    assert!(!observation.retain_for_poll(&[], start + std::time::Duration::from_secs(1)));
    assert!(observation.retain_for_poll(
        &["session".into()],
        start + std::time::Duration::from_secs(1)
    ));
}

#[test]
fn test_使用率観測_開始境界をrolloutのミリ秒精度にそろえる() {
    // Given
    for start in [
        std::time::UNIX_EPOCH + std::time::Duration::from_micros(123_400),
        std::time::UNIX_EPOCH - std::time::Duration::from_micros(123_400),
    ] {
        let mut observation = CodexUsageObservation::new("session", start);
        let boundary = if start > std::time::UNIX_EPOCH {
            std::time::UNIX_EPOCH + std::time::Duration::from_millis(123)
        } else {
            std::time::UNIX_EPOCH - std::time::Duration::from_millis(124)
        };
        let record = |id: &str, timestamp| CodexUsageRecord {
            id: id.into(),
            timestamp,
            usage: ProviderUsage {
                provider: ProviderKind::Codex,
                windows: Vec::new(),
                plan: None,
            },
        };
        // When / Then
        assert!(!observation.accept(&record(
            "old",
            boundary - std::time::Duration::from_millis(1)
        )));
        assert!(observation.accept(&record("same-millisecond", boundary)));
        assert!(!observation.accept(&record("same-millisecond", boundary)));
        assert!(observation.accept(&record("equal", start)));
        assert!(!observation.accept(&record("equal", start)));
        assert!(observation.accept(&record(
            "later",
            boundary + std::time::Duration::from_millis(1)
        )));
    }
}
