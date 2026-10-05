use super::*;
use crate::domain::daemon_supervision::DesktopUpdateInstaller;
#[derive(Default)]
struct Runtime {
    failure: Option<&'static str>,
    calls: parking_lot::Mutex<Vec<String>>,
}
#[async_trait::async_trait]
impl UpdateRuntime for Arc<Runtime> {
    async fn check(&self) -> Result<Option<Arc<dyn UpdatePackage>>, String> {
        self.calls.lock().push("check".into());
        Ok(Some(self.clone()))
    }
    fn progress(&self, percent: u32) {
        self.calls.lock().push(format!("progress:{percent}"));
    }
    fn restart(&self) -> Result<(), String> {
        self.calls.lock().push("restart".into());
        if self.failure == Some("restart") {
            Err("restart failed".into())
        } else {
            Ok(())
        }
    }
}
#[async_trait::async_trait]
impl UpdatePackage for Runtime {
    fn info(&self) -> UpdateInfo {
        UpdateInfo {
            version: "next".into(),
            notes: "notes".into(),
        }
    }
    async fn download(&self, progress: Progress) -> Result<Vec<u8>, String> {
        self.calls.lock().push("download".into());
        if self.failure == Some("download") {
            return Err("download failed".into());
        }
        progress(1, Some(2));
        progress(1, Some(2));
        Ok(vec![1, 2])
    }
    fn install(&self, bytes: Vec<u8>) -> Result<(), String> {
        assert_eq!(bytes, [1, 2]);
        self.calls.lock().push("install".into());
        if self.failure == Some("install") {
            Err("install failed".into())
        } else {
            Ok(())
        }
    }
}
#[tokio::test]
async fn test_更新gateway_検証済みdownloadとinstallとrestartの結果を伝える() {
    // Given
    let runtime = Arc::new(Runtime::default());
    let gateway = TauriUpdateGateway::with_runtime(Arc::new(runtime.clone()));
    // When / Then
    assert_eq!(gateway.check().await.unwrap().unwrap().version, "next");
    assert_eq!(
        gateway.install().await.unwrap_err(),
        "No verified update has been downloaded."
    );
    // When
    gateway.download().await.unwrap();
    gateway.install().await.unwrap();
    gateway.restart().unwrap();
    // Then
    assert_eq!(
        *runtime.calls.lock(),
        [
            "check",
            "download",
            "progress:50",
            "progress:100",
            "install",
            "restart"
        ]
    );
    assert!(gateway.install().await.is_err());
}
#[tokio::test]
async fn test_更新gateway_各境界の失敗理由を握り潰さず伝える() {
    for failure in ["download", "install", "restart"] {
        // Given
        let runtime = Arc::new(Runtime {
            failure: Some(failure),
            ..Default::default()
        });
        let gateway = TauriUpdateGateway::with_runtime(Arc::new(runtime));
        gateway.check().await.unwrap();
        // When
        let result = async {
            gateway.download().await?;
            gateway.install().await?;
            gateway.restart()
        }
        .await;
        // Then
        assert_eq!(result.unwrap_err(), format!("{failure} failed"));
    }
}
