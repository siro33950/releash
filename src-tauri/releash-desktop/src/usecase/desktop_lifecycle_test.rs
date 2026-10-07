use super::*;
use crate::domain::{daemon_connection::*, login_item::*};
use crate::usecase::daemon_connection_query::{
    DaemonConnectionQueryService, DesktopSettingsSubscription,
};
use parking_lot::Mutex;

struct FakeConnection(Mutex<DesktopSettingsDto>, Mutex<Vec<&'static str>>);
impl DaemonService for FakeConnection {
    fn discover(&self) -> DaemonResult<'_, Option<DiscoveredDaemon>> {
        Box::pin(async {
            Ok(Some(DiscoveredDaemon {
                endpoint: DaemonEndpoint {
                    url: "localhost".into(),
                    token: "test".into(),
                },
                protocol: releash_sdk::descriptor::protocol(),
                release: "server".into(),
            }))
        })
    }
    fn start(&self) -> DaemonResult<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn stop(&self) -> DaemonResult<'_, ()> {
        Box::pin(async {
            self.1.lock().push("stop");
            Ok(())
        })
    }
}
impl DaemonConnectionQueryService for FakeConnection {
    fn settings(&self) -> Option<DesktopSettingsDto> {
        Some(*self.0.lock())
    }
}
impl DesktopSettingsSubscription for FakeConnection {
    fn connect<'a>(&'a self, _: &'a DaemonEndpoint) -> DaemonResult<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}
#[derive(Default)]
struct Output {
    calls: Mutex<Vec<&'static str>>,
    confirmed: Mutex<bool>,
    confirmation_error: Mutex<Option<String>>,
}
impl DesktopLifecycleOutput for Output {
    fn apply_settings(&self, _: DesktopSettingsDto) {
        self.calls.lock().push("settings");
    }
    fn failure_window(&self) -> bool {
        false
    }
    fn show(&self, ready: bool) -> Result<(), String> {
        self.calls
            .lock()
            .push(if ready { "show" } else { "failure" });
        Ok(())
    }
    fn connected_window(&self, visible: bool) {
        self.calls
            .lock()
            .push(if visible { "show" } else { "hidden" });
    }
    fn connection_failed(&self, _: String) {
        self.calls.lock().push("failure");
    }
    fn confirm_stop(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, String>> + Send + '_>>
    {
        Box::pin(async {
            if let Some(error) = self.confirmation_error.lock().clone() {
                return Err(error);
            }
            Ok(*self.confirmed.lock())
        })
    }
    fn stop_failed(&self, _: String) {
        self.calls.lock().push("stop failure");
    }
}
#[derive(Default)]
struct Login(Mutex<Vec<&'static str>>);
impl LoginItemPort for Login {
    fn status(&self) -> Result<LoginItemStatus, String> {
        Ok(LoginItemStatus::NotRegistered)
    }
    fn location(&self) -> Result<RegistrationLocation, String> {
        Ok(RegistrationLocation {
            translocated: false,
            read_only: false,
        })
    }
    fn register(&self) -> Result<(), String> {
        self.0.lock().push("register");
        Ok(())
    }
    fn unregister(&self) -> Result<(), String> {
        Ok(())
    }
    fn open_settings(&self) -> Result<(), String> {
        Ok(())
    }
}
#[async_trait::async_trait]
impl LoginPreferencePort for Login {
    async fn load(&self) -> Result<bool, String> {
        Ok(false)
    }
    async fn save(&self, _: bool) -> Result<(), String> {
        Ok(())
    }
}
#[tokio::test]
async fn test_接続後処理_設定適用とログイン項目復元と窓表示を各入口で共有する() {
    // Given
    let settings = DesktopSettingsDto {
        close_to_tray: true,
        start_minimized: true,
        crash_reporting: false,
        performance_telemetry: false,
        auto_launch: true,
    };
    let port = Arc::new(FakeConnection(Mutex::new(settings), Mutex::new(Vec::new())));
    let connection = Arc::new(DaemonConnectionUsecase::new(port.clone(), port.clone()));
    let output = Arc::new(Output::default());
    let login = Arc::new(Login::default());
    let lifecycle = DesktopLifecycleUsecase::new(
        connection,
        Arc::new(LoginItemUsecase::new(login.clone(), login.clone())),
        output.clone(),
    );
    // When
    lifecycle.initialize(true).await.unwrap();
    // Then
    assert_eq!(*output.calls.lock(), ["settings", "hidden"]);
    assert_eq!(*login.0.lock(), ["register"]);
    // When / Then
    output.calls.lock().clear();
    lifecycle.endpoint().await.unwrap();
    assert!(output.calls.lock().is_empty());
    lifecycle.start().await.unwrap();
    assert_eq!(*output.calls.lock(), ["settings", "show"]);
    output.calls.lock().clear();
    lifecycle.replace().await.unwrap();
    assert_eq!(*output.calls.lock(), ["settings", "show"]);
    assert_eq!(*port.1.lock(), ["stop"]);
    output.calls.lock().clear();
    lifecycle.settings_changed(DesktopSettingsDto {
        auto_launch: false,
        ..settings
    });
    assert_eq!(*output.calls.lock(), ["settings"]);
    // When / Then
    lifecycle.confirm_stop().await.unwrap();
    assert_eq!(*port.1.lock(), ["stop"]);
    *output.confirmed.lock() = true;
    lifecycle.confirm_stop().await.unwrap();
    assert_eq!(*port.1.lock(), ["stop", "stop"]);
    // When / Then
    *output.confirmation_error.lock() = Some("confirmation closed".into());
    assert_eq!(
        lifecycle.confirm_stop().await,
        Err(DaemonConnectionState::TechnicalFailure(
            "confirmation closed".into()
        ))
    );
    assert_eq!(*port.1.lock(), ["stop", "stop"]);
}
