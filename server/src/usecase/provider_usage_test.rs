use super::*;
use std::sync::Mutex;

#[derive(Default)]
struct Source(Mutex<Option<(String, ProviderUsage)>>);
#[async_trait::async_trait]
impl ProviderUsageSource for Source {
    fn claude(&self, payload: &[u8]) -> Result<Option<ProviderUsage>, UsageError> {
        if payload == b"invalid" {
            return Err(UsageError::Invalid);
        }
        Ok(self
            .0
            .lock()
            .unwrap()
            .as_ref()
            .map(|(_, usage)| usage.clone()))
    }
    async fn codex(&self, session: &str) -> Result<Option<(String, ProviderUsage)>, UsageError> {
        if session == "invalid" {
            return Err(UsageError::Invalid);
        }
        Ok(self.0.lock().unwrap().clone())
    }
}
#[derive(Default)]
struct Projection(Mutex<Vec<ProviderUsage>>);
impl ProviderUsageQueryService for Projection {
    fn snapshot(&self) -> Vec<ProviderUsage> {
        self.0.lock().unwrap().clone()
    }
}
impl ProviderUsageOutput for Projection {
    fn replace(&self, usage: ProviderUsage) {
        *self.0.lock().unwrap() = vec![usage];
    }
}
fn usage(percent: f64) -> ProviderUsage {
    ProviderUsage {
        provider: ProviderKind::Codex,
        windows: vec![UsageWindow {
            used_percent: percent,
            window_minutes: 300,
            resets_at: 123,
            warning: false,
        }],
        plan: None,
    }
}

#[tokio::test]
async fn test_使用率_変更を通知し同じrolloutの再読では上書きしない() {
    // Given
    let source = Arc::new(Source::default());
    let model = Arc::new(Projection::default());
    let subscriptions = crate::test_support::state_subscription::test_subscriptions();
    let mut changes = subscriptions.changes();
    let usecase =
        ProviderUsageUsecase::new(source.clone(), model.clone(), model.clone(), subscriptions);
    *source.0.lock().unwrap() = Some(("first".into(), usage(20.0)));
    // When
    usecase.observe_codex("session").await.unwrap();
    // Then
    assert_eq!(
        changes.try_recv().unwrap(),
        StateChangeSource::ProviderUsage
    );
    assert_eq!(usecase.snapshot()[0].windows[0].used_percent, 20.0);
    *source.0.lock().unwrap() = Some(("first".into(), usage(30.0)));
    usecase.observe_codex("session").await.unwrap();
    assert!(changes.try_recv().is_err());
    assert_eq!(usecase.snapshot()[0].windows[0].used_percent, 20.0);
    *source.0.lock().unwrap() = Some(("second".into(), usage(40.0)));
    usecase.observe_codex("session").await.unwrap();
    assert_eq!(usecase.snapshot()[0].windows[0].used_percent, 40.0);
    assert_eq!(
        changes.try_recv().unwrap(),
        StateChangeSource::ProviderUsage
    );
}

#[test]
fn test_使用率_claudeの未取得と不正入力では通知も状態変更もしない() {
    // Given
    let source = Arc::new(Source::default());
    let model = Arc::new(Projection::default());
    let subscriptions = crate::test_support::state_subscription::test_subscriptions();
    let mut changes = subscriptions.changes();
    let usecase = ProviderUsageUsecase::new(source.clone(), model.clone(), model, subscriptions);
    // When / Then
    usecase.receive_claude(b"missing").unwrap();
    assert!(usecase.receive_claude(b"invalid").is_err());
    assert!(usecase.snapshot().is_empty());
    assert!(changes.try_recv().is_err());
    *source.0.lock().unwrap() = Some(("value".into(), usage(70.0)));
    usecase.receive_claude(b"valid").unwrap();
    assert_eq!(
        changes.try_recv().unwrap(),
        StateChangeSource::ProviderUsage
    );
    assert_eq!(usecase.snapshot()[0].windows[0].used_percent, 70.0);
}

#[tokio::test]
async fn test_使用率_再起動時のrolloutは基準だけ読み次の観測まで表示しない() {
    // Given
    let source = Arc::new(Source::default());
    *source.0.lock().unwrap() = Some(("old".into(), usage(20.0)));
    let model = Arc::new(Projection::default());
    let usecase = ProviderUsageUsecase::new(
        source.clone(),
        model.clone(),
        model,
        crate::test_support::state_subscription::test_subscriptions(),
    );
    // When
    usecase.seed_codex_session("session").await.unwrap();
    usecase.poll_codex(&["session".into()]).await;
    // Then
    assert!(usecase.snapshot().is_empty());
    *source.0.lock().unwrap() = Some(("new".into(), usage(30.0)));
    usecase.poll_codex(&["session".into()]).await;
    assert_eq!(usecase.snapshot()[0].windows[0].used_percent, 30.0);
}

#[tokio::test]
async fn test_使用率_一つのcodex読取失敗でも別のsessionの使用率を反映する() {
    // Given
    let source = Arc::new(Source::default());
    *source.0.lock().unwrap() = Some(("record".into(), usage(70.0)));
    let model = Arc::new(Projection::default());
    let usecase = ProviderUsageUsecase::new(
        source,
        model.clone(),
        model,
        crate::test_support::state_subscription::test_subscriptions(),
    );
    // When
    let failures = usecase
        .poll_codex(&["invalid".into(), "valid".into()])
        .await;
    // Then
    assert_eq!(failures.len(), 1);
    assert_eq!(usecase.snapshot()[0].windows[0].used_percent, 70.0);
}
