use super::*;

#[test]
fn test_接続後の窓_ログイン起動の設定と失敗窓からの復旧に従う() {
    // Given
    for (hidden, start_minimized, failure, expected) in [
        (true, true, false, false),
        (false, true, false, true),
        (true, false, false, true),
        (true, true, true, true),
    ] {
        // When
        let show = show_after_connection(hidden, start_minimized, failure);
        // Then
        assert_eq!(
            show,
            if expected {
                ConnectedWindow::Visible
            } else {
                ConnectedWindow::Hidden
            }
        );
    }
}

#[test]
fn test_表示する窓_接続の有無に従う() {
    // When / Then
    assert_eq!(window(true), DesktopWindow::Normal);
    assert_eq!(window(false), DesktopWindow::ConnectionFailure);
}

#[test]
fn test_接続待ち中の表示要求_hiddenでも接続後に表示し次の接続へ持ち越さない() {
    // Given
    let mut lifecycle = DesktopLifecycle::default();
    // When / Then
    assert_eq!(lifecycle.show(false), DesktopWindow::ConnectionFailure);
    assert_eq!(
        lifecycle.connected(true, true, false),
        ConnectedWindow::Visible
    );
    assert_eq!(
        lifecycle.connected(true, true, false),
        ConnectedWindow::Hidden
    );
    assert_eq!(lifecycle.show(true), DesktopWindow::Normal);
    assert_eq!(
        lifecycle.connected(true, true, false),
        ConnectedWindow::Hidden
    );
}
