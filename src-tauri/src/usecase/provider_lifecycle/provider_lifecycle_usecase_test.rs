use std::sync::atomic::{AtomicU8, Ordering};

use tokio::sync::Notify;

use super::*;
use crate::domain::provider_lifecycle::{
    IssuedProviderLifecycleCredential, ProviderHookHealth, ProviderHookHealthRepository,
    ProviderHookHealthRepositoryError, ProviderLifecycleCapabilityHash,
    ProviderLifecycleUnavailableReason, VersionedProviderHookHealth,
};

#[derive(Default)]
struct FakeCredentials {
    next: AtomicU8,
}

impl ProviderLifecycleCredentialGateway for FakeCredentials {
    fn issue(&self) -> IssuedProviderLifecycleCredential {
        let sequence = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        IssuedProviderLifecycleCredential::new(
            format!("binding-{sequence}"),
            format!("capability-{sequence}"),
            ProviderLifecycleCapabilityHash::from_digest([sequence; 32]),
        )
    }

    fn hash(&self, capability: &str) -> ProviderLifecycleCapabilityHash {
        let sequence = capability
            .strip_prefix("capability-")
            .and_then(|value| value.parse::<u8>().ok())
            .unwrap_or(0);
        ProviderLifecycleCapabilityHash::from_digest([sequence; 32])
    }
}

#[derive(Default)]
struct RecordingEvents {
    batches: Mutex<Vec<Vec<ScopedProviderLifecycleEvent>>>,
    failure: Mutex<Option<ProviderLifecycleRepositoryError>>,
}

impl RecordingEvents {
    fn fail_with(&self, error: ProviderLifecycleRepositoryError) {
        *self.failure.lock().unwrap() = Some(error);
    }

    fn clear_failure(&self) {
        *self.failure.lock().unwrap() = None;
    }

    fn batch_count(&self) -> usize {
        self.batches.lock().unwrap().len()
    }
}

#[async_trait::async_trait]
impl ProviderLifecycleEventRepository for RecordingEvents {
    async fn append(
        &self,
        events: Vec<ScopedProviderLifecycleEvent>,
    ) -> Result<(), ProviderLifecycleRepositoryError> {
        if let Some(error) = self.failure.lock().unwrap().clone() {
            return Err(error);
        }
        self.batches.lock().unwrap().push(events);
        Ok(())
    }

    async fn load_scope(
        &self,
        scope: &ProviderLifecycleScope,
    ) -> Result<Vec<ScopedProviderLifecycleEvent>, ProviderLifecycleRepositoryError> {
        Ok(self
            .batches
            .lock()
            .unwrap()
            .iter()
            .flatten()
            .filter_map(|event| {
                let (event_scope, event) = event.clone().into_parts();
                (event_scope == *scope)
                    .then(|| ScopedProviderLifecycleEvent::new(event_scope, event))
            })
            .collect())
    }
}

struct ConcurrentEvents {
    blocked_entered: Notify,
    independent_entered: Notify,
    release_blocked: Notify,
}

impl ConcurrentEvents {
    fn new() -> Self {
        Self {
            blocked_entered: Notify::new(),
            independent_entered: Notify::new(),
            release_blocked: Notify::new(),
        }
    }
}

#[async_trait::async_trait]
impl ProviderLifecycleEventRepository for ConcurrentEvents {
    async fn append(
        &self,
        events: Vec<ScopedProviderLifecycleEvent>,
    ) -> Result<(), ProviderLifecycleRepositoryError> {
        let agent_session_id = events
            .into_iter()
            .next()
            .map(ScopedProviderLifecycleEvent::into_parts)
            .map(|(scope, _)| scope.agent_session_id().to_string())
            .unwrap();
        match agent_session_id.as_str() {
            "agent-blocked" => {
                self.blocked_entered.notify_one();
                self.release_blocked.notified().await;
            }
            "agent-independent" => self.independent_entered.notify_one(),
            _ => {}
        }
        Ok(())
    }
}

fn slot_id(value: &str) -> ProviderLifecycleSlotId {
    ProviderLifecycleSlotId::new(value).unwrap()
}

fn scope(session: &str) -> ProviderLifecycleScope {
    ProviderLifecycleScope::new(session).unwrap()
}

