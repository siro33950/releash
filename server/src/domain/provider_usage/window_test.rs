use super::*;

#[test]
fn test_使用率_窓の境界と警告閾値を検証する() {
    // Given / When / Then
    assert!(UsageWindow::new(0.0, 300, 1).is_ok());
    assert!(UsageWindow::new(100.0, 300, 1).is_ok());
    for percent in [-1.0, 100.1, f64::NAN, f64::INFINITY] {
        assert!(UsageWindow::new(percent, 300, 1).is_err());
    }
    assert!(UsageWindow::new(1.0, 0, 1).is_err());
    assert!(UsageWindow::new(1.0, 300, 0).is_err());
    assert!(UsageWindow::new(1.0, 300, u64::MAX).is_err());
    assert!(!UsageWindow::new(80.0, 300, 1).unwrap().warning());
    assert!(UsageWindow::new(80.1, 300, 1).unwrap().warning());
}
