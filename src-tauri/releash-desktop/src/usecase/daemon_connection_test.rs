use super::*;
use parking_lot::Mutex;

#[derive(Default)]
struct FakeConnection {
    calls: Mutex<Vec<&'static str>>,
    endpoint: Mutex<Option<DaemonEndpoint>>,
    subscribed: Mutex<Option<DaemonEndpoint>>,
    failure: Mutex<Option<ConnectionError>>,
    subscribe_error: Mutex<Option<String>>,
    stop_error: Mutex<Option<String>>,
}
fn endpoint(token: &str) -> DaemonEndpoint {
    DaemonEndpoint {
        url: "http://127.0.0.1:1234".into(),
        token: token.into(),
    }
}
#[async_trait::async_trait]
impl DaemonConnectionPort for FakeConnection {
    async fn discover(&self) -> Result<Option<DaemonEndpoint>, ConnectionError> {
        self.calls.lock().push("discover");
        Ok(self.endpoint.lock().clone())
    }
    async fn start(&self) -> Result<(), ConnectionError> {
        self.calls.lock().push("start");
        *self.endpoint.lock() = Some(endpoint("new"));
        Ok(())
    }
    async fn subscribe(&self, endpoint: &DaemonEndpoint) -> Result<(), ConnectionError> {
        self.calls.lock().push("subscribe");
        if let Some(error) = self.subscribe_error.lock().clone() {
            return Err(error.into());
        }
        *self.subscribed.lock() = Some(endpoint.clone());
        Ok(())
    }
    async fn stop(&self) -> Result<(), ConnectionError> {
        self.calls.lock().push("stop");
        if let Some(error) = self.stop_error.lock().clone() {
            return Err(error.into());
        }
        *self.endpoint.lock() = None;
        *self.subscribed.lock() = None;
        Ok(())
    }
    fn subscribed_to(&self, target: &DaemonEndpoint) -> bool {
        self.subscribed.lock().as_ref() == Some(target)
    }
    fn record_failure(&self, failure: Option<ConnectionError>) {
        *self.failure.lock() = failure;
    }
}
impl DaemonConnectionQueryService for FakeConnection {
    fn failure(&self) -> Option<ConnectionFailure> {
        self.failure
            .lock()
            .as_ref()
            .map(|failure| ConnectionFailure {
                message: failure.message.clone(),
                server_older: failure.server_older,
            })
    }
    fn settings(&self) -> Option<DesktopSettingsDto> {
        None
    }
    fn settings_update(&self) -> Option<DesktopSettingsDto> {
        None
    }
}

#[tokio::test]
async fn test_接続先要求_初回設定の拒否や期限切れでは成功せず再発見だけでは起動しない() {
    // Given
    let port = Arc::new(FakeConnection::default());
    let usecase = DaemonConnectionUsecase::new(port.clone(), port.clone());
    // When / Then
    assert!(usecase.endpoint().await.is_err());
    assert_eq!(*port.calls.lock(), ["discover"]);
    *port.endpoint.lock() = Some(endpoint("first"));
    for error in ["subscription denied", "deadline has elapsed"] {
        *port.subscribe_error.lock() = Some(error.into());
        assert_eq!(usecase.endpoint().await.unwrap_err().message, error);
        assert_eq!(usecase.failure().unwrap().message, error);
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
    *port.endpoint.lock() = Some(endpoint("external"));
    let (current, changed) = usecase.endpoint().await.unwrap();
    // Then
    assert!(changed);
    assert_eq!(current, endpoint("external"));
    assert_eq!(*port.subscribed.lock(), Some(current));
    assert!(!port.calls.lock().contains(&"start"));
}

#[tokio::test]
async fn test_入れ替え_停止完了後に起動と初回設定受信へ進み停止失敗時は進まない() {
    // Given
    let port = Arc::new(FakeConnection::default());
    *port.endpoint.lock() = Some(endpoint("old"));
    let usecase = DaemonConnectionUsecase::new(port.clone(), port.clone());
    *port.stop_error.lock() = Some("shutdown incomplete".into());
    // When / Then
    assert_eq!(
        usecase.replace().await.unwrap_err().message,
        "shutdown incomplete"
    );
    assert_eq!(*port.calls.lock(), ["stop"]);
    assert_eq!(*port.endpoint.lock(), Some(endpoint("old")));
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
