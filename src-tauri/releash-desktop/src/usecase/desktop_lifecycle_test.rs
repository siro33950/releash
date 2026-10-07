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
    fn initial_settings(&self) -> Option<DesktopSettingsDto> {
        self.settings()
    }
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
async fn test_接続後処理_初回設定を一度復元して窓の値を返し再要求では適用しない() {
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
    let initial = lifecycle.initialize(Some(true), false).await.unwrap();
    assert_eq!(initial.window, Some(ConnectedWindow::Hidden));
    assert_eq!(initial.settings, Some(settings));
    assert!(initial.restoration.is_ok());
    assert_eq!(*login.0.lock(), ["register"]);
    assert_eq!(lifecycle.show(), DesktopWindow::Normal);
    let (_, unchanged) = lifecycle.endpoint(false).await.unwrap();
    assert!(unchanged.window.is_none());
    assert!(unchanged.settings.is_none());
    assert_eq!(*login.0.lock(), ["register"]);
    let (_, recovery) = lifecycle.endpoint(true).await.unwrap();
    assert_eq!(recovery.window, Some(ConnectedWindow::Visible));
    assert!(recovery.settings.is_none());
    assert_eq!(*login.0.lock(), ["register"]);
    let started = lifecycle.initialize(None, false).await.unwrap();
    assert!(started.window.is_none());
    let replaced = lifecycle.replace(false).await.unwrap();
    assert_eq!(replaced.window, Some(ConnectedWindow::Visible));
    assert_eq!(replaced.settings, Some(settings));
    assert_eq!(*login.0.lock(), ["register", "register"]);
    lifecycle.settings_changed(settings).unwrap();
    assert_eq!(*login.0.lock(), ["register", "register", "register"]);
    lifecycle.stop().await.unwrap();
    assert_eq!(*port.1.lock(), ["stop", "stop"]);
}
#[tokio::test]
async fn test_設定変更_復元失敗を返し初回接続では窓と適用する設定を返す() {
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
        Arc::new(LoginItemUsecase::new(login.clone(), login.clone())),
    );
    assert_eq!(
        lifecycle
            .settings_changed(settings)
            .unwrap_err()
            .to_string(),
        "registration denied"
    );
    // When
    let connected = lifecycle.initialize(None, false).await.unwrap();
    // Then
    assert_eq!(connected.settings, Some(settings));
    assert_eq!(
        connected.restoration.unwrap_err().to_string(),
        "registration denied"
    );
    assert_eq!(connected.window, Some(ConnectedWindow::Visible));
    assert_eq!(*login.0.lock(), ["register", "register"]);
}