fn session_start(
    armed: &ArmedProviderLifecycle,
    provider_session_id: &str,
) -> ProviderLifecycleSignal {
    ProviderLifecycleSignal::session_started(
        armed.binding_id(),
        armed.provider(),
        armed.scope().clone(),
        provider_session_id,
        None,
    )
    .unwrap()
}

fn usecase() -> (
    ProviderLifecycleUsecase,
    Arc<RecordingEvents>,
    Arc<FakeCredentials>,
) {
    let events = Arc::new(RecordingEvents::default());
    let credentials = Arc::new(FakeCredentials::default());
    (
        ProviderLifecycleUsecase::new(credentials.clone(), events.clone()),
        events,
        credentials,
    )
}

#[tokio::test]
async fn test_providerライフサイクルusecase_永続化失敗時は直前bindingをcurrentに保つ() {
    let (usecase, events, _) = usecase();
    let slot = slot_id("slot-1");
    let previous = usecase
        .arm(slot.clone(), ProviderKind::Codex, scope("agent-previous"))
        .await
        .unwrap();
    events.fail_with(ProviderLifecycleRepositoryError::StorageUnavailable);

    assert_eq!(
        usecase
            .arm(slot.clone(), ProviderKind::Codex, scope("agent-current"),)
            .await,
        Err(ProviderLifecycleUsecaseError::StorageUnavailable)
    );
    events.clear_failure();

    assert_eq!(
        usecase
            .receive(
                &slot,
                previous.capability(),
                session_start(&previous, "provider-session-previous"),
            )
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Applied
    );
    assert_eq!(events.batch_count(), 2);
}

#[tokio::test]
async fn test_providerライフサイクルusecase_同一slotの再起動が旧bindingを拒否する() {
    let (usecase, events, _) = usecase();
    let slot = slot_id("slot-1");
    let previous = usecase
        .arm(slot.clone(), ProviderKind::Codex, scope("agent-previous"))
        .await
        .unwrap();
    usecase
        .arm(slot.clone(), ProviderKind::Codex, scope("agent-current"))
        .await
        .unwrap();

    assert_eq!(
        usecase
            .receive(
                &slot,
                previous.capability(),
                session_start(&previous, "provider-session-previous"),
            )
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Rejected(ProviderLifecycleRejection::BindingExpired)
    );
    assert_eq!(events.batch_count(), 2);
}

#[tokio::test]
async fn test_providerライフサイクルusecase_異なるslotは互いのbindingを失効させない() {
    let (usecase, _, _) = usecase();
    let first_slot = slot_id("slot-1");
    let second_slot = slot_id("slot-2");
    let first = usecase
        .arm(first_slot.clone(), ProviderKind::Claude, scope("agent-1"))
        .await
        .unwrap();
    usecase
        .arm(second_slot, ProviderKind::Claude, scope("agent-2"))
        .await
        .unwrap();

    assert_eq!(
        usecase
            .receive(
                &first_slot,
                first.capability(),
                session_start(&first, "provider-session-1"),
            )
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Applied
    );
}

#[tokio::test]
async fn test_providerライフサイクルusecase_一方のslot永続化中も別slotを処理する() {
    let events = Arc::new(ConcurrentEvents::new());
    let usecase = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(FakeCredentials::default()),
        events.clone(),
    ));
    let blocked = {
        let usecase = usecase.clone();
        tokio::spawn(async move {
            usecase
                .arm(
                    slot_id("slot-blocked"),
                    ProviderKind::Claude,
                    scope("agent-blocked"),
                )
                .await
        })
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        events.blocked_entered.notified(),
    )
    .await
    .unwrap();

    let independent = {
        let usecase = usecase.clone();
        tokio::spawn(async move {
            usecase
                .arm(
                    slot_id("slot-independent"),
                    ProviderKind::Codex,
                    scope("agent-independent"),
                )
                .await
        })
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        events.independent_entered.notified(),
    )
    .await
    .unwrap();
    assert!(independent.await.unwrap().is_ok());

    events.release_blocked.notify_one();
    assert!(blocked.await.unwrap().is_ok());
}

