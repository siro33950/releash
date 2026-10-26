use super::*;
use crate::domain::agent_session::services::AgentSessionService;
use crate::domain::failure::{StorageFailure, StorageFailureSource, TechnicalFailureNature};
use crate::usecase::agent_session::test_helpers::{
    FakeThreadRepository, RecordingTerminal, RecordingTerminalInput,
};

#[test]
fn test_所有済みセッション_ingressの両経路で分類を保持する() {
    // Given
    for (repository, usecase, expected) in [
        (
            AgentSessionRepositoryError::ProviderSessionAlreadyOwned {
                agent_session_id: "owner".into(),
            },
            AgentSessionUsecaseError::ProviderSessionAlreadyOwned {
                agent_session_id: "owner".into(),
            },
            ProviderLifecycleIngressUsecaseError::Store(StorageFailure {
                nature: TechnicalFailureNature::Other,
                source: StorageFailureSource::AgentSession(Box::new(
                    AgentSessionRepositoryError::ProviderSessionAlreadyOwned {
                        agent_session_id: "owner".into(),
                    },
                )),
                context: None,
            }),
        ),
        (
            AgentSessionRepositoryError::Conflict,
            AgentSessionUsecaseError::Conflict,
            ProviderLifecycleIngressUsecaseError::Conflict,
        ),
        (
            AgentSessionRepositoryError::Unavailable,
            AgentSessionUsecaseError::Unavailable,
            ProviderLifecycleIngressUsecaseError::StorageUnavailable,
        ),
    ] {
        // When / Then
        assert_eq!(map_session_repository_error(repository), expected);
        assert_eq!(map_session_error(usecase), expected);
    }
}

