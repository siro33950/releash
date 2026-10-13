use super::*;
use crate::domain::provider_usage::{CodexUsageRecord, ProviderUsage, UsageWindow};
use std::sync::Mutex;

#[derive(Default)]
struct Source(Mutex<Option<CodexUsageRecord>>);
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
            .map(|record| record.usage.clone()))
    }
    async fn codex(&self, session: &str) -> Result<Option<CodexUsageRecord>, UsageError> {
        if session == "invalid" {
            return Err(UsageError::Invalid);
        }
        Ok(self.0.lock().unwrap().clone())
    }
}
#[derive(Default)]
struct Projection(
    Mutex<Vec<ProviderUsage>>,
    Mutex<std::collections::HashMap<String, CodexUsageObservation>>,
);
struct Clock;
impl ProviderUsageClock for Clock {
    fn now(&self) -> std::time::SystemTime {
        std::time::UNIX_EPOCH + std::time::Duration::from_micros(100_123_400)
    }
}
fn record(id: &str, percent: f64, seconds: u64) -> CodexUsageRecord {
    CodexUsageRecord {
        id: id.into(),
        usage: usage(percent),
        timestamp: std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds),
    }
}

impl ProviderUsageQueryService for Projection {
    fn snapshot(&self) -> Vec<ProviderUsageDto> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|v| ProviderUsageDto {
                provider: v.provider,
                plan: v.plan.clone(),
                windows: v
                    .windows
                    .iter()
                    .map(|w| UsageWindowDto {
                        used_percent: w.used_percent(),
                        window_minutes: w.window_minutes(),
                        resets_at: w.resets_at(),
                        warning: w.warning(),
                    })
                    .collect(),
            })
            .collect()
    }
}
impl ProviderUsageRepository for Projection {
    fn replace(&self, usage: ProviderUsage) {
        *self.0.lock().unwrap() = vec![usage];
    }
    fn start_observation(&self, o: CodexUsageObservation) {
        self.1.lock().unwrap().insert(o.session.clone(), o);
    }
    fn has_observation(&self, id: &str) -> bool {
        self.1.lock().unwrap().contains_key(id)
    }
    fn accept_record(
        &self,
        id: &str,
        record: &crate::domain::provider_usage::CodexUsageRecord,
    ) -> bool {
        self.1
            .lock()
            .unwrap()
            .get_mut(id)
            .is_some_and(|observation| observation.accept(record))
    }
    fn retain_observations(&self, ids: &[String], poll_started_at: std::time::SystemTime) {
        self.1
            .lock()
            .unwrap()
            .retain(|_, observation| observation.retain_for_poll(ids, poll_started_at));
    }
}
fn usage(percent: f64) -> ProviderUsage {
    ProviderUsage {
        provider: ProviderKind::Codex,
        windows: vec![UsageWindow::new(percent, 300, 123).unwrap()],
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
    let usecase = ProviderUsageUsecase::new(
        source.clone(),
        model.clone(),
        model.clone(),
        Arc::new(Clock),
        subscriptions,
    );
    *source.0.lock().unwrap() = Some(record("first", 20.0, 101));
    usecase.start_codex_session("session");
    // When
    usecase.observe_codex("session").await.unwrap();
    // Then
    assert_eq!(
        changes.try_recv().unwrap(),
        StateChangeSource::ProviderUsage
    );
    assert_eq!(usecase.snapshot()[0].windows[0].used_percent, 20.0);
    *source.0.lock().unwrap() = Some(record("first", 30.0, 101));
    usecase.observe_codex("session").await.unwrap();
    assert!(changes.try_recv().is_err());
    assert_eq!(usecase.snapshot()[0].windows[0].used_percent, 20.0);
    *source.0.lock().unwrap() = Some(record("second", 40.0, 101));
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
    let usecase = ProviderUsageUsecase::new(
        source.clone(),
        model.clone(),
        model,
        Arc::new(Clock),
        subscriptions,
    );
    // When / Then
    usecase.receive_claude(b"missing").unwrap();
    assert!(usecase.receive_claude(b"invalid").is_err());
    assert!(usecase.snapshot().is_empty());
    assert!(changes.try_recv().is_err());
    *source.0.lock().unwrap() = Some(record("value", 70.0, 101));
    usecase.receive_claude(b"valid").unwrap();
    assert_eq!(
        changes.try_recv().unwrap(),
        StateChangeSource::ProviderUsage
    );
    assert_eq!(usecase.snapshot()[0].windows[0].used_percent, 70.0);
}

#[tokio::test]
async fn test_使用率_開始境界前を除外し同じミリ秒の新しい記録を保存して通知する() {
    // Given
    let source = Arc::new(Source::default());
    let mut old = record("old", 20.0, 100);
    old.timestamp += std::time::Duration::from_millis(122);
    *source.0.lock().unwrap() = Some(old);
    let model = Arc::new(Projection::default());
    let subscriptions = crate::test_support::state_subscription::test_subscriptions();
    let mut changes = subscriptions.changes();
    let usecase = ProviderUsageUsecase::new(
        source.clone(),
        model.clone(),
        model,
        Arc::new(Clock),
        subscriptions,
    );
    // When / Then
    usecase.start_codex_session("session");
    usecase.observe_codex("session").await.unwrap();
    assert!(usecase.snapshot().is_empty());
    assert!(changes.try_recv().is_err());
    let mut new = record("new", 30.0, 100);
    new.timestamp += std::time::Duration::from_millis(123);
    new.usage.windows = vec![UsageWindow::new(30.0, 300, 456).unwrap()];
    new.usage.plan = Some("pro".into());
    *source.0.lock().unwrap() = Some(new);
    usecase.observe_codex("session").await.unwrap();
    let snapshot = usecase.snapshot();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].windows[0].used_percent, 30.0);
    assert_eq!(snapshot[0].windows[0].resets_at, 456);
    assert_eq!(snapshot[0].plan.as_deref(), Some("pro"));
    assert_eq!(
        changes.try_recv().unwrap(),
        StateChangeSource::ProviderUsage
    );
    usecase.observe_codex("session").await.unwrap();
    assert_eq!(usecase.snapshot(), snapshot);
    assert!(changes.try_recv().is_err());
}

#[tokio::test]
async fn test_使用率_一つのcodex読取失敗でも別のsessionの使用率を反映する() {
    // Given
    let source = Arc::new(Source::default());
    *source.0.lock().unwrap() = Some(record("record", 70.0, 101));
    let model = Arc::new(Projection::default());
    let usecase = ProviderUsageUsecase::new(
        source,
        model.clone(),
        model,
        Arc::new(Clock),
        crate::test_support::state_subscription::test_subscriptions(),
    );
    usecase.start_codex_session("invalid");
    usecase.start_codex_session("valid");
    // When
    let failures = usecase
        .poll_codex(&["invalid".into(), "valid".into()], usecase.clock.now())
        .await;
    // Then
    assert_eq!(failures.len(), 1);
    assert_eq!(usecase.snapshot()[0].windows[0].used_percent, 70.0);
}