#[tokio::test]
async fn test_providerライフサイクルusecase_明示releaseでlive_slotを除去する() {
    let (usecase, _, _) = usecase();
    let slot = slot_id("slot-1");
    let armed = usecase
        .arm(slot.clone(), ProviderKind::Claude, scope("agent-1"))
        .await
        .unwrap();
    assert_eq!(usecase.live_slot_count().unwrap(), 1);

    assert_eq!(
        usecase.release(&slot, armed.binding_id()).await.unwrap(),
        ProviderLifecycleIngressResult::Applied
    );
    assert_eq!(usecase.live_slot_count().unwrap(), 0);
    assert_eq!(
        usecase
            .receive(
                &slot,
                armed.capability(),
                session_start(&armed, "provider-session-1"),
            )
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Rejected(ProviderLifecycleRejection::BindingNotActive)
    );
}

#[tokio::test]
async fn test_providerライフサイクルusecase_agent_session終了でscopeのbindingを全て失効する() {
    let (usecase, _, _) = usecase();
    let target_scope = scope("agent-1");
    let first = usecase
        .arm(
            slot_id("slot-1"),
            ProviderKind::Claude,
            target_scope.clone(),
        )
        .await
        .unwrap();
    let second = usecase
        .arm(
            slot_id("slot-2"),
            ProviderKind::Claude,
            target_scope.clone(),
        )
        .await
        .unwrap();
    usecase
        .arm(
            slot_id("slot-other"),
            ProviderKind::Codex,
            scope("agent-other"),
        )
        .await
        .unwrap();

    assert_eq!(usecase.release_scope(&target_scope).await.unwrap(), 2);
    assert_eq!(usecase.live_slot_count().unwrap(), 1);
    for armed in [first, second] {
        assert_eq!(
            usecase
                .receive(
                    armed.slot_id(),
                    armed.capability(),
                    session_start(&armed, "provider-session-1"),
                )
                .await
                .unwrap(),
            ProviderLifecycleIngressResult::Rejected(ProviderLifecycleRejection::BindingNotActive)
        );
    }
}

#[tokio::test]
async fn test_providerライフサイクルusecase_再起動後もscopeのbindingを失効する() {
    let events = Arc::new(RecordingEvents::default());
    let credentials = Arc::new(FakeCredentials::default());
    let before_restart = ProviderLifecycleUsecase::new(credentials.clone(), events.clone());
    let target_scope = scope("agent-restarted");
    before_restart
        .arm(
            slot_id("slot-before-restart"),
            ProviderKind::Codex,
            target_scope.clone(),
        )
        .await
        .unwrap();

    let after_restart = ProviderLifecycleUsecase::new(credentials, events.clone());
    assert_eq!(after_restart.release_scope(&target_scope).await.unwrap(), 1);

    let persisted = events.load_scope(&target_scope).await.unwrap();
    assert!(matches!(
        persisted.last().cloned().map(|event| event.into_parts().1),
        Some(ProviderLifecycleEvent::BindingExpired { .. })
    ));
    assert_eq!(after_restart.release_scope(&target_scope).await.unwrap(), 0);
}

#[test]
fn test_providerライフサイクルusecase_repository_errorをusecase_errorへ変換する() {
    for (repository_error, expected) in [
        (
            ProviderLifecycleRepositoryError::InvalidInput,
            ProviderLifecycleUsecaseError::InvalidInput,
        ),
        (
            ProviderLifecycleRepositoryError::StorageUnavailable,
            ProviderLifecycleUsecaseError::StorageUnavailable,
        ),
        (
            ProviderLifecycleRepositoryError::Corrupt,
            ProviderLifecycleUsecaseError::Corrupt,
        ),
    ] {
        assert_eq!(
            ProviderLifecycleUsecaseError::from(repository_error),
            expected
        );
    }
}

#[derive(Default)]
struct InMemoryHookHealthRepository {
    stored: Mutex<std::collections::HashMap<ProviderKind, VersionedProviderHookHealth>>,
}

#[async_trait::async_trait]
impl ProviderHookHealthRepository for InMemoryHookHealthRepository {
    async fn load(
        &self,
        provider: ProviderKind,
    ) -> Result<VersionedProviderHookHealth, ProviderHookHealthRepositoryError> {
        Ok(self
            .stored
            .lock()
            .unwrap()
            .get(&provider)
            .cloned()
            .unwrap_or_else(|| {
                VersionedProviderHookHealth::restored(ProviderHookHealth::new(provider), 0)
            }))
    }

