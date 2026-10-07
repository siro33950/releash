use super::*;
use crate::domain::{daemon_connection::*, login_item::*};
use crate::usecase::daemon_connection_query::DaemonConnectionQueryService;
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
                protocol: 1,
                release: "server".into(),
            }))
        })
    }
    fn connect<'a>(&'a self, _: &'a DaemonEndpoint) -> DaemonResult<'a, ()> {
        Box::pin(async { Ok(()) })
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

#[derive(Default)]
struct Login(Mutex<Vec<&'static str>>, Mutex<Option<String>>);
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
        if let Some(error) = self.1.lock().clone() {
            return Err(error);
        }
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
async fn test_接続後処理_窓の値を返し設定の復元は通知の入口で一度だけ行う() {
    // Given
    let settings = DesktopSettingsDto {
        close_to_tray: true,
        start_minimized: true,
        crash_reporting: false,
        performance_telemetry: false,
        auto_launch: true,
    };
    let port = Arc::new(FakeConnection(Mutex::new(settings), Mutex::new(Vec::new())));
    let connection = Arc::new(DaemonConnectionUsecase::new(
        port.clone(),
        port.clone(),
        1,
        "client".into(),
    ));
    let login = Arc::new(Login::default());
    let lifecycle = DesktopLifecycleUsecase::new(
        connection,
        Arc::new(LoginItemUsecase::new(login.clone(), login.clone())),
    );
    // When / Then
    assert_eq!(lifecycle.show(), DesktopWindow::ConnectionFailure);
    lifecycle.initialize().await.unwrap();
    assert_eq!(
        lifecycle.connected(true, false, true),
        Some(ConnectedWindow::Hidden)
    );
    assert_eq!(
        lifecycle.connected(true, true, true),
        Some(ConnectedWindow::Visible)
    );
    assert_eq!(lifecycle.show(), DesktopWindow::Normal);
    assert!(!lifecycle.endpoint().await.unwrap().1);
    assert!(lifecycle.connected(false, false, false).is_none());
    assert_eq!(
        lifecycle.connected(true, true, false),
        Some(ConnectedWindow::Visible)
    );
    lifecycle.start().await.unwrap();
    assert_eq!(
        lifecycle.connected(false, false, true),
        Some(ConnectedWindow::Visible)
    );
    lifecycle.replace().await.unwrap();
    assert_eq!(
        lifecycle.connected(false, false, true),
        Some(ConnectedWindow::Visible)
    );
    assert!(login.0.lock().is_empty());
    let applied = lifecycle.settings_changed(settings).unwrap();
    assert_eq!(applied.close_to_tray, settings.close_to_tray);
    assert_eq!(
        applied.performance_telemetry,
        settings.performance_telemetry
    );
    assert_eq!(*login.0.lock(), ["register"]);
    lifecycle.stop().await.unwrap();
    assert_eq!(*port.1.lock(), ["stop", "stop"]);
}
#[tokio::test]
async fn test_設定変更_ログイン項目の復元失敗を返す() {
    // Given
    let settings = DesktopSettingsDto {
        close_to_tray: true,
        start_minimized: false,
        crash_reporting: false,
        performance_telemetry: false,
        auto_launch: true,
    };
    let port = Arc::new(FakeConnection(Mutex::new(settings), Mutex::new(Vec::new())));
    let connection = Arc::new(DaemonConnectionUsecase::new(
        port.clone(),
        port,
        1,
        "client".into(),
    ));
    let login = Arc::new(Login::default());
    *login.1.lock() = Some("registration denied".into());
    let lifecycle = DesktopLifecycleUsecase::new(
        connection,
        Arc::new(LoginItemUsecase::new(login.clone(), login)),
    );
    assert_eq!(
        lifecycle
            .settings_changed(settings)
            .unwrap_err()
            .to_string(),
        "registration denied"
    );
}
