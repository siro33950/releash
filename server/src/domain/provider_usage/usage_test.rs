use super::*;
#[test]
fn test_使用率_claudeの窓とplanを導く() {
    // Given / When
    let usage = ProviderUsage::claude(
        vec![
            UsageWindow::five_hour(20.0, 123).unwrap(),
            UsageWindow::seven_day(90.0, 123).unwrap(),
        ],
        Some("claude_max"),
        Some("20x"),
    );
    // Then
    assert_eq!(usage.windows[0].window_minutes(), 300);
    assert_eq!(usage.windows[1].window_minutes(), 10080);
    assert_eq!(usage.plan.as_deref(), Some("claude_max · 20x"));
    assert!(ProviderUsage::claude(Vec::new(), Some(""), None)
        .plan
        .is_none());
}
