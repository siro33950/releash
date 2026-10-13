use super::*;

#[test]
fn test_使用率_窓の境界と警告閾値を検証する() {
    // Given / When / Then
    assert!(valid_window(0.0, 300, 1));
    assert!(valid_window(100.0, 300, 1));
    for percent in [-1.0, 100.1, f64::NAN, f64::INFINITY] {
        assert!(!valid_window(percent, 300, 1));
    }
    assert!(!valid_window(1.0, 0, 1));
    assert!(!valid_window(1.0, 300, 0));
    assert!(!valid_window(1.0, 300, u64::MAX));
    assert!(!warns(80.0));
    assert!(warns(80.1));
}
