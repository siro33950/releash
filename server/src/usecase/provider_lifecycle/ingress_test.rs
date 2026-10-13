use super::*;
use crate::domain::failure::{StorageFailure, StorageFailureSource, TechnicalFailureNature};

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
            Arc::new(AgentSessionUsecase::new(sessions.clone())),
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
        let (result, started, usage_error) = ingress
            .receive_payload(&slot, armed.capability(), input())
            .await
            .unwrap();
        assert_eq!((result, started), (expected, true));
        assert!(usage_error.is_none());
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
    let (result, started, usage_error) = ingress
        .receive_payload(&slot, armed.capability(), input())
        .await
        .unwrap();
    assert_eq!(
        (result, started),
        (ProviderLifecycleIngressResult::Ignored, false)
    );
    assert!(usage_error.is_none());
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
impl crate::usecase::provider_usage::ProviderUsageSource for UsageSource {
    fn claude(
        &self,
        _: &[u8],
    ) -> Result<
        Option<crate::usecase::provider_usage::ProviderUsage>,
        crate::usecase::provider_usage::UsageError,
    > {
        unreachable!("codex test")
    }
    async fn codex(
        &self,
        _: &str,
    ) -> Result<
        Option<(String, crate::usecase::provider_usage::ProviderUsage)>,
        crate::usecase::provider_usage::UsageError,
    > {
        self.reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if self.fail {
            return Err(crate::usecase::provider_usage::UsageError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: TechnicalFailureNature::TimedOut,
                    message: "usage unavailable".into(),
                },
            ));
        }
        Ok(Some((
            self.record.lock().unwrap().clone(),
            crate::usecase::provider_usage::ProviderUsage {
                provider: ProviderKind::Codex,
                windows: vec![crate::usecase::provider_usage::UsageWindow {
                    used_percent: 70.0,
                    window_minutes: 300,
                    resets_at: 123,
                    warning: false,
                }],
                plan: Some("pro".into()),
            },
        )))
    }
}
#[derive(Default)]
struct UsageProjection(Mutex<Vec<crate::usecase::provider_usage::ProviderUsage>>);
impl crate::usecase::provider_usage::ProviderUsageQueryService for UsageProjection {
    fn snapshot(&self) -> Vec<crate::usecase::provider_usage::ProviderUsage> {
        self.0.lock().unwrap().clone()
    }
}
impl crate::usecase::provider_usage::ProviderUsageOutput for UsageProjection {
    fn replace(&self, usage: crate::usecase::provider_usage::ProviderUsage) {
        *self.0.lock().unwrap() = vec![usage];
    }
}

#[tokio::test]
async fn test_使用率監視_受理したcodexの開始だけでrolloutの基準を取り現行bindingを監視する() {
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
    let usage = Arc::new(crate::usecase::provider_usage::ProviderUsageUsecase::new(
        source.clone(),
        projection.clone(),
        projection,
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
        Arc::new(AgentSessionUsecase::new(sessions.clone())),
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
    let (result, started, preparation) = ingress
        .receive_payload(&slot, armed.capability(), input())
        .await
        .unwrap();
    assert_eq!(
        (result, started),
        (ProviderLifecycleIngressResult::Applied, true)
    );
    preparation.unwrap().await.unwrap();
    assert_eq!(
        ingress
            .receive_payload(&slot, armed.capability(), input())
            .await
            .unwrap()
            .0,
        ProviderLifecycleIngressResult::Duplicate
    );
    assert_eq!(source.reads.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert!(usage
        .poll_active_codex(&lifecycle)
        .await
        .unwrap()
        .is_empty());
    assert!(usage.snapshot().is_empty());
    let reads_before = source.reads.load(std::sync::atomic::Ordering::Relaxed);
    let observation = usage.lock_codex_observation().await;
    let mut poll = Box::pin(usage.poll_active_codex(&lifecycle));
    assert!(futures_util::poll!(poll.as_mut()).is_pending());
    assert_eq!(
        source.reads.load(std::sync::atomic::Ordering::Relaxed),
        reads_before
    );
    drop(observation);
    poll.await.unwrap();
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
async fn test_使用率準備失敗_session開始を適用し型付き失敗を別に返す() {
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
        projection,
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
        Arc::new(AgentSessionUsecase::new(sessions.clone())),
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
    let (result, started, usage_error) = ingress
        .receive_payload(&slot, armed.capability(), input())
        .await
        .unwrap();
    // Then
    assert_eq!(
        (result, started),
        (ProviderLifecycleIngressResult::Applied, true)
    );
    match usage_error.unwrap().await.unwrap_err() {
        crate::usecase::provider_usage::UsageError::Technical(error) => {
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
