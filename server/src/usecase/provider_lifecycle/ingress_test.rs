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
        assert_eq!(
            ingress
                .receive_payload(&slot, armed.capability(), input())
                .await
                .unwrap(),
            (expected, true)
        );
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
    assert_eq!(
        ingress
            .receive_payload(&slot, armed.capability(), input())
            .await
            .unwrap(),
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
            .unwrap_err();
        // Then
        assert_eq!(
            error,
            ProviderLifecycleIngressUsecaseError::Payload(failure.clone())
        );
        assert_eq!(error.to_string(), failure.to_string());
    }
    assert_eq!(sessions.stored.lock().unwrap().revision(), revision);
}
