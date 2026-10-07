use super::show_after_connection;

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
        assert_eq!(show, expected);
    }
}

#[test]
fn test_トレイ停止_agent終了を確認し承諾した場合だけ停止する() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    // Given
    let stopped = Arc::new(AtomicUsize::new(0));
    for confirmed in [false, true] {
        let count = stopped.clone();
        // When
        crate::infrastructure::platform::tray::dispatch_menu_event(
            crate::infrastructure::platform::tray::ids::STOP_DAEMON,
            || panic!("unexpected show"),
            || panic!("unexpected quit"),
            || {
                super::confirm_stop_with(
                    |message, reply| {
                        assert!(message.contains("agent の Session も停止"));
                        reply(confirmed);
                    },
                    move || {
                        count.fetch_add(1, Ordering::SeqCst);
                    },
                )
            },
        );
        // Then
        assert_eq!(stopped.load(Ordering::SeqCst), usize::from(confirmed));
    }
}