use super::super::test_helpers::*;
use crate::domain::agent_session::aggregates::AgentSession;
use crate::domain::provider_lifecycle::{ProviderKind, ProviderLifecycleScope};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::agent_session::test_helpers::session_location;
use crate::usecase::provider_lifecycle::ingress::{ProviderPayloadInput, ProviderPayloadReceiver};
use crate::usecase::test_helpers::TestCredentials as LocalProviderLifecycleCredentialGateway;
use std::sync::{Arc, Mutex};
struct PayloadInterpreter {
    calls: Mutex<Vec<(ProviderKind, String, ProviderLifecycleScope, Vec<u8>)>>,
    result: Result<
        crate::domain::provider_lifecycle::ProviderPayloadInterpretation,
        crate::domain::provider_lifecycle::ProviderPayloadError,
    >,
}
impl crate::domain::provider_lifecycle::ProviderPayloadInterpreter for PayloadInterpreter {
    fn interpret(
        &self,
        provider: crate::domain::provider_lifecycle::ProviderKind,
        binding_id: &str,
        scope: crate::domain::provider_lifecycle::ProviderLifecycleScope,
        payload: &[u8],
    ) -> Result<
        crate::domain::provider_lifecycle::ProviderPayloadInterpretation,
        crate::domain::provider_lifecycle::ProviderPayloadError,
    > {
        self.calls
            .lock()
            .unwrap()
            .push((provider, binding_id.into(), scope, payload.to_vec()));
        self.result.clone()
    }
}
#[tokio::test]
async fn test_生payload受付_信号を既存入口へ渡しsubagentと解釈失敗は記録しない() {
    use crate::domain::provider_lifecycle::{
        ProviderKind, ProviderLifecycleScope, ProviderPayloadError,
        ProviderPayloadInterpretation as Parsed,
    };
    // Given
    let scope = ProviderLifecycleScope::new("session").unwrap();
    let slot = ProviderLifecycleSlotId::new("slot").unwrap();
    let session = AgentSession::create(
        "session",
        WorkspaceIdentity::new("/repo"),
        "/repo",
        ProviderKind::Claude,
        session_location("session"),
    )
    .unwrap();
    let sessions = Arc::new(MemoryAgentSessions {
        stored: Mutex::new(VersionedAgentSession::restored(session, 1)),
        fail_save: false,
        fail_activity_save: false,
        save_observed: None,
    });
    let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        Arc::new(MemoryLifecycleEvents),
    ));
    let armed = lifecycle
        .arm(slot.clone(), ProviderKind::Claude, scope.clone())
        .await
        .unwrap();
    let signal = ProviderLifecycleSignal::session_started(
        armed.binding_id(),
        ProviderKind::Claude,
        scope.clone(),
        "provider-session",
        None,
    )
    .unwrap();
    let make_ingress = |interpreter: Arc<PayloadInterpreter>| {
        ProviderLifecycleIngressUsecase::new(
            interpreter,
            Arc::new(crate::usecase::test_helpers::TestIdentity),
            lifecycle.clone(),
            Arc::new(AgentSessionUsecase::new(
                sessions.clone(),
                Arc::new(FakeThreadRepository::default()),
                AgentSessionService::new(
                    Arc::new(RecordingTerminal::default()),
                    Arc::new(RecordingTerminalInput::default()),
                ),
            )),
            Arc::new(ProviderHookHealthUsecase::new(Arc::new(
                MemoryHookHealth::default(),
            ))),
            (sessions.clone(), Arc::new(MemoryWorkflowStops::default())),
            crate::test_support::state_subscription::test_subscriptions(),
        )
    };
    let interpreter = Arc::new(PayloadInterpreter {
        calls: Mutex::new(Vec::new()),
        result: Ok(Parsed::Signal(signal)),
    });
    let ingress = make_ingress(interpreter.clone());
    let input = || ProviderPayloadInput {
        provider: ProviderKind::Claude,
        binding_id: armed.binding_id(),
        scope: scope.clone(),
        payload: b"payload",
    };
    // When / Then
    for expected in [
        ProviderLifecycleIngressResult::Applied,
        ProviderLifecycleIngressResult::Duplicate,
    ] {
        let (result, started) = ingress
            .receive_payload(&slot, armed.capability(), input())
            .await
            .unwrap();
        assert_eq!((result, started), (expected, true));
    }
    assert_eq!(
        *interpreter.calls.lock().unwrap(),
        vec![
            (
                ProviderKind::Claude,
                armed.binding_id().into(),
                scope.clone(),
                b"payload".to_vec()
            );
            2
        ]
    );
    assert_eq!(
        sessions
            .stored
            .lock()
            .unwrap()
            .session()
            .provider_session_id(),
        Some("provider-session")
    );
    let revision = sessions.stored.lock().unwrap().revision();
    let ingress = make_ingress(Arc::new(PayloadInterpreter {
        calls: Mutex::new(Vec::new()),
        result: Ok(Parsed::Subagent),
    }));
    let (result, started) = ingress
        .receive_payload(&slot, armed.capability(), input())
        .await
        .unwrap();
    assert_eq!(
        (result, started),
        (ProviderLifecycleIngressResult::Ignored, false)
    );
    for failure in [
        ProviderPayloadError::InvalidPayload,
        ProviderPayloadError::UnsupportedEvent("unknown".into()),
        ProviderPayloadError::InvalidSignal(
            crate::domain::provider_lifecycle::ProviderLifecycleInputError::Empty("binding_id"),
        ),
    ] {
        // When
        let ingress = make_ingress(Arc::new(PayloadInterpreter {
            calls: Mutex::new(Vec::new()),
            result: Err(failure.clone()),
        }));
        let error = ingress
            .receive_payload(&slot, armed.capability(), input())
            .await
            .err()
            .unwrap();
        // Then
        assert_eq!(
            error,
            ProviderLifecycleIngressUsecaseError::Payload(failure.clone())
        );
        assert_eq!(error.to_string(), failure.to_string());
    }
    assert_eq!(sessions.stored.lock().unwrap().revision(), revision);
}

