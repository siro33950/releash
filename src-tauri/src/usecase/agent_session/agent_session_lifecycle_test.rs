mod error_tests {
    use super::super::*;

    use crate::domain::workflow::WorkflowError;

    #[test]
    fn test_session操作を経由してもworkflowの失敗分類を保持する() {
        // Given / When / Then
        for error in [
            WorkflowError::Store(
                crate::domain::failure::StorageFailure::from(
                    crate::domain::local_event::CommitBatchError::QueueBusy,
                )
                .with_message("busy"),
            ),
            WorkflowError::Store(
                crate::domain::failure::StorageFailure::from(
                    crate::domain::local_event::CommitBatchError::PayloadConflict,
                )
                .with_message("repair"),
            ),
            WorkflowError::Technical(crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::Cancelled,
                message: "query cancelled".into(),
            }),
            WorkflowError::External("internal".into()),
            WorkflowError::IncompatibleStoredEvent("version".into()),
            WorkflowError::CorruptStoredState("corrupt".into()),
            WorkflowError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                    message: "failure".into(),
                })
                .into(),
            ),
            WorkflowError::Conflict("revision".into()),
        ] {
            let expected = match &error {
                WorkflowError::Store(failure) => {
                    AgentSessionLifecycleUsecaseError::Store(failure.clone())
                }
                WorkflowError::CorruptStoredState(_) => AgentSessionLifecycleUsecaseError::Corrupt,
                WorkflowError::Conflict(_) => {
                    AgentSessionLifecycleUsecaseError::Conflict(error.clone().into())
                }
                _ => AgentSessionLifecycleUsecaseError::Workflow(error.clone()),
            };
            assert_eq!(map_workflow_error(error), expected);
        }
    }

    #[test]
    fn test_session所有済みと保存競合を区別して伝播する() {
        use super::super::AgentSessionUsecaseError;

        // Given / When / Then
        for source in [
            AgentSessionUsecaseError::Conflict,
            AgentSessionUsecaseError::ProviderSessionAlreadyOwned {
                agent_session_id: "owner".into(),
            },
        ] {
            let expected = match &source {
            AgentSessionUsecaseError::Conflict => {
                crate::domain::agent_session::repository::AgentSessionRepositoryError::Conflict
                    .into()
            }
            AgentSessionUsecaseError::ProviderSessionAlreadyOwned { agent_session_id } => {
                crate::domain::agent_session::repository::AgentSessionRepositoryError::ProviderSessionAlreadyOwned {
                    agent_session_id: agent_session_id.clone(),
                }
                .into()
            }
            _ => unreachable!(),
        };
            assert_eq!(
                super::super::map_session_error(source),
                super::super::AgentSessionLifecycleUsecaseError::Conflict(expected)
            );
        }
    }

    #[test]
    fn test_workflowの結果不明を一時的なstore失敗に変えない() {
        let error = WorkflowError::Store(
            crate::domain::local_event::CommitBatchError::AppendOutcomeUnknown.into(),
        );
        // When / Then
        assert_eq!(
            map_workflow_error(error),
            AgentSessionLifecycleUsecaseError::Store(
                crate::domain::local_event::CommitBatchError::AppendOutcomeUnknown.into()
            )
        );
    }

    #[test]
    fn test_session失敗_全変種から技術的な失敗だけを参照する() {
        use super::super::AgentSessionLifecycleUsecaseError as E;
        use crate::domain::agent_session::{
            ProviderAgentLaunchGatewayError as L, ProviderAgentTerminalGatewayError as T,
        };
        use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
        // Given
        let storage = crate::domain::failure::StorageFailure::from(
            crate::domain::local_event::CommitBatchError::QueueBusy,
        );
        // When / Then
        for error in [
            E::NotFound,
            E::InvalidOperation,
            E::ProviderUnavailable,
            E::StorageUnavailable,
            E::Corrupt,
            E::Store(storage.clone()),
            E::Conflict(storage.clone()),
            E::Launch(L::InvalidInput),
            E::Terminal(T::NotFound("missing".into())),
            E::Terminal(T::InvalidOperation("invalid".into())),
            E::Terminal(T::StaleAttachment),
            E::Terminal(T::OwnerConflict),
            E::Workflow(crate::domain::workflow::WorkflowError::External(
                "workflow".into(),
            )),
        ] {
            assert_eq!(error.technical_failure(), None);
        }
        for nature in [
            TechnicalFailureNature::Transient,
            TechnicalFailureNature::TimedOut,
            TechnicalFailureNature::Cancelled,
            TechnicalFailureNature::Other,
        ] {
            let failure = TechnicalFailure {
                nature,
                message: "source failure".into(),
            };
            for error in [
                E::Launch(L::Technical(failure.clone())),
                E::Terminal(T::Technical(failure.clone())),
            ] {
                assert_eq!(error.technical_failure(), Some(&failure));
            }
        }
    }
}
