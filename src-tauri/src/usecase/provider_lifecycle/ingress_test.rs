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

struct PayloadInterpreter(
    Result<
        crate::domain::provider_lifecycle::ProviderPayloadInterpretation,
        crate::domain::provider_lifecycle::ProviderPayloadError,
    >,
);
impl crate::domain::provider_lifecycle::ProviderPayloadInterpreter for PayloadInterpreter {
    fn interpret(
        &self,
        _: crate::domain::provider_lifecycle::ProviderKind,
        _: &str,
        _: crate::domain::provider_lifecycle::ProviderLifecycleScope,
        _: &[u8],
    ) -> Result<
        crate::domain::provider_lifecycle::ProviderPayloadInterpretation,
        crate::domain::provider_lifecycle::ProviderPayloadError,
    > {
        self.0.clone()
    }
}
struct RecordingIngress(std::sync::Mutex<Vec<ProviderLifecycleSignal>>);
#[async_trait::async_trait]
impl ProviderLifecycleIngressPort for RecordingIngress {
    async fn receive(
        &self,
        _: &ProviderLifecycleSlotId,
        _: &str,
        signal: ProviderLifecycleSignal,
    ) -> Result<ProviderLifecycleIngressResult, ProviderLifecycleIngressUsecaseError> {
        let mut signals = self.0.lock().unwrap();
        let duplicate = signals.contains(&signal);
        signals.push(signal);
        Ok(if duplicate {
            ProviderLifecycleIngressResult::Duplicate
        } else {
            ProviderLifecycleIngressResult::Applied
        })
    }
    async fn report_unavailable(
        &self,
        _: &ProviderLifecycleSlotId,
        _: &str,
        _: ProviderLifecycleUnavailableObservation,
    ) -> Result<ProviderLifecycleIngressResult, ProviderLifecycleIngressUsecaseError> {
        unreachable!()
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
    let signal = ProviderLifecycleSignal::session_started(
        "binding",
        ProviderKind::Claude,
        scope.clone(),
        "provider-session",
        None,
    )
    .unwrap();
    let ingress = RecordingIngress(Default::default());
    let input = || ProviderPayloadInput {
        provider: ProviderKind::Claude,
        binding_id: "binding",
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
                .receive_payload(
                    &PayloadInterpreter(Ok(Parsed::Signal(signal.clone()))),
                    &slot,
                    "capability",
                    input()
                )
                .await
                .unwrap(),
            (expected, true)
        );
    }
    assert_eq!(
        ingress.0.lock().unwrap().as_slice(),
        &[signal.clone(), signal]
    );
    assert_eq!(
        ingress
            .receive_payload(
                &PayloadInterpreter(Ok(Parsed::Subagent)),
                &slot,
                "capability",
                input()
            )
            .await
            .unwrap(),
        (ProviderLifecycleIngressResult::Ignored, false)
    );
    assert_eq!(
        ingress
            .receive_payload(
                &PayloadInterpreter(Err(ProviderPayloadError::InvalidPayload)),
                &slot,
                "capability",
                input()
            )
            .await
            .unwrap_err(),
        ProviderLifecycleIngressUsecaseError::InvalidInput
    );
    assert_eq!(ingress.0.lock().unwrap().len(), 2);
}
