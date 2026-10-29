pub(crate) mod tests {
    use super::super::*;

    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering},
        Arc,
    };

    struct ObservedWindow {
        visible: Arc<AtomicBool>,
    }

    impl WindowVisibility for ObservedWindow {
        fn hide(&self) -> Result<(), tauri::Error> {
            self.visible.store(false, AtomicOrdering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn test_ウィンドウ閉鎖_close_requestedを処理しても終了要求せず再表示できる() {
        for label in [NORMAL_WINDOW_LABEL, STARTUP_FAILURE_WINDOW_LABEL] {
            // Given
            let app = tauri::test::mock_builder()
                .build(tauri::test::mock_context(tauri::test::noop_assets()))
                .unwrap();
            let window = tauri::WebviewWindowBuilder::new(&app, label, Default::default())
                .build()
                .unwrap();
            let closed = std::sync::Arc::new(AtomicUsize::new(0));
            let observed = closed.clone();
            let visible = Arc::new(AtomicBool::new(true));
            let observed_visibility = visible.clone();
            let mut cleanup = false;
            // When
            app.run_return(move |app, event| match event {
                tauri::RunEvent::Ready => window.close().unwrap(),
                event @ tauri::RunEvent::WindowEvent {
                    event: tauri::WindowEvent::CloseRequested { .. },
                    ..
                } => {
                    handle_run_event_with_window(
                        event,
                        |_| panic!("CloseRequested must not enter application quit"),
                        || panic!("CloseRequested must not reopen the window"),
                        |closed_label| {
                            assert_eq!(closed_label, label);
                            Some(ObservedWindow {
                                visible: visible.clone(),
                            })
                        },
                    );
                    observed.fetch_add(1, AtomicOrdering::SeqCst);
                }
                tauri::RunEvent::MainEventsCleared
                    if observed.load(AtomicOrdering::SeqCst) == 1 && !cleanup =>
                {
                    // Then: prevent_close preserved the window through the event loop.
                    let window = app.get_webview_window(label).expect("window was destroyed");
                    assert!(!visible.load(AtomicOrdering::SeqCst));
                    window.show().unwrap();
                    window.unminimize().unwrap();
                    cleanup = true;
                    window.destroy().unwrap();
                }
                tauri::RunEvent::ExitRequested { .. } if !cleanup => {
                    panic!("window close must not request application exit");
                }
                _ => {}
            });
            assert_eq!(closed.load(AtomicOrdering::SeqCst), 1);
            assert!(!observed_visibility.load(AtomicOrdering::SeqCst));
        }
    }

    #[test]
    fn exit_is_prevented_until_tray_quit_is_requested() {
        let _guard = crate::infrastructure::platform::tray::QUIT_REQUESTED_TEST_LOCK
            .lock()
            .unwrap();
        crate::infrastructure::platform::tray::QUIT_REQUESTED.store(false, Ordering::SeqCst);
        assert!(should_prevent_exit());

        crate::infrastructure::platform::tray::QUIT_REQUESTED.store(true, Ordering::SeqCst);
        assert!(!should_prevent_exit());

        crate::infrastructure::platform::tray::QUIT_REQUESTED.store(false, Ordering::SeqCst);
    }
}