    async fn save(
        &self,
        mut health: VersionedProviderHookHealth,
        _caller_request_id: &str,
    ) -> Result<VersionedProviderHookHealth, ProviderHookHealthRepositoryError> {
        let event_count = health.health_mut().take_uncommitted_events().len() as u64;
        let revision = health.revision() + event_count;
        let saved = VersionedProviderHookHealth::restored(health.into_health(), revision);
        self.stored
            .lock()
            .unwrap()
            .insert(saved.health().provider(), saved.clone());
        Ok(saved)
    }
}

#[tokio::test]
async fn test_provider_hook_health_usecase_異常を警告し後続session_startで解除する() {
    let repository = Arc::new(InMemoryHookHealthRepository::default());
    let usecase = ProviderHookHealthUsecase::new(repository);

    usecase
        .record_launch(ProviderKind::Codex, "launch-1", "launch-request-1")
        .await
        .unwrap();
    usecase
        .record_unavailable(
            ProviderKind::Codex,
            "launch-1",
            ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed,
            "warning-request-1",
        )
        .await
        .unwrap();
    assert_eq!(
        usecase.warnings().await.unwrap(),
        vec![ProviderHookHealthWarning {
            provider: ProviderKind::Codex,
            launch_id: "launch-1".to_string(),
            reason: ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed,
        }]
    );

    usecase
        .record_launch(ProviderKind::Codex, "launch-2", "launch-request-2")
        .await
        .unwrap();
    usecase
        .record_session_started(ProviderKind::Codex, "launch-2", "clear-request-1")
        .await
        .unwrap();
    assert!(usecase.warnings().await.unwrap().is_empty());
}

struct FixedHookDeliveryFailures {
    observations: Vec<ProviderHookHealthFailureObservation>,
}

#[async_trait::async_trait]
impl ProviderHookHealthFailureQuery for FixedHookDeliveryFailures {
    async fn list(
        &self,
        _limit: usize,
    ) -> Result<Vec<ProviderHookHealthFailureObservation>, ProviderHookHealthFailureQueryError>
    {
        Ok(self.observations.clone())
    }
}

#[tokio::test]
async fn test_provider_hook_health_read_local_api配送失敗を最新launchの警告へ反映する() {
    let repository = Arc::new(InMemoryHookHealthRepository::default());
    let health = Arc::new(ProviderHookHealthUsecase::new(repository));
    health
        .record_launch(
            ProviderKind::Claude,
            "launch-latest",
            "launch-latest-request",
        )
        .await
        .unwrap();
    let read = ProviderHookHealthReadUsecase::new(
        health,
        Arc::new(FixedHookDeliveryFailures {
            observations: vec![
                ProviderHookHealthFailureObservation {
                    provider: ProviderKind::Claude,
                    launch_id: "launch-old".to_string(),
                    reason: ProviderLifecycleUnavailableReason::LocalApiUnavailable,
                },
                ProviderHookHealthFailureObservation {
                    provider: ProviderKind::Claude,
                    launch_id: "launch-latest".to_string(),
                    reason: ProviderLifecycleUnavailableReason::LocalApiUnavailable,
                },
            ],
        }),
    );

    assert_eq!(
        read.warnings().await.unwrap(),
        vec![ProviderHookHealthWarning {
            provider: ProviderKind::Claude,
            launch_id: "launch-latest".to_string(),
            reason: ProviderLifecycleUnavailableReason::LocalApiUnavailable,
        }]
    );
}

#[tokio::test]
async fn test_provider_hook_health_正常session_start後の同一launch欠落報告を無視する() {
    let repository = Arc::new(InMemoryHookHealthRepository::default());
    let health = ProviderHookHealthUsecase::new(repository);
    health
        .record_launch(ProviderKind::Claude, "launch-1", "launch-1-request")
        .await
        .unwrap();

    health
        .record_unavailable(
            ProviderKind::Claude,
            "launch-1",
            ProviderLifecycleUnavailableReason::SessionStartDeadlineExceeded,
            "missing-1-request",
        )
        .await
        .unwrap();
    assert_eq!(health.warnings().await.unwrap().len(), 1);

    health
        .record_session_started(ProviderKind::Claude, "launch-1", "session-started-request")
        .await
        .unwrap();
    health
        .record_unavailable(
            ProviderKind::Claude,
            "launch-1",
            ProviderLifecycleUnavailableReason::SessionStartDeadlineExceeded,
            "missing-after-success-request",
        )
        .await
        .unwrap();
    assert!(health.warnings().await.unwrap().is_empty());
}
