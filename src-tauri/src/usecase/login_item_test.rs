use super::*;
#[derive(Default)]
struct Preference(std::sync::atomic::AtomicBool);
#[async_trait::async_trait]
impl LoginPreferencePort for Preference {
    async fn load(&self) -> Result<bool, String> {
        Ok(self.0.load(std::sync::atomic::Ordering::SeqCst))
    }
    async fn save(&self, requested: bool) -> Result<(), String> {
        self.0.store(requested, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}
fn service(port: Arc<dyn LoginItemPort>) -> LoginItemUsecase {
    LoginItemUsecase::new(port, Arc::new(Preference::default()))
}

struct FakeLogin {
    status: parking_lot::Mutex<LoginItemStatus>,
    calls: parking_lot::Mutex<Vec<&'static str>>,
    error: Option<(&'static str, String)>,
}
impl LoginItemPort for FakeLogin {
    fn status(&self) -> Result<LoginItemStatus, String> {
        if let Some(("status", error)) = &self.error {
            return Err(error.clone());
        }
        Ok(*self.status.lock())
    }
    fn location(&self) -> Result<crate::domain::login_item::RegistrationLocation, String> {
        Ok(crate::domain::login_item::RegistrationLocation {
            translocated: false,
            read_only: false,
        })
    }
    fn register(&self) -> Result<(), String> {
        self.calls.lock().push("register");
        if let Some(("register", error)) = &self.error {
            return Err(error.clone());
        }
        *self.status.lock() = LoginItemStatus::Enabled;
        Ok(())
    }
    fn unregister(&self) -> Result<(), String> {
        self.calls.lock().push("unregister");
        if let Some(("unregister", error)) = &self.error {
            return Err(error.clone());
        }
        *self.status.lock() = LoginItemStatus::NotRegistered;
        Ok(())
    }
    fn open_settings(&self) -> Result<(), String> {
        self.calls.lock().push("settings");
        if let Some(("settings", error)) = &self.error {
            return Err(error.clone());
        }
        Ok(())
    }
}
#[tokio::test]
async fn test_ログイン項目_承認待ちでも起動し設定への導線と無効状態を返す() {
    // Given
    let port = Arc::new(FakeLogin {
        status: parking_lot::Mutex::new(LoginItemStatus::RequiresApproval),
        calls: Default::default(),
        error: None,
    });
    let service = service(port.clone());
    // When
    service.restore(true).unwrap();
    let status = service.status().await.unwrap();
    service.open_settings().unwrap();
    // Then
    assert!(!status.enabled);
    assert!(status.requires_approval);
    assert_eq!(*port.calls.lock(), ["settings"]);
}
#[tokio::test]
async fn test_ログイン項目_失われた登録を復旧し無効化後の再読込でも無効になる() {
    // Given
    let port = Arc::new(FakeLogin {
        status: parking_lot::Mutex::new(LoginItemStatus::NotFound),
        calls: Default::default(),
        error: None,
    });
    let preference = Arc::new(Preference(std::sync::atomic::AtomicBool::new(true)));
    let service = LoginItemUsecase::new(port.clone(), preference.clone());
    // When
    service.restore(preference.load().await.unwrap()).unwrap();
    let status = service.status().await.unwrap();
    // Then
    assert!(status.enabled);
    assert!(status.requested);
    assert!(preference.load().await.unwrap());
    assert_eq!(*port.calls.lock(), ["register"]);
    // When
    service.set_enabled(false).await.unwrap();
    service.restore(false).unwrap();
    // Then
    assert!(!service.status().await.unwrap().enabled);
    assert_eq!(*port.calls.lock(), ["register", "unregister"]);
}
#[tokio::test]
async fn test_ログイン項目_登録失敗理由を保持して表示する() {
    // Given
    let service = service(Arc::new(FakeLogin {
        status: parking_lot::Mutex::new(LoginItemStatus::NotRegistered),
        calls: Default::default(),
        error: Some(("register", "read-only volume".into())),
    }));
    // When
    let error = service.restore(true).unwrap_err();
    // Then
    assert_eq!(error.0, "read-only volume");
    assert_eq!(
        service.status().await.unwrap().reason.as_deref(),
        Some("read-only volume")
    );
}

#[tokio::test]
async fn test_ログイン項目_状態取得失敗では登録変更を実行しない() {
    // Given
    let port = Arc::new(FakeLogin {
        status: parking_lot::Mutex::new(LoginItemStatus::Enabled),
        calls: Default::default(),
        error: Some(("status", "status unavailable".into())),
    });
    let service = service(port.clone());
    // When
    let restore = service.restore(true).unwrap_err();
    let disable = service.set_enabled(false).await.unwrap_err();
    // Then
    assert_eq!(restore.0, "status unavailable");
    assert_eq!(disable.0, "status unavailable");
    assert!(port.calls.lock().is_empty());
}

#[tokio::test]
async fn test_ログイン項目_解除と設定を開く失敗を成功に置き換えない() {
    for action in ["unregister", "settings"] {
        // Given
        let port = Arc::new(FakeLogin {
            status: parking_lot::Mutex::new(LoginItemStatus::Enabled),
            calls: Default::default(),
            error: Some((action, "operation failed".into())),
        });
        let service = service(port.clone());
        // When
        let error = if action == "unregister" {
            service.set_enabled(false).await.unwrap_err()
        } else {
            service.open_settings().unwrap_err()
        };
        // Then
        assert_eq!(error.0, "operation failed");
        assert!(service.status().await.unwrap().enabled);
        assert_eq!(*port.calls.lock(), [action]);
        if action == "unregister" {
            assert_eq!(
                service.status().await.unwrap().reason.as_deref(),
                Some("operation failed")
            );
        }
    }
}

#[tokio::test]
async fn test_登録希望_承認待ちの無効表示とは別にrustが希望を保存する() {
    // Given
    let port = Arc::new(FakeLogin {
        status: parking_lot::Mutex::new(LoginItemStatus::RequiresApproval),
        calls: Default::default(),
        error: None,
    });
    let preference = Arc::new(Preference::default());
    let service = LoginItemUsecase::new(port.clone(), preference.clone());
    // When
    let state = service.set_enabled(true).await.unwrap();
    // Then
    assert!(!state.enabled);
    assert!(state.requires_approval);
    assert!(state.requested);
    assert!(preference.load().await.unwrap());
    assert!(port.calls.lock().is_empty());
    service.set_enabled(false).await.unwrap();
    assert!(!service.status().await.unwrap().requested);
    assert_eq!(*port.calls.lock(), ["unregister"]);
}

#[tokio::test]
async fn test_登録希望_読込失敗ではosを変えず保存失敗は成功として返さない() {
    struct FailingPreference(bool);
    #[async_trait::async_trait]
    impl LoginPreferencePort for FailingPreference {
        async fn load(&self) -> Result<bool, String> {
            if self.0 {
                Err("load failed".into())
            } else {
                Preference::default().load().await
            }
        }
        async fn save(&self, _: bool) -> Result<(), String> {
            Err("save failed".into())
        }
    }
    for load_fails in [true, false] {
        // Given
        let port = Arc::new(FakeLogin {
            status: parking_lot::Mutex::new(LoginItemStatus::NotRegistered),
            calls: Default::default(),
            error: None,
        });
        let service = LoginItemUsecase::new(port.clone(), Arc::new(FailingPreference(load_fails)));
        // When
        let error = service.set_enabled(true).await.unwrap_err();
        // Then
        assert_eq!(
            error.0,
            if load_fails {
                "load failed"
            } else {
                "save failed"
            }
        );
        assert_eq!(
            *port.calls.lock(),
            if load_fails { vec![] } else { vec!["register"] }
        );
    }
}
