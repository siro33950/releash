use super::*;
use std::{cell::Cell, time::Duration};

#[tokio::test(start_paused = true)]
async fn test_daemon停止_自然終了なら強制終了しない() {
    // Given
    let start = tokio::time::Instant::now();
    // When
    wait_for_termination(
        || std::future::ready(Ok(start.elapsed() >= Duration::from_secs(1))),
        || panic!("a stopped daemon must not be killed"),
    )
    .await
    .unwrap();
    // Then
    assert_eq!(start.elapsed(), Duration::from_secs(1));
}

#[tokio::test(start_paused = true)]
async fn test_daemon停止_五秒後に一度強制終了して終了確認まで待つ() {
    // Given
    let start = tokio::time::Instant::now();
    let killed = Cell::new(false);
    // When
    wait_for_termination(
        || std::future::ready(Ok(start.elapsed() >= Duration::from_secs(6))),
        || {
            assert_eq!(start.elapsed(), Duration::from_secs(5));
            assert!(!killed.replace(true));
            Ok(())
        },
    )
    .await
    .unwrap();
    // Then
    assert!(killed.get());
    assert_eq!(start.elapsed(), Duration::from_secs(6));
}

#[tokio::test(start_paused = true)]
async fn test_daemon停止_強制終了後も終了未確認なら十秒で失敗する() {
    // Given
    let start = tokio::time::Instant::now();
    let kills = Cell::new(0);
    // When
    let error = wait_for_termination(
        || std::future::ready(Ok(false)),
        || {
            kills.set(kills.get() + 1);
            Ok(())
        },
    )
    .await
    .unwrap_err();
    // Then
    assert_eq!(
        error,
        "Daemon exit could not be confirmed after termination."
    );
    assert_eq!(start.elapsed(), Duration::from_secs(10));
    assert_eq!(kills.get(), 1);
}

#[tokio::test(start_paused = true)]
async fn test_daemon停止_終了観測とkillのエラーを伝播する() {
    // Given / When / Then
    assert_eq!(
        wait_for_termination(
            || std::future::ready(Err("wait failed".into())),
            || panic!("must not kill after an observation error"),
        )
        .await
        .unwrap_err(),
        "wait failed"
    );
    assert_eq!(
        wait_for_termination(
            || std::future::ready(Ok(false)),
            || Err("kill failed".into()),
        )
        .await
        .unwrap_err(),
        "kill failed"
    );
}
