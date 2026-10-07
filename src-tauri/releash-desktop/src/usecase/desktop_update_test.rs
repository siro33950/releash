use super::*;
#[derive(Default)]
struct FakeUpdate {
    calls: parking_lot::Mutex<Vec<&'static str>>,
    fail: Option<&'static str>,
    install_wait: Option<(tokio::sync::Notify, tokio::sync::Notify)>,
}
impl FakeUpdate {
    fn step(&self, step: &'static str) -> Result<(), String> {
        self.calls.lock().push(step);
        if self.fail == Some(step) {
            Err(step.into())
        } else {
            Ok(())
        }
    }
}
#[async_trait::async_trait]
impl DesktopUpdateGateway for FakeUpdate {
    async fn check(&self) -> Result<Option<UpdateInfo>, String> {
        Ok(None)
    }
}
#[async_trait::async_trait]
impl DesktopUpdateInstaller for FakeUpdate {
    async fn download(&self) -> Result<(), String> {
        self.step("download")
    }
    async fn install(&self) -> Result<(), String> {
        self.step("install")?;
        if let Some((entered, resume)) = &self.install_wait {
            entered.notify_one();
            resume.notified().await;
        }
        Ok(())
    }
    fn restart(&self) -> Result<(), String> {
        self.step("restart")
    }
}
#[tokio::test]
async fn test_画面更新_順に適用し失敗以降の操作を実行しない() {
    // Given
    for (fail, expected) in [
        (None, vec!["download", "install", "restart"]),
        (Some("download"), vec!["download"]),
        (Some("install"), vec!["download", "install"]),
        (Some("restart"), vec!["download", "install", "restart"]),
    ] {
        let gateway = Arc::new(FakeUpdate {
            fail,
            ..Default::default()
        });
        let update = DesktopUpdateUsecase::new(gateway.clone());
        // When
        let result = update.apply().await;
        // Then
        assert_eq!(result.is_ok(), fail.is_none());
        assert_eq!(*gateway.calls.lock(), expected);
    }
}

#[tokio::test]
async fn test_更新排他_適用中の二つ目の要求は副作用なしで拒否する() {
    // Given
    let gateway = Arc::new(FakeUpdate {
        install_wait: Some(Default::default()),
        ..Default::default()
    });
    let update = Arc::new(DesktopUpdateUsecase::new(gateway.clone()));
    let first = update.clone();
    let applying = tokio::spawn(async move { first.apply().await });
    let (entered, resume) = gateway.install_wait.as_ref().unwrap();
    entered.notified().await;
    // When
    let error = update.apply().await.unwrap_err();
    // Then
    assert_eq!(error.to_string(), "An update is already in progress.");
    assert_eq!(*gateway.calls.lock(), ["download", "install"]);
    resume.notify_one();
    applying.await.unwrap().unwrap();
    assert_eq!(*gateway.calls.lock(), ["download", "install", "restart"]);
}