#[derive(Default)]
struct UsageSource {
    record: Mutex<String>,
    fail: bool,
    reads: std::sync::atomic::AtomicUsize,
}
#[async_trait::async_trait]
impl crate::domain::provider_usage::ProviderUsageSource for UsageSource {
    fn claude(
        &self,
        payload: &[u8],
    ) -> Result<
        Option<crate::domain::provider_usage::ProviderUsage>,
        crate::domain::provider_usage::UsageError,
    > {
        Ok(Some(crate::domain::provider_usage::ProviderUsage {
            provider: ProviderKind::Claude,
            windows: vec![crate::domain::provider_usage::UsageWindow::new(
                if payload == b"updated" { 90.0 } else { 70.0 },
                300,
                123,
            )
            .unwrap()],
            plan: None,
        }))
    }
    async fn codex(
        &self,
        _: &str,
    ) -> Result<
        Option<crate::domain::provider_usage::CodexUsageRecord>,
        crate::domain::provider_usage::UsageError,
    > {
        self.reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if self.fail {
            return Err(crate::domain::provider_usage::UsageError::from(
                crate::domain::failure::TechnicalFailure {
                    nature: TechnicalFailureNature::TimedOut,
                    message: "usage unavailable".into(),
                },
            ));
        }
        let record = self.record.lock().unwrap().clone();
        Ok(Some(crate::domain::provider_usage::CodexUsageRecord {
            id: record.clone(),
            timestamp: std::time::UNIX_EPOCH
                + std::time::Duration::from_secs(if record == "old" { 99 } else { 101 }),
            usage: crate::domain::provider_usage::ProviderUsage {
                provider: ProviderKind::Codex,
                windows: vec![
                    crate::domain::provider_usage::UsageWindow::new(70.0, 300, 123).unwrap(),
                ],
                plan: Some("pro".into()),
            },
        }))
    }
}
use crate::domain::provider_usage::{
    CodexUsageObservation, ProviderUsage, ProviderUsageClock, ProviderUsageRepository,
};
use crate::usecase::provider_usage::{ProviderUsageDto, ProviderUsageQueryService, UsageWindowDto};
#[derive(Default)]
struct UsageProjection(
    Mutex<Vec<ProviderUsage>>,
    Mutex<std::collections::HashMap<String, CodexUsageObservation>>,
);
struct Clock;
impl ProviderUsageClock for Clock {
    fn now(&self) -> std::time::SystemTime {
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(100)
    }
}
impl ProviderUsageQueryService for UsageProjection {
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
impl ProviderUsageRepository for UsageProjection {
    fn replace(&self, usage: ProviderUsage) {
        *self.0.lock().unwrap() = vec![usage];
    }
}
impl crate::domain::provider_usage::CodexUsageObservationRepository for UsageProjection {
    fn start_observation(&self, o: CodexUsageObservation) {
        self.1.lock().unwrap().entry(o.session.clone()).or_insert(o);
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
#[tokio::test]
async fn test_使用率監視_受理したcodexの開始時刻から現行bindingを監視する() {
    use crate::domain::provider_lifecycle::ProviderPayloadInterpretation;
    // Given
    let scope = ProviderLifecycleScope::new("session").unwrap();
    let slot = ProviderLifecycleSlotId::new("slot").unwrap();
    let session = AgentSession::create(
        "session",
        WorkspaceIdentity::new("/repo"),
        "/repo",
        ProviderKind::Codex,
        session_location("session"),
    )
    .unwrap();
    let sessions = Arc::new(MemoryAgentSessions {
        stored: Mutex::new(VersionedAgentSession::restored(session, 1)),
        fail_save: false,
        fail_activity_save: false,
        save_observed: None,
    });
    let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        Arc::new(MemoryLifecycleEvents),
    ));
    let armed = lifecycle
        .arm(slot.clone(), ProviderKind::Codex, scope.clone())
        .await
        .unwrap();
    let source = Arc::new(UsageSource::default());
    *source.record.lock().unwrap() = "old".into();
    let projection = Arc::new(UsageProjection::default());
    let clock = Arc::new(ChangingClock(std::sync::atomic::AtomicU64::new(100)));
    let usage = Arc::new(crate::usecase::provider_usage::ProviderUsageUsecase::new(
        source.clone(),
        projection.clone(),
        projection.clone(),
        projection.clone(),
        clock.clone(),
        crate::test_support::state_subscription::test_subscriptions(),
    ));
    let interpreter = Arc::new(PayloadInterpreter {
        calls: Mutex::new(Vec::new()),
        result: Ok(ProviderPayloadInterpretation::Signal(
            ProviderLifecycleSignal::session_started(
                armed.binding_id(),
                ProviderKind::Codex,
                scope.clone(),
                "provider-session",
                None,
            )
            .unwrap(),
        )),
    });
    let ingress = ProviderLifecycleIngressUsecase::new(
        interpreter,
        Arc::new(crate::usecase::test_helpers::TestIdentity),
        lifecycle.clone(),
        Arc::new(AgentSessionUsecase::new(
            sessions.clone(),
            Arc::new(FakeThreadRepository::default()),
            AgentSessionService::new(
                Arc::new(RecordingTerminal::default()),
                Arc::new(RecordingTerminalInput::default()),
            ),
        )),
        Arc::new(ProviderHookHealthUsecase::new(Arc::new(
            MemoryHookHealth::default(),
        ))),
        (sessions, Arc::new(MemoryWorkflowStops::default())),
        crate::test_support::state_subscription::test_subscriptions(),
    )
    .with_usage(usage.clone());
    let input = || ProviderPayloadInput {
        provider: ProviderKind::Codex,
        binding_id: armed.binding_id(),
        scope: scope.clone(),
        payload: b"start",
    };
    // When / Then
    assert!(matches!(
        ingress
            .receive_payload(&slot, "invalid", input())
            .await
            .unwrap()
            .0,
        ProviderLifecycleIngressResult::Rejected(_)
    ));
    assert_eq!(source.reads.load(std::sync::atomic::Ordering::Relaxed), 0);
    let (result, started) = ingress
        .receive_payload(&slot, armed.capability(), input())
        .await
        .unwrap();
    assert_eq!(
        (result, started),
        (ProviderLifecycleIngressResult::Applied, true)
    );
    clock.0.store(200, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        ingress
            .receive_payload(&slot, armed.capability(), input())
            .await
            .unwrap()
            .0,
        ProviderLifecycleIngressResult::Duplicate
    );
    {
        let observation = projection.1.lock().unwrap();
        assert!(!observation["provider-session"].retain_for_poll(
            &[],
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(150)
        ));
    }
    assert_eq!(source.reads.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert!(usage
        .poll_active_codex(&lifecycle)
        .await
        .unwrap()
        .is_empty());
    assert!(usage.snapshot().is_empty());
    *source.record.lock().unwrap() = "new".into();
    usage.poll_active_codex(&lifecycle).await.unwrap();
    assert_eq!(usage.snapshot()[0].plan.as_deref(), Some("pro"));
    lifecycle.release(&slot, armed.binding_id()).await.unwrap();
    let reads = source.reads.load(std::sync::atomic::Ordering::Relaxed);
    usage.poll_active_codex(&lifecycle).await.unwrap();
    assert_eq!(
        source.reads.load(std::sync::atomic::Ordering::Relaxed),
        reads
    );
}

#[tokio::test]
async fn test_使用率読取失敗_session開始は読取を待たず適用し監視時に失敗を返す() {
    use crate::domain::provider_lifecycle::ProviderPayloadInterpretation;
    // Given
    let scope = ProviderLifecycleScope::new("session").unwrap();
    let slot = ProviderLifecycleSlotId::new("slot").unwrap();
    let session = AgentSession::create(
        "session",
        WorkspaceIdentity::new("/repo"),
        "/repo",
        ProviderKind::Codex,
        session_location("session"),
    )
    .unwrap();
    let sessions = Arc::new(MemoryAgentSessions {
        stored: Mutex::new(VersionedAgentSession::restored(session, 1)),
        fail_save: false,
        fail_activity_save: false,
        save_observed: None,
    });
    let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        Arc::new(MemoryLifecycleEvents),
    ));
    let armed = lifecycle
        .arm(slot.clone(), ProviderKind::Codex, scope.clone())
        .await
        .unwrap();
    let source = Arc::new(UsageSource {
        fail: true,
        ..Default::default()
    });
    *source.record.lock().unwrap() = "old".into();
    let projection = Arc::new(UsageProjection::default());
    let usage = Arc::new(crate::usecase::provider_usage::ProviderUsageUsecase::new(
        source.clone(),
        projection.clone(),
        projection.clone(),
        projection,
        Arc::new(Clock),
        crate::test_support::state_subscription::test_subscriptions(),
    ));
    let interpreter = Arc::new(PayloadInterpreter {
        calls: Mutex::new(Vec::new()),
        result: Ok(ProviderPayloadInterpretation::Signal(
            ProviderLifecycleSignal::session_started(
                armed.binding_id(),
                ProviderKind::Codex,
                scope.clone(),
                "provider-session",
                None,
            )
            .unwrap(),
        )),
    });
    let ingress = ProviderLifecycleIngressUsecase::new(
        interpreter,
        Arc::new(crate::usecase::test_helpers::TestIdentity),
        lifecycle.clone(),
        Arc::new(AgentSessionUsecase::new(
            sessions.clone(),
            Arc::new(FakeThreadRepository::default()),
            AgentSessionService::new(
                Arc::new(RecordingTerminal::default()),
                Arc::new(RecordingTerminalInput::default()),
            ),
        )),
        Arc::new(ProviderHookHealthUsecase::new(Arc::new(
            MemoryHookHealth::default(),
        ))),
        (sessions, Arc::new(MemoryWorkflowStops::default())),
        crate::test_support::state_subscription::test_subscriptions(),
    )
    .with_usage(usage.clone());
    let input = || ProviderPayloadInput {
        provider: ProviderKind::Codex,
        binding_id: armed.binding_id(),
        scope: scope.clone(),
        payload: b"start",
    };
    // When
    let (result, started) = ingress
        .receive_payload(&slot, armed.capability(), input())
        .await
        .unwrap();
    // Then
    assert_eq!(
        (result, started),
        (ProviderLifecycleIngressResult::Applied, true)
    );
    assert_eq!(source.reads.load(std::sync::atomic::Ordering::Relaxed), 0);
    let failures = usage.poll_active_codex(&lifecycle).await.unwrap();
    match &failures[0] {
        crate::domain::provider_usage::UsageError::Technical(error) => {
            assert_eq!(error.nature, TechnicalFailureNature::TimedOut);
            assert_eq!(error.message, "usage unavailable");
        }
        error => panic!("unexpected usage failure: {error}"),
    }
    assert_eq!(
        lifecycle
            .provider_sessions(ProviderKind::Codex)
            .await
            .unwrap(),
        vec!["provider-session"]
    );
}

