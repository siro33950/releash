use super::*;
use crate::domain::daemon_connection::{DaemonResult, DiscoveredDaemon};
use crate::usecase::daemon_connection_query::DaemonConnectionQueryService;
use parking_lot::Mutex;

#[derive(Default)]
struct FakeConnection {
    calls: Mutex<Vec<&'static str>>,
    server: Mutex<Option<DiscoveredDaemon>>,
    subscribed: Mutex<Option<DaemonEndpoint>>,
    start_error: Mutex<Option<DaemonConnectionState>>,
    subscribe_error: Mutex<Option<DaemonConnectionState>>,
    stop_error: Mutex<Option<DaemonConnectionState>>,
}
fn endpoint(token: &str) -> DaemonEndpoint {
    DaemonEndpoint {
        url: "http://127.0.0.1:1234".into(),
        token: token.into(),
    }
}
fn server(token: &str) -> DiscoveredDaemon {
    DiscoveredDaemon {
        endpoint: endpoint(token),
        protocol: 1,
        release: "server".into(),
    }
}
impl DaemonService for FakeConnection {
    fn discover(&self) -> DaemonResult<'_, Option<DiscoveredDaemon>> {
        Box::pin(async {
            self.calls.lock().push("discover");
            Ok(self.server.lock().clone())
        })
    }
    fn connect<'a>(&'a self, endpoint: &'a DaemonEndpoint) -> DaemonResult<'a, ()> {
        Box::pin(async {
            self.calls.lock().push("subscribe");
            if let Some(error) = self.subscribe_error.lock().clone() {
                return Err(error);
            }
            *self.subscribed.lock() = Some(endpoint.clone());
            Ok(())
        })
    }
    fn start(&self) -> DaemonResult<'_, ()> {
        Box::pin(async {
            self.calls.lock().push("start");
            if let Some(error) = self.start_error.lock().clone() {
                return Err(error);
            }
            *self.server.lock() = Some(server("new"));
            Ok(())
        })
    }
    fn stop(&self) -> DaemonResult<'_, ()> {
        Box::pin(async {
            self.calls.lock().push("stop");
            if let Some(error) = self.stop_error.lock().clone() {
                return Err(error);
            }
            *self.server.lock() = None;
            *self.subscribed.lock() = None;
            Ok(())
        })
    }
}

impl DaemonConnectionQueryService for FakeConnection {
    fn settings(&self) -> Option<DesktopSettingsDto> {
        None
    }
}

#[tokio::test]
async fn test_接続先要求_初回設定の拒否や期限切れでは成功せず再発見だけでは起動しない() {
    // Given
    let port = Arc::new(FakeConnection::default());
    let usecase = DaemonConnectionUsecase::new(port.clone(), port.clone(), 1, "client".into());
    // When / Then
    assert!(usecase.failure().is_none());
    assert!(usecase.endpoint().await.is_err());
    assert_eq!(*port.calls.lock(), ["discover"]);
    *port.server.lock() = Some(server("first"));
    for detail in [Some("subscription denied".into()), None] {
        let error = DaemonConnectionState::InitialSettingsUnavailable { detail };
        *port.subscribe_error.lock() = Some(error.clone());
        assert_eq!(usecase.endpoint().await.unwrap_err(), error);
        assert_eq!(usecase.failure(), Some(error));
        assert!(port.subscribed.lock().is_none());
    }
    // When
    *port.subscribe_error.lock() = None;
    let (first, changed) = usecase.endpoint().await.unwrap();
    // Then
    assert!(changed);
    assert_eq!(first, endpoint("first"));
    assert!(usecase.failure().is_none());
    assert!(!usecase.endpoint().await.unwrap().1);
    // When
    *port.server.lock() = Some(server("external"));
    let (current, changed) = usecase.endpoint().await.unwrap();
    // Then
    assert!(changed);
    assert_eq!(current, endpoint("external"));
    assert_eq!(*port.subscribed.lock(), Some(current));
    assert!(!port.calls.lock().contains(&"start"));
}
#[tokio::test]
async fn test_接続_起動失敗を集約へ記録する() {
    // Given
    let port = Arc::new(FakeConnection::default());
    let error = DaemonConnectionState::StartupFailed {
        status: Some("exit status: 7".into()),
        stderr: "startup denied".into(),
    };
    *port.start_error.lock() = Some(error.clone());
    let usecase = DaemonConnectionUsecase::new(port.clone(), port.clone(), 1, "client".into());
    // When / Then
    assert_eq!(usecase.connect().await.unwrap_err(), error);
    assert_eq!(usecase.failure(), Some(error));
    assert_eq!(*port.calls.lock(), ["discover", "start"]);
}
#[tokio::test]
async fn test_入れ替え_停止完了後に起動と初回設定受信へ進み停止失敗時は進まない() {
    // Given
    let port = Arc::new(FakeConnection::default());
    *port.server.lock() = Some(server("old"));
    let usecase = DaemonConnectionUsecase::new(port.clone(), port.clone(), 1, "client".into());
    let error = DaemonConnectionState::TechnicalFailure("shutdown incomplete".into());
    *port.stop_error.lock() = Some(error.clone());
    // When / Then
    assert_eq!(usecase.replace().await.unwrap_err(), error);
    assert_eq!(*port.calls.lock(), ["stop"]);
    assert_eq!(*port.server.lock(), Some(server("old")));
    // When
    port.calls.lock().clear();
    *port.stop_error.lock() = None;
    usecase.replace().await.unwrap();
    // Then
    assert_eq!(
        *port.calls.lock(),
        ["stop", "discover", "start", "discover", "subscribe"]
    );
    assert_eq!(*port.subscribed.lock(), Some(endpoint("new")));
}
#[tokio::test]
async fn test_接続_互換でないサーバには購読しない() {
    // Given
    let port = Arc::new(FakeConnection::default());
    let mut discovered = server("newer");
    discovered.protocol += 1;
    *port.server.lock() = Some(discovered);
    let usecase = DaemonConnectionUsecase::new(port.clone(), port.clone(), 1, "client".into());
    // When / Then
    assert!(usecase.connect().await.is_err());
    assert_eq!(*port.calls.lock(), ["discover"]);
}

#[tokio::test]
async fn test_接続_注入した画面のprotocolとreleaseで互換を判断する() {
    // Given
    let port = Arc::new(FakeConnection::default());
    let mut discovered = server("same");
    discovered.protocol = 2;
    *port.server.lock() = Some(discovered);
    let usecase =
        DaemonConnectionUsecase::new(port.clone(), port.clone(), 2, "injected-client".into());
    // When / Then
    usecase.connect().await.unwrap();
    assert_eq!(*port.subscribed.lock(), Some(endpoint("same")));
    port.server.lock().as_mut().unwrap().protocol = 3;
    assert_eq!(
        usecase.connect().await.unwrap_err(),
        DaemonConnectionState::Incompatible {
            server_older: false,
            server_release: "server".into(),
            client_release: "injected-client".into(),
        }
    );
}
