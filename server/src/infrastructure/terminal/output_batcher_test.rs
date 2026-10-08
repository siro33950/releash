use std::time::{Duration, Instant};

use super::TerminalOutputBatcher;

#[test]
fn test_連続outputを2msか16kib境界まで内容と順序を変えず結合する() {
    let origin = Instant::now();
    let mut batcher = TerminalOutputBatcher::default();
    let chunk = "x".repeat(4 * 1024);

    assert!(batcher.push(origin, chunk.clone()).is_empty());
    assert!(batcher
        .push(origin + Duration::from_micros(500), chunk.clone())
        .is_empty());
    assert!(batcher
        .push(origin + Duration::from_millis(1), chunk.clone())
        .is_empty());
    assert_eq!(
        batcher.push(origin + Duration::from_micros(1500), chunk.clone()),
        vec![chunk.repeat(4)]
    );

    assert!(batcher.push(origin, "a".to_string()).is_empty());
    assert_eq!(
        batcher.flush_due(origin + Duration::from_millis(2)),
        Some("a".to_string())
    );
}

#[test]
fn test_16kib超のoutputはcode_unit境界で分割して返す() {
    let origin = Instant::now();
    let mut batcher = TerminalOutputBatcher::default();
    let chunk = "y".repeat(16 * 1024 + 100);

    let ready = batcher.push(origin, chunk);
    assert_eq!(ready, vec!["y".repeat(16 * 1024)]);
    assert_eq!(batcher.flush(), Some("y".repeat(100)));
}

#[test]
fn test_exit前flushで保留outputを返す() {
    let origin = Instant::now();
    let mut batcher = TerminalOutputBatcher::default();

    assert!(batcher.push(origin, "before-exit".to_string()).is_empty());
    assert_eq!(batcher.flush(), Some("before-exit".to_string()));
    assert_eq!(batcher.flush(), None);
}