struct ChangingClock(std::sync::atomic::AtomicU64);
impl ProviderUsageClock for ChangingClock {
    fn now(&self) -> std::time::SystemTime {
        std::time::UNIX_EPOCH
            + std::time::Duration::from_secs(self.0.load(std::sync::atomic::Ordering::Relaxed))
    }
}

#[tokio::test]
async fn test_使用率受付_ingressで資格を照合し拒否した要求では値を変えない() {
    // Given
    let scope = ProviderLifecycleScope::new("session").unwrap();
    let slot = ProviderLifecycleSlotId::new("slot").unwrap();
    let session = AgentSession::create(
        "session",
        WorkspaceIdentity::new("/repo"),
        "/repo",
        ProviderKind::Claude,
        session_location("session"),
    )
    .unwrap();
    let sessions = Arc::new(MemoryAgentSessions {
        stored: Mutex::new(VersionedAgentSession::restored(session, 1)),
        fail_save: false,
        fail_activity_save: false,
        save_observed: None,
    });
    let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        Arc::new(MemoryLifecycleEvents),
    ));
    let armed = lifecycle
        .arm(slot.clone(), ProviderKind::Claude, scope.clone())
        .await
        .unwrap();
    let projection = Arc::new(UsageProjection::default());
    let usage = Arc::new(crate::usecase::provider_usage::ProviderUsageUsecase::new(
        Arc::new(UsageSource::default()),
        projection.clone(),
        projection.clone(),
        projection,
        Arc::new(Clock),
        crate::test_support::state_subscription::test_subscriptions(),
    ));
    let ingress = ProviderLifecycleIngressUsecase::new(
        Arc::new(PayloadInterpreter {
            calls: Mutex::new(Vec::new()),
            result: Ok(crate::domain::provider_lifecycle::ProviderPayloadInterpretation::Subagent),
        }),
        Arc::new(crate::usecase::test_helpers::TestIdentity),
        lifecycle,
        Arc::new(AgentSessionUsecase::new(
            sessions.clone(),
            Arc::new(FakeThreadRepository::default()),
            AgentSessionService::new(
                Arc::new(RecordingTerminal::default()),
                Arc::new(RecordingTerminalInput::default()),
            ),
        )),
        Arc::new(ProviderHookHealthUsecase::new(Arc::new(
            MemoryHookHealth::default(),
        ))),
        (sessions, Arc::new(MemoryWorkflowStops::default())),
        crate::test_support::state_subscription::test_subscriptions(),
    );
    // When / Then
    assert!(matches!(
        ingress
            .receive_usage(
                &slot,
                armed.capability(),
                armed.binding_id(),
                &scope,
                b"valid"
            )
            .await,
        Err(ProviderLifecycleIngressUsecaseError::StorageUnavailable)
    ));
    assert!(usage.snapshot().is_empty());
    let ingress = ingress.with_usage(usage.clone());
    assert_eq!(
        ingress
            .receive_usage(
                &slot,
                armed.capability(),
                armed.binding_id(),
                &scope,
                b"valid",
            )
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Applied
    );
    let saved = usage.snapshot();
    assert_eq!(saved[0].windows[0].used_percent, 70.0);
    assert_eq!(
        ingress
            .receive_usage(&slot, "invalid", armed.binding_id(), &scope, b"updated",)
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Rejected(
            crate::domain::provider_lifecycle::ProviderLifecycleRejection::InvalidCapability
        )
    );
    assert_eq!(usage.snapshot(), saved);
}

