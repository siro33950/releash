use super::*;

#[test]
fn test_使用率_read_modelはproviderごとの最後の値だけを持ち再生成で空になる() {
    // Given
    let model = ProviderUsageReadModel::default();
    let usage = |percent| ProviderUsage {
        provider: ProviderKind::Claude,
        windows: vec![UsageWindow {
            used_percent: percent,
            window_minutes: 300,
            resets_at: 123,
            warning: false,
        }],
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
