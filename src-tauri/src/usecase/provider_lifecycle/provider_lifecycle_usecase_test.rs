use std::sync::atomic::{AtomicU8, Ordering};

use tokio::sync::Notify;

use super::*;
use crate::domain::provider_lifecycle::{
    IssuedProviderLifecycleCredential, ProviderLifecycleCapabilityHash,
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

fn scope(session: &str, node_execution: &str, attempt: u32) -> ProviderLifecycleScope {
    ProviderLifecycleScope::new(session, "workflow-execution-1", node_execution, attempt).unwrap()
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
        .arm(
            slot.clone(),
            ProviderKind::Codex,
            scope("agent-previous", "node-previous", 1),
        )
        .await
        .unwrap();
    events.fail_with(ProviderLifecycleRepositoryError::StorageUnavailable);

    assert_eq!(
        usecase
            .arm(
                slot.clone(),
                ProviderKind::Codex,
                scope("agent-current", "node-current", 2),
            )
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
async fn test_providerライフサイクルusecase_同一slotの新attemptが旧bindingを拒否する() {
    let (usecase, events, _) = usecase();
    let slot = slot_id("slot-1");
    let previous = usecase
        .arm(
            slot.clone(),
            ProviderKind::Codex,
            scope("agent-previous", "node-previous", 1),
        )
        .await
        .unwrap();
    usecase
        .arm(
            slot.clone(),
            ProviderKind::Codex,
            scope("agent-current", "node-current", 2),
        )
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
        .arm(
            first_slot.clone(),
            ProviderKind::Claude,
            scope("agent-1", "node-1", 1),
        )
        .await
        .unwrap();
    usecase
        .arm(
            second_slot,
            ProviderKind::Claude,
            scope("agent-2", "node-2", 1),
        )
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
                    scope("agent-blocked", "node-blocked", 1),
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
                    scope("agent-independent", "node-independent", 1),
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
        .arm(
            slot.clone(),
            ProviderKind::Claude,
            scope("agent-1", "node-1", 1),
        )
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