#[tokio::test]
async fn test_使用率監視_hook保存失敗後の再送で欠けた観測を復旧する() {
    use crate::domain::provider_lifecycle::ProviderPayloadInterpretation;
    // Given
    let scope = ProviderLifecycleScope::new("session").unwrap();
    let slot = ProviderLifecycleSlotId::new("slot").unwrap();
    let session = AgentSession::create(
        "session",
        WorkspaceIdentity::new("/repo"),
        "/repo",
        ProviderKind::Codex,
        session_location("session"),
    )
    .unwrap();
    let sessions = Arc::new(MemoryAgentSessions {
        stored: Mutex::new(VersionedAgentSession::restored(session, 1)),
        fail_save: false,
        fail_activity_save: false,
        save_observed: None,
    });
    let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        Arc::new(MemoryLifecycleEvents),
    ));
    let armed = lifecycle
        .arm(slot.clone(), ProviderKind::Codex, scope.clone())
        .await
        .unwrap();
    let source = Arc::new(UsageSource::default());
    *source.record.lock().unwrap() = "old".into();
    let projection = Arc::new(UsageProjection::default());
    let clock = Arc::new(ChangingClock(std::sync::atomic::AtomicU64::new(100)));
    let usage = Arc::new(crate::usecase::provider_usage::ProviderUsageUsecase::new(
        source.clone(),
        projection.clone(),
        projection.clone(),
        projection.clone(),
        clock.clone(),
        crate::test_support::state_subscription::test_subscriptions(),
    ));
    let interpreter = Arc::new(PayloadInterpreter {
        calls: Mutex::new(Vec::new()),
        result: Ok(ProviderPayloadInterpretation::Signal(
            ProviderLifecycleSignal::session_started(
                armed.binding_id(),
                ProviderKind::Codex,
                scope.clone(),
                "provider-session",
                None,
            )
            .unwrap(),
        )),
    });
    let hook_health = Arc::new(MemoryHookHealth::default());
    let health = Arc::new(ProviderHookHealthUsecase::new(hook_health.clone()));
    hook_health
        .fail_saves
        .store(4, std::sync::atomic::Ordering::Relaxed);
    let ingress = ProviderLifecycleIngressUsecase::new(
        interpreter,
        Arc::new(crate::usecase::test_helpers::TestIdentity),
        lifecycle.clone(),
        Arc::new(AgentSessionUsecase::new(
            sessions.clone(),
            Arc::new(FakeThreadRepository::default()),
            AgentSessionService::new(
                Arc::new(RecordingTerminal::default()),
                Arc::new(RecordingTerminalInput::default()),
            ),
        )),
        health,
        (sessions, Arc::new(MemoryWorkflowStops::default())),
        crate::test_support::state_subscription::test_subscriptions(),
    )
    .with_usage(usage.clone());
    let input = || ProviderPayloadInput {
        provider: ProviderKind::Codex,
        binding_id: armed.binding_id(),
        scope: scope.clone(),
        payload: b"start",
    };
    // When
    assert!(matches!(
        ingress
            .receive_payload(&slot, armed.capability(), input())
            .await,
        Err(ProviderLifecycleIngressUsecaseError::StorageUnavailable)
    ));
    assert!(projection.1.lock().unwrap().is_empty());
    let result = ingress
        .receive_payload(&slot, armed.capability(), input())
        .await
        .unwrap();
    // Then
    assert_eq!(result.0, ProviderLifecycleIngressResult::Duplicate);
    assert!(projection
        .1
        .lock()
        .unwrap()
        .contains_key("provider-session"));
    *source.record.lock().unwrap() = "new".into();
    usage.poll_active_codex(&lifecycle).await.unwrap();
    assert_eq!(usage.snapshot()[0].plan.as_deref(), Some("pro"));
}
