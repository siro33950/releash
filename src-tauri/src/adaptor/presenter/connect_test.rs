use super::*;
use crate::adaptor::presenter::connect::ConnectFailure;
use crate::adaptor::presenter::error::AppError;
use connectrpc::ErrorCode;

#[test]
fn test_失敗分類を単一の対応表でconnectへ変換する() {
    // Given / When / Then
    for (kind, expected) in [
        (ErrorCode::Unavailable, connectrpc::ErrorCode::Unavailable),
        (ErrorCode::Aborted, connectrpc::ErrorCode::Aborted),
        (
            ErrorCode::FailedPrecondition,
            connectrpc::ErrorCode::FailedPrecondition,
        ),
        (
            ErrorCode::DeadlineExceeded,
            connectrpc::ErrorCode::DeadlineExceeded,
        ),
        (ErrorCode::DataLoss, connectrpc::ErrorCode::DataLoss),
        (ErrorCode::Internal, connectrpc::ErrorCode::Internal),
        (
            ErrorCode::InvalidArgument,
            connectrpc::ErrorCode::InvalidArgument,
        ),
        (ErrorCode::NotFound, connectrpc::ErrorCode::NotFound),
        (
            ErrorCode::AlreadyExists,
            connectrpc::ErrorCode::AlreadyExists,
        ),
        (
            ErrorCode::PermissionDenied,
            connectrpc::ErrorCode::PermissionDenied,
        ),
        (
            ErrorCode::ResourceExhausted,
            connectrpc::ErrorCode::ResourceExhausted,
        ),
        (
            ErrorCode::Unimplemented,
            connectrpc::ErrorCode::Unimplemented,
        ),
        (ErrorCode::Canceled, connectrpc::ErrorCode::Canceled),
        (ErrorCode::Unknown, connectrpc::ErrorCode::Unknown),
        (ErrorCode::OutOfRange, connectrpc::ErrorCode::OutOfRange),
        (
            ErrorCode::Unauthenticated,
            connectrpc::ErrorCode::Unauthenticated,
        ),
    ] {
        let error = AppError::new("failure").with_status(kind);
        assert_eq!(classified_error(error.clone()).code, expected);
        let coded = AppError::coded("SAME_CODE", "failure", kind);
        assert_eq!(coded.connect_code(), kind);
        assert_eq!(command_error(coded.into()).code, expected);
        let command = command_error(error.into());
        assert_eq!(command.code, expected);
        assert_eq!(command.details.len(), 1);
    }
}

#[test]
fn test_実行中のworktreeは状態を直すまで再試行できない() {
    // Given
    let error = crate::usecase::workflow::runtime_error::WorkflowRuntimeError::AlreadyActive(
        crate::domain::workflow::services::start_admission::WorktreeActiveExecution {
            worktree_path: "/worktree".into(),
            execution_id: "execution".into(),
            workflow_name: "workflow".into(),
        },
    );
    // When
    let error = command_error(AppError::from_failure(error).into());
    // Then
    assert_eq!(error.code, connectrpc::ErrorCode::FailedPrecondition);
}

#[test]
fn test_技術的な失敗_自身の性質だけから転送コードを決める() {
    use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature as N};
    use connectrpc::ErrorCode as C;
    // Given
    for (nature, code) in [
        (N::Transient, C::Unavailable),
        (N::TimedOut, C::DeadlineExceeded),
        (N::Cancelled, C::Canceled),
        (N::Other, C::Internal),
    ] {
        // When
        let error = classified_error(TechnicalFailure {
            nature,
            message: "failure".into(),
        });
        // Then
        assert_eq!(error.code, code);
        assert_eq!(error.message.as_deref(), Some("failure"));
    }
}

#[test]
fn test_作業手順の失敗_storeに包んでも転送コードを保持する() {
    use crate::domain::failure::{StorageFailure, TechnicalFailure, TechnicalFailureNature};
    use crate::domain::workflow::WorkflowError;
    use crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError;

    // Given
    for (error, expected) in [
        (
            WorkflowError::Technical(TechnicalFailure {
                nature: TechnicalFailureNature::TimedOut,
                message: "expired".into(),
            }),
            ErrorCode::DeadlineExceeded,
        ),
        (
            WorkflowError::Technical(TechnicalFailure {
                nature: TechnicalFailureNature::Cancelled,
                message: "cancelled".into(),
            }),
            ErrorCode::Canceled,
        ),
        (
            WorkflowError::External("internal".into()),
            ErrorCode::Internal,
        ),
        (
            WorkflowError::Editor(crate::domain::external_editor::EditorError::Launch(
                "launch".into(),
            )),
            ErrorCode::FailedPrecondition,
        ),
        (
            WorkflowError::Store(StorageFailure::from(
                crate::domain::local_event::CommitBatchError::QueueBusy,
            )),
            ErrorCode::Unavailable,
        ),
        (
            WorkflowError::Store(StorageFailure::from(TechnicalFailure {
                nature: TechnicalFailureNature::TimedOut,
                message: "expired".into(),
            })),
            ErrorCode::DeadlineExceeded,
        ),
        (
            WorkflowError::Store(StorageFailure::from(
                crate::domain::local_event::CommitBatchError::PayloadConflict,
            )),
            ErrorCode::FailedPrecondition,
        ),
    ] {
        // When
        let failure = StorageFailure::from(error);
        let failure_code = failure.connect_code();
        let usecase_code = ProviderLifecycleIngressUsecaseError::Store(failure).connect_code();
        // Then
        assert_eq!(failure_code, expected);
        assert_eq!(usecase_code, expected);
    }
}

#[test]
fn test_mainでstoreだった経路_httpとconnectのコードを保持する() {
    use crate::adaptor::controller::api::error::ApiError;
    use crate::domain::agent_session::repository::AgentSessionRepositoryError;
    use crate::domain::failure::TechnicalFailure;
    use crate::domain::local_event::{CommitBatchError, LocalEventQueryError};
    use crate::domain::workflow::WorkflowError;
    use crate::usecase::workflow::runtime_error::WorkflowRuntimeError;

    let cases = [
        (
            WorkflowError::storage(CommitBatchError::QueueBusy, "commit"),
            ErrorCode::Unavailable,
        ),
        (
            WorkflowError::from(CommitBatchError::TreeHeadConflict),
            ErrorCode::Aborted,
        ),
        (
            WorkflowError::storage(LocalEventQueryError::QueryBusy, "query"),
            ErrorCode::Unavailable,
        ),
        (
            WorkflowError::storage(
                TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                    message: "deadline".into(),
                },
                "archive",
            ),
            ErrorCode::DeadlineExceeded,
        ),
        (
            WorkflowError::storage(
                WorkflowError::Editor(crate::domain::external_editor::EditorError::Launch(
                    "editor".into(),
                )),
                "editor",
            ),
            ErrorCode::FailedPrecondition,
        ),
        (
            WorkflowError::Store(
                crate::domain::failure::StorageFailure::from(WorkflowError::Conflict(
                    "advanced".into(),
                ))
                .with_message("reconcile"),
            ),
            ErrorCode::Aborted,
        ),
    ];
    for (error, code) in cases {
        assert!(matches!(error, WorkflowError::Store(_)));
        assert_eq!(error.connect_code(), code);
        assert_eq!(ApiError::from(error).status, code.http_status());
    }

    let runtime = WorkflowRuntimeError::Store(
        crate::domain::failure::StorageFailure::from(WorkflowError::Conflict("advanced".into()))
            .with_message("isolated worktree"),
    );
    assert_eq!(runtime.connect_code(), ErrorCode::Aborted);
    let WorkflowRuntimeError::Store(failure) = runtime else {
        panic!("runtime error must stay in Store");
    };
    let workflow = WorkflowError::Store(failure);
    assert_eq!(
        ApiError::from(workflow).status,
        ErrorCode::Aborted.http_status()
    );

    let owned = AgentSessionRepositoryError::ProviderSessionAlreadyOwned {
        agent_session_id: "owner".into(),
    };
    let ingress = crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::Store(
        owned.into(),
    );
    assert_eq!(ingress.connect_code(), ErrorCode::FailedPrecondition);
    assert_eq!(
        ApiError::from(ingress).status,
        ErrorCode::FailedPrecondition.http_status()
    );
}

mod connect_mapping_tests {
    mod usecase_repository_state_error_test {
        #[test]
        fn test_失敗分類_全変種と委譲した理由を保持する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
                (
                    crate::usecase::repository_state::error::RepositoryStateError::ScanInvalidated,
                    F::Aborted,
                ),
                (
                    crate::usecase::repository_state::error::RepositoryStateError::Background {
                        kind: crate::usecase::failure::Failure::Technical(
                            crate::domain::failure::TechnicalFailureNature::Transient,
                        ),
                        message: "worker".into(),
                    },
                    F::Unavailable,
                ),
                (
                    crate::usecase::repository_state::error::RepositoryStateError::Background {
                        kind: crate::usecase::failure::Failure::Technical(
                            crate::domain::failure::TechnicalFailureNature::TimedOut,
                        ),
                        message: "deadline".into(),
                    },
                    F::DeadlineExceeded,
                ),
                (
                    crate::usecase::repository_state::error::RepositoryStateError::Watcher(
                        "watch".into(),
                    ),
                    F::Internal,
                ),
                (
                    crate::usecase::repository_state::error::RepositoryStateError::Repository(
                        crate::usecase::repository_error::UsecaseError::Rule("state".into()),
                    ),
                    F::FailedPrecondition,
                ),
                (
                    crate::usecase::repository_state::error::RepositoryStateError::Repository(
                        crate::usecase::repository_error::UsecaseError::Repository(
                            crate::domain::repository::RepositoryError::External("io".into()),
                        ),
                    ),
                    F::Internal,
                ),
                (
                    crate::usecase::repository_state::error::RepositoryStateError::Code(
                        crate::usecase::code_error::CodeUsecaseError::Code(
                            crate::domain::code::CodeError::StaleReviewGroupTarget {
                                group_id: "g".into(),
                            },
                        ),
                    ),
                    F::Aborted,
                ),
                (
                    crate::usecase::repository_state::error::RepositoryStateError::Code(
                        crate::usecase::code_error::CodeUsecaseError::Code(
                            crate::domain::code::CodeError::External("io".into()),
                        ),
                    ),
                    F::Internal,
                ),
            ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod usecase_provider_lifecycle_hook_health_error_test {
        #[test]
        fn test_失敗分類_usecase_全変種と委譲した理由を保持する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
        (crate::usecase::provider_lifecycle::ProviderHookHealthUsecaseError::Conflict, F::Aborted),
        (
            crate::usecase::provider_lifecycle::ProviderHookHealthUsecaseError::InvalidInput,
            F::InvalidArgument,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderHookHealthUsecaseError::StorageUnavailable,
            F::Unavailable,
        ),
        (crate::usecase::provider_lifecycle::ProviderHookHealthUsecaseError::Corrupt, F::DataLoss),
        (
            crate::usecase::provider_lifecycle::ProviderHookHealthUsecaseError::Store(
                (crate::domain::local_event::CommitBatchError::QueueBusy).into(),
            ),
            F::Unavailable,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderHookHealthUsecaseError::Store(
                (crate::domain::local_event::CommitBatchError::TreeHeadConflict).into(),
            ),
            F::Aborted,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderHookHealthUsecaseError::Store(
                (crate::domain::local_event::CommitBatchError::PayloadConflict).into(),
            ),
            F::FailedPrecondition,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderHookHealthUsecaseError::Store(
                (crate::domain::local_event::LocalEventQueryError::InvalidRequest).into(),
            ),
            F::InvalidArgument,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderHookHealthUsecaseError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                    message: "failure".into(),
                })
                .into(),
            ),
            F::DeadlineExceeded,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderHookHealthUsecaseError::Store(
                (crate::domain::workflow::WorkflowError::NotFound("missing".into())).into(),
            ),
            F::NotFound,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderHookHealthUsecaseError::Store(
                (crate::domain::workflow::WorkflowError::UnauthorizedApprovalTarget(
                    "denied".into(),
                ))
                .into(),
            ),
            F::PermissionDenied,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderHookHealthUsecaseError::Store(
                (crate::domain::local_event::CommitBatchError::CapacityExceeded).into(),
            ),
            F::ResourceExhausted,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderHookHealthUsecaseError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: "failure".into(),
                })
                .into(),
            ),
            F::Internal,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderHookHealthUsecaseError::Store(
                (crate::domain::local_event::CommitBatchError::Corrupt {
                    correlation_id: "id".into(),
                })
                .into(),
            ),
            F::DataLoss,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderHookHealthUsecaseError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Cancelled,
                    message: "failure".into(),
                })
                .into(),
            ),
            F::Canceled,
        ),
    ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod usecase_provider_lifecycle_hook_health_error_test_2 {
        #[test]
        fn test_失敗分類_query_全変種と委譲した理由を保持する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
            (
                crate::usecase::provider_lifecycle::ProviderHookHealthFailureQueryError::Technical(
                    crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::Transient,
                        message: "unavailable".into(),
                    },
                ),
                F::Unavailable,
            ),
            (
                crate::usecase::provider_lifecycle::ProviderHookHealthFailureQueryError::Corrupt,
                F::DataLoss,
            ),
        ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod usecase_provider_lifecycle_provider_lifecycle_ingress_test {
        #[test]
        fn test_失敗分類_usecase_全変種と委譲した理由を保持する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::Store(
                crate::domain::agent_session::repository::AgentSessionRepositoryError::ProviderSessionAlreadyOwned { agent_session_id: "owner".into() }.into(),
            ),
            F::FailedPrecondition,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::InvalidInput,
            F::InvalidArgument,
        ),
        (crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::Conflict, F::Aborted),
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::StorageUnavailable,
            F::Unavailable,
        ),
        (crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::Corrupt, F::DataLoss),
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::Store(
                (crate::domain::local_event::CommitBatchError::QueueBusy).into(),
            ),
            F::Unavailable,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::Store(
                (crate::domain::local_event::CommitBatchError::TreeHeadConflict).into(),
            ),
            F::Aborted,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::Store(
                (crate::domain::local_event::CommitBatchError::PayloadConflict).into(),
            ),
            F::FailedPrecondition,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::Store(
                (crate::domain::local_event::LocalEventQueryError::InvalidRequest).into(),
            ),
            F::InvalidArgument,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                    message: "failure".into(),
                })
                .into(),
            ),
            F::DeadlineExceeded,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::Store(
                (crate::domain::workflow::WorkflowError::NotFound("missing".into())).into(),
            ),
            F::NotFound,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::Store(
                (crate::domain::workflow::WorkflowError::UnauthorizedApprovalTarget(
                    "denied".into(),
                ))
                .into(),
            ),
            F::PermissionDenied,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::Store(
                (crate::domain::local_event::CommitBatchError::CapacityExceeded).into(),
            ),
            F::ResourceExhausted,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: "failure".into(),
                })
                .into(),
            ),
            F::Internal,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::Store(
                (crate::domain::local_event::CommitBatchError::Corrupt {
                    correlation_id: "id".into(),
                })
                .into(),
            ),
            F::DataLoss,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Cancelled,
                    message: "failure".into(),
                })
                .into(),
            ),
            F::Canceled,
        ),
    ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod usecase_provider_lifecycle_provider_lifecycle_usecase_test {
        #[test]
        fn test_失敗分類_usecase_全変種と委譲した理由を保持する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleUsecaseError::InvalidInput,
            F::InvalidArgument,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleUsecaseError::StorageUnavailable,
            F::Unavailable,
        ),
        (crate::usecase::provider_lifecycle::ProviderLifecycleUsecaseError::Corrupt, F::DataLoss),
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleUsecaseError::Store(
                (crate::domain::local_event::CommitBatchError::QueueBusy).into(),
            ),
            F::Unavailable,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleUsecaseError::Store(
                (crate::domain::local_event::CommitBatchError::TreeHeadConflict).into(),
            ),
            F::Aborted,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleUsecaseError::Store(
                (crate::domain::local_event::CommitBatchError::PayloadConflict).into(),
            ),
            F::FailedPrecondition,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleUsecaseError::Store(
                (crate::domain::local_event::LocalEventQueryError::InvalidRequest).into(),
            ),
            F::InvalidArgument,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleUsecaseError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                    message: "failure".into(),
                })
                .into(),
            ),
            F::DeadlineExceeded,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleUsecaseError::Store(
                (crate::domain::workflow::WorkflowError::NotFound("missing".into())).into(),
            ),
            F::NotFound,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleUsecaseError::Store(
                (crate::domain::workflow::WorkflowError::UnauthorizedApprovalTarget(
                    "denied".into(),
                ))
                .into(),
            ),
            F::PermissionDenied,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleUsecaseError::Store(
                (crate::domain::local_event::CommitBatchError::CapacityExceeded).into(),
            ),
            F::ResourceExhausted,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleUsecaseError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: "failure".into(),
                })
                .into(),
            ),
            F::Internal,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleUsecaseError::Store(
                (crate::domain::local_event::CommitBatchError::Corrupt {
                    correlation_id: "id".into(),
                })
                .into(),
            ),
            F::DataLoss,
        ),
        (
            crate::usecase::provider_lifecycle::ProviderLifecycleUsecaseError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Cancelled,
                    message: "failure".into(),
                })
                .into(),
            ),
            F::Canceled,
        ),
    ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod usecase_workflow_runtime_error_test {
        #[test]
        fn test_失敗分類_全変種と委譲した理由を保持する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
        (
            crate::usecase::workflow::runtime_error::WorkflowRuntimeError::ExecutionNotFound("reason".into()),
            F::NotFound,
        ),
        (
            crate::usecase::workflow::runtime_error::WorkflowRuntimeError::SessionNotFound("reason".into()),
            F::NotFound,
        ),
        (
            crate::usecase::workflow::runtime_error::WorkflowRuntimeError::InvalidWorkflow("reason".into()),
            F::InvalidArgument,
        ),
        (
            crate::usecase::workflow::runtime_error::WorkflowRuntimeError::InvalidState("reason".into()),
            F::FailedPrecondition,
        ),
        (crate::usecase::workflow::runtime_error::WorkflowRuntimeError::Conflict("reason".into()), F::Aborted),
        (
            crate::usecase::workflow::runtime_error::WorkflowRuntimeError::ValidationError("reason".into()),
            F::InvalidArgument,
        ),
        (
            crate::usecase::workflow::runtime_error::WorkflowRuntimeError::UnauthorizedWorktree("reason".into()),
            F::PermissionDenied,
        ),
        (
            crate::usecase::workflow::runtime_error::WorkflowRuntimeError::UnauthorizedApprovalTarget("reason".into()),
            F::PermissionDenied,
        ),
        (
            crate::usecase::workflow::runtime_error::WorkflowRuntimeError::SessionStore("reason".into()),
            F::Internal,
        ),
        (
            crate::usecase::workflow::runtime_error::WorkflowRuntimeError::AgentSession("reason".into()),
            F::FailedPrecondition,
        ),
        (
            crate::usecase::workflow::runtime_error::WorkflowRuntimeError::AlreadyActive(
                crate::domain::workflow::services::start_admission::WorktreeActiveExecution {
                    worktree_path: "/repo".into(),
                    execution_id: "execution".into(),
                    workflow_name: "workflow".into(),
                },
            ),
            F::FailedPrecondition,
        ),
        (
            crate::usecase::workflow::runtime_error::WorkflowRuntimeError::Store(
                (crate::domain::local_event::CommitBatchError::QueueBusy).into(),
            ),
            F::Unavailable,
        ),
        (
            crate::usecase::workflow::runtime_error::WorkflowRuntimeError::Store(
                (crate::domain::local_event::CommitBatchError::TreeHeadConflict).into(),
            ),
            F::Aborted,
        ),
        (
            crate::usecase::workflow::runtime_error::WorkflowRuntimeError::Store(
                (crate::domain::local_event::CommitBatchError::PayloadConflict).into(),
            ),
            F::FailedPrecondition,
        ),
        (
            crate::usecase::workflow::runtime_error::WorkflowRuntimeError::Store(
                (crate::domain::local_event::LocalEventQueryError::InvalidRequest).into(),
            ),
            F::InvalidArgument,
        ),
        (
            crate::usecase::workflow::runtime_error::WorkflowRuntimeError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                    message: "failure".into(),
                })
                .into(),
            ),
            F::DeadlineExceeded,
        ),
        (
            crate::usecase::workflow::runtime_error::WorkflowRuntimeError::Store(
                (crate::domain::workflow::WorkflowError::NotFound("missing".into())).into(),
            ),
            F::NotFound,
        ),
        (
            crate::usecase::workflow::runtime_error::WorkflowRuntimeError::Store(
                (crate::domain::workflow::WorkflowError::UnauthorizedApprovalTarget(
                    "denied".into(),
                ))
                .into(),
            ),
            F::PermissionDenied,
        ),
        (
            crate::usecase::workflow::runtime_error::WorkflowRuntimeError::Store(
                (crate::domain::local_event::CommitBatchError::CapacityExceeded).into(),
            ),
            F::ResourceExhausted,
        ),
        (
            crate::usecase::workflow::runtime_error::WorkflowRuntimeError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: "failure".into(),
                })
                .into(),
            ),
            F::Internal,
        ),
        (
            crate::usecase::workflow::runtime_error::WorkflowRuntimeError::Store(
                (crate::domain::local_event::CommitBatchError::Corrupt {
                    correlation_id: "id".into(),
                })
                .into(),
            ),
            F::DataLoss,
        ),
        (
            crate::usecase::workflow::runtime_error::WorkflowRuntimeError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Cancelled,
                    message: "failure".into(),
                })
                .into(),
            ),
            F::Canceled,
        ),
    ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod usecase_notion_error_test {
        #[test]
        fn test_失敗分類_全変種と委譲した理由を保持する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
                (
                    crate::usecase::notion::error::NotionUsecaseError::ConfigNotFound,
                    F::FailedPrecondition,
                ),
                (
                    crate::usecase::notion::error::NotionUsecaseError::AppConfig(
                        crate::domain::app_config::AppConfigError::InvalidInput("input".into()),
                    ),
                    F::InvalidArgument,
                ),
                (
                    crate::usecase::notion::error::NotionUsecaseError::AppConfig(
                        crate::domain::app_config::AppConfigError::Repository("io".into()),
                    ),
                    F::Internal,
                ),
                (
                    crate::usecase::notion::error::NotionUsecaseError::Notion(
                        crate::domain::notion::NotionError::RequestFailed("network".into()),
                    ),
                    F::Unavailable,
                ),
                (
                    crate::usecase::notion::error::NotionUsecaseError::Notion(
                        crate::domain::notion::NotionError::ApiError("api".into()),
                    ),
                    F::FailedPrecondition,
                ),
                (
                    crate::usecase::notion::error::NotionUsecaseError::Notion(
                        crate::domain::notion::NotionError::ParseError("parse".into()),
                    ),
                    F::Internal,
                ),
            ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod usecase_app_config_error_test {
        #[test]
        fn test_失敗分類_全変種と委譲した理由を保持する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
                (
                    crate::usecase::app_config::error::UsecaseError::InvalidInput("input".into()),
                    F::InvalidArgument,
                ),
                (
                    crate::usecase::app_config::error::UsecaseError::AppConfig(
                        crate::domain::app_config::AppConfigError::InvalidInput("input".into()),
                    ),
                    F::InvalidArgument,
                ),
                (
                    crate::usecase::app_config::error::UsecaseError::AppConfig(
                        crate::domain::app_config::AppConfigError::Repository("io".into()),
                    ),
                    F::Internal,
                ),
            ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod usecase_agent_session_agent_session_initial_instruction_test {
        #[test]
        fn test_失敗分類_全変種と委譲した理由を保持する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
        (
            crate::usecase::agent_session::AgentSessionInitialInstructionError::InvalidInput,
            F::InvalidArgument,
        ),
        (crate::usecase::agent_session::AgentSessionInitialInstructionError::NotFound, F::NotFound),
        (
            crate::usecase::agent_session::AgentSessionInitialInstructionError::Conflict(
                (crate::domain::local_event::CommitBatchError::TreeHeadConflict).into(),
            ),
            F::Aborted,
        ),
        (
            crate::usecase::agent_session::AgentSessionInitialInstructionError::StorageUnavailable,
            F::Unavailable,
        ),
        (crate::usecase::agent_session::AgentSessionInitialInstructionError::Corrupt, F::DataLoss),
        (
            crate::usecase::agent_session::AgentSessionInitialInstructionError::Store(
                (crate::domain::local_event::CommitBatchError::QueueBusy).into(),
            ),
            F::Unavailable,
        ),
        (
            crate::usecase::agent_session::AgentSessionInitialInstructionError::Store(
                (crate::domain::local_event::CommitBatchError::TreeHeadConflict).into(),
            ),
            F::Aborted,
        ),
        (
            crate::usecase::agent_session::AgentSessionInitialInstructionError::Store(
                (crate::domain::local_event::CommitBatchError::PayloadConflict).into(),
            ),
            F::FailedPrecondition,
        ),
        (
            crate::usecase::agent_session::AgentSessionInitialInstructionError::Store(
                (crate::domain::local_event::LocalEventQueryError::InvalidRequest).into(),
            ),
            F::InvalidArgument,
        ),
        (
            crate::usecase::agent_session::AgentSessionInitialInstructionError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                    message: "failure".into(),
                })
                .into(),
            ),
            F::DeadlineExceeded,
        ),
        (
            crate::usecase::agent_session::AgentSessionInitialInstructionError::Store(
                (crate::domain::workflow::WorkflowError::NotFound("missing".into())).into(),
            ),
            F::NotFound,
        ),
        (
            crate::usecase::agent_session::AgentSessionInitialInstructionError::Store(
                (crate::domain::workflow::WorkflowError::UnauthorizedApprovalTarget(
                    "denied".into(),
                ))
                .into(),
            ),
            F::PermissionDenied,
        ),
        (
            crate::usecase::agent_session::AgentSessionInitialInstructionError::Store(
                (crate::domain::local_event::CommitBatchError::CapacityExceeded).into(),
            ),
            F::ResourceExhausted,
        ),
        (
            crate::usecase::agent_session::AgentSessionInitialInstructionError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: "failure".into(),
                })
                .into(),
            ),
            F::Internal,
        ),
        (
            crate::usecase::agent_session::AgentSessionInitialInstructionError::Store(
                (crate::domain::local_event::CommitBatchError::Corrupt {
                    correlation_id: "id".into(),
                })
                .into(),
            ),
            F::DataLoss,
        ),
        (
            crate::usecase::agent_session::AgentSessionInitialInstructionError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Cancelled,
                    message: "failure".into(),
                })
                .into(),
            ),
            F::Canceled,
        ),
    ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod usecase_agent_session_agent_session_test {
        #[test]
        fn test_失敗分類_全変種と委譲した理由を保持する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
        (crate::usecase::agent_session::AgentSessionUsecaseError::NotFound, F::NotFound),
        (
            crate::usecase::agent_session::AgentSessionUsecaseError::InvalidOperation,
            F::FailedPrecondition,
        ),
        (crate::usecase::agent_session::AgentSessionUsecaseError::Conflict, F::Aborted),
        (
            crate::usecase::agent_session::AgentSessionUsecaseError::ProviderSessionAlreadyOwned {
                agent_session_id: "session".into(),
            },
            F::FailedPrecondition,
        ),
        (crate::usecase::agent_session::AgentSessionUsecaseError::Unavailable, F::Unavailable),
        (crate::usecase::agent_session::AgentSessionUsecaseError::Corrupt, F::DataLoss),
        (
            crate::usecase::agent_session::AgentSessionUsecaseError::Store(
                (crate::domain::local_event::CommitBatchError::QueueBusy).into(),
            ),
            F::Unavailable,
        ),
        (
            crate::usecase::agent_session::AgentSessionUsecaseError::Store(
                (crate::domain::local_event::CommitBatchError::TreeHeadConflict).into(),
            ),
            F::Aborted,
        ),
        (
            crate::usecase::agent_session::AgentSessionUsecaseError::Store(
                (crate::domain::local_event::CommitBatchError::PayloadConflict).into(),
            ),
            F::FailedPrecondition,
        ),
        (
            crate::usecase::agent_session::AgentSessionUsecaseError::Store(
                (crate::domain::local_event::LocalEventQueryError::InvalidRequest).into(),
            ),
            F::InvalidArgument,
        ),
        (
            crate::usecase::agent_session::AgentSessionUsecaseError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                    message: "failure".into(),
                })
                .into(),
            ),
            F::DeadlineExceeded,
        ),
        (
            crate::usecase::agent_session::AgentSessionUsecaseError::Store(
                (crate::domain::workflow::WorkflowError::NotFound("missing".into())).into(),
            ),
            F::NotFound,
        ),
        (
            crate::usecase::agent_session::AgentSessionUsecaseError::Store(
                (crate::domain::workflow::WorkflowError::UnauthorizedApprovalTarget(
                    "denied".into(),
                ))
                .into(),
            ),
            F::PermissionDenied,
        ),
        (
            crate::usecase::agent_session::AgentSessionUsecaseError::Store(
                (crate::domain::local_event::CommitBatchError::CapacityExceeded).into(),
            ),
            F::ResourceExhausted,
        ),
        (
            crate::usecase::agent_session::AgentSessionUsecaseError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: "failure".into(),
                })
                .into(),
            ),
            F::Internal,
        ),
        (
            crate::usecase::agent_session::AgentSessionUsecaseError::Store(
                (crate::domain::local_event::CommitBatchError::Corrupt {
                    correlation_id: "id".into(),
                })
                .into(),
            ),
            F::DataLoss,
        ),
        (
            crate::usecase::agent_session::AgentSessionUsecaseError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Cancelled,
                    message: "failure".into(),
                })
                .into(),
            ),
            F::Canceled,
        ),
    ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod usecase_agent_session_agent_session_lifecycle_error_test {
        #[test]
        fn test_失敗分類_全変種と委譲した理由を保持する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
        (crate::usecase::agent_session::AgentSessionLifecycleUsecaseError::NotFound, F::NotFound),
        (
            crate::usecase::agent_session::AgentSessionLifecycleUsecaseError::InvalidOperation,
            F::FailedPrecondition,
        ),
        (
            crate::usecase::agent_session::AgentSessionLifecycleUsecaseError::Conflict(
                (crate::domain::local_event::CommitBatchError::TreeHeadConflict).into(),
            ),
            F::Aborted,
        ),
        (
            crate::usecase::agent_session::AgentSessionLifecycleUsecaseError::StorageUnavailable,
            F::Unavailable,
        ),
        (
            crate::usecase::agent_session::AgentSessionLifecycleUsecaseError::Launch(crate::domain::agent_session::ProviderAgentLaunchGatewayError::Technical(crate::domain::failure::TechnicalFailure { nature: crate::domain::failure::TechnicalFailureNature::Transient, message: "unavailable".into() })),
            F::Unavailable,
        ),
        (
            crate::usecase::agent_session::AgentSessionLifecycleUsecaseError::Terminal(crate::domain::agent_session::ProviderAgentTerminalGatewayError::Technical(crate::domain::failure::TechnicalFailure { nature: crate::domain::failure::TechnicalFailureNature::Transient, message: "unavailable".into() })),
            F::Unavailable,
        ),
        (crate::usecase::agent_session::AgentSessionLifecycleUsecaseError::Corrupt, F::DataLoss),
        (
            crate::usecase::agent_session::AgentSessionLifecycleUsecaseError::Store(
                (crate::domain::local_event::CommitBatchError::QueueBusy).into(),
            ),
            F::Unavailable,
        ),
        (
            crate::usecase::agent_session::AgentSessionLifecycleUsecaseError::Store(
                (crate::domain::local_event::CommitBatchError::TreeHeadConflict).into(),
            ),
            F::Aborted,
        ),
        (
            crate::usecase::agent_session::AgentSessionLifecycleUsecaseError::Store(
                (crate::domain::local_event::CommitBatchError::PayloadConflict).into(),
            ),
            F::FailedPrecondition,
        ),
        (
            crate::usecase::agent_session::AgentSessionLifecycleUsecaseError::Store(
                (crate::domain::local_event::LocalEventQueryError::InvalidRequest).into(),
            ),
            F::InvalidArgument,
        ),
        (
            crate::usecase::agent_session::AgentSessionLifecycleUsecaseError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                    message: "failure".into(),
                })
                .into(),
            ),
            F::DeadlineExceeded,
        ),
        (
            crate::usecase::agent_session::AgentSessionLifecycleUsecaseError::Store(
                (crate::domain::workflow::WorkflowError::NotFound("missing".into())).into(),
            ),
            F::NotFound,
        ),
        (
            crate::usecase::agent_session::AgentSessionLifecycleUsecaseError::Store(
                (crate::domain::workflow::WorkflowError::UnauthorizedApprovalTarget(
                    "denied".into(),
                ))
                .into(),
            ),
            F::PermissionDenied,
        ),
        (
            crate::usecase::agent_session::AgentSessionLifecycleUsecaseError::Store(
                (crate::domain::local_event::CommitBatchError::CapacityExceeded).into(),
            ),
            F::ResourceExhausted,
        ),
        (
            crate::usecase::agent_session::AgentSessionLifecycleUsecaseError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: "failure".into(),
                })
                .into(),
            ),
            F::Internal,
        ),
        (
            crate::usecase::agent_session::AgentSessionLifecycleUsecaseError::Store(
                (crate::domain::local_event::CommitBatchError::Corrupt {
                    correlation_id: "id".into(),
                })
                .into(),
            ),
            F::DataLoss,
        ),
        (
            crate::usecase::agent_session::AgentSessionLifecycleUsecaseError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Cancelled,
                    message: "failure".into(),
                })
                .into(),
            ),
            F::Canceled,
        ),
    ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod usecase_agent_session_agent_session_query_test {
        #[test]
        fn test_失敗分類_全変種と委譲した理由を保持する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
                (
                    crate::usecase::agent_session::AgentSessionQueryError::InvalidRequest,
                    F::InvalidArgument,
                ),
                (
                    crate::usecase::agent_session::AgentSessionQueryError::Unavailable,
                    F::Unavailable,
                ),
                (
                    crate::usecase::agent_session::AgentSessionQueryError::Corrupt,
                    F::DataLoss,
                ),
                (
                    crate::usecase::agent_session::AgentSessionQueryError::Store(
                        (crate::domain::local_event::CommitBatchError::QueueBusy).into(),
                    ),
                    F::Unavailable,
                ),
                (
                    crate::usecase::agent_session::AgentSessionQueryError::Store(
                        (crate::domain::local_event::CommitBatchError::TreeHeadConflict).into(),
                    ),
                    F::Aborted,
                ),
                (
                    crate::usecase::agent_session::AgentSessionQueryError::Store(
                        (crate::domain::local_event::CommitBatchError::PayloadConflict).into(),
                    ),
                    F::FailedPrecondition,
                ),
                (
                    crate::usecase::agent_session::AgentSessionQueryError::Store(
                        (crate::domain::local_event::LocalEventQueryError::InvalidRequest).into(),
                    ),
                    F::InvalidArgument,
                ),
                (
                    crate::usecase::agent_session::AgentSessionQueryError::Store(
                        (crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                            message: "failure".into(),
                        })
                        .into(),
                    ),
                    F::DeadlineExceeded,
                ),
                (
                    crate::usecase::agent_session::AgentSessionQueryError::Store(
                        (crate::domain::workflow::WorkflowError::NotFound("missing".into())).into(),
                    ),
                    F::NotFound,
                ),
                (
                    crate::usecase::agent_session::AgentSessionQueryError::Store(
                        (crate::domain::workflow::WorkflowError::UnauthorizedApprovalTarget(
                            "denied".into(),
                        ))
                        .into(),
                    ),
                    F::PermissionDenied,
                ),
                (
                    crate::usecase::agent_session::AgentSessionQueryError::Store(
                        (crate::domain::local_event::CommitBatchError::CapacityExceeded).into(),
                    ),
                    F::ResourceExhausted,
                ),
                (
                    crate::usecase::agent_session::AgentSessionQueryError::Store(
                        (crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::Other,
                            message: "failure".into(),
                        })
                        .into(),
                    ),
                    F::Internal,
                ),
                (
                    crate::usecase::agent_session::AgentSessionQueryError::Store(
                        (crate::domain::local_event::CommitBatchError::Corrupt {
                            correlation_id: "id".into(),
                        })
                        .into(),
                    ),
                    F::DataLoss,
                ),
                (
                    crate::usecase::agent_session::AgentSessionQueryError::Store(
                        (crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::Cancelled,
                            message: "failure".into(),
                        })
                        .into(),
                    ),
                    F::Canceled,
                ),
            ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod usecase_agent_session_agent_session_rename_test {
        #[test]
        fn test_失敗分類_全変種と委譲した理由を保持する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
            (
                crate::usecase::agent_session::AgentSessionRenameError::NotFound,
                F::NotFound,
            ),
            (
                crate::usecase::agent_session::AgentSessionRenameError::InvalidOperation,
                F::FailedPrecondition,
            ),
            (
                crate::usecase::agent_session::AgentSessionRenameError::Conflict,
                F::Aborted,
            ),
            (
                crate::usecase::agent_session::AgentSessionRenameError::ProviderSessionAlreadyOwned,
                F::FailedPrecondition,
            ),
            (
                crate::usecase::agent_session::AgentSessionRenameError::Unavailable,
                F::Unavailable,
            ),
            (
                crate::usecase::agent_session::AgentSessionRenameError::Corrupt,
                F::DataLoss,
            ),
            (
                crate::usecase::agent_session::AgentSessionRenameError::Store(
                    (crate::domain::local_event::CommitBatchError::QueueBusy).into(),
                ),
                F::Unavailable,
            ),
            (
                crate::usecase::agent_session::AgentSessionRenameError::Store(
                    (crate::domain::local_event::CommitBatchError::TreeHeadConflict).into(),
                ),
                F::Aborted,
            ),
            (
                crate::usecase::agent_session::AgentSessionRenameError::Store(
                    (crate::domain::local_event::CommitBatchError::PayloadConflict).into(),
                ),
                F::FailedPrecondition,
            ),
            (
                crate::usecase::agent_session::AgentSessionRenameError::Store(
                    (crate::domain::local_event::LocalEventQueryError::InvalidRequest).into(),
                ),
                F::InvalidArgument,
            ),
            (
                crate::usecase::agent_session::AgentSessionRenameError::Store(
                    (crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                        message: "failure".into(),
                    })
                    .into(),
                ),
                F::DeadlineExceeded,
            ),
            (
                crate::usecase::agent_session::AgentSessionRenameError::Store(
                    (crate::domain::workflow::WorkflowError::NotFound("missing".into())).into(),
                ),
                F::NotFound,
            ),
            (
                crate::usecase::agent_session::AgentSessionRenameError::Store(
                    (crate::domain::workflow::WorkflowError::UnauthorizedApprovalTarget(
                        "denied".into(),
                    ))
                    .into(),
                ),
                F::PermissionDenied,
            ),
            (
                crate::usecase::agent_session::AgentSessionRenameError::Store(
                    (crate::domain::local_event::CommitBatchError::CapacityExceeded).into(),
                ),
                F::ResourceExhausted,
            ),
            (
                crate::usecase::agent_session::AgentSessionRenameError::Store(
                    (crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::Other,
                        message: "failure".into(),
                    })
                    .into(),
                ),
                F::Internal,
            ),
            (
                crate::usecase::agent_session::AgentSessionRenameError::Store(
                    (crate::domain::local_event::CommitBatchError::Corrupt {
                        correlation_id: "id".into(),
                    })
                    .into(),
                ),
                F::DataLoss,
            ),
            (
                crate::usecase::agent_session::AgentSessionRenameError::Store(
                    (crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::Cancelled,
                        message: "failure".into(),
                    })
                    .into(),
                ),
                F::Canceled,
            ),
        ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod usecase_agent_session_agent_session_read_test {
        #[test]
        fn test_失敗分類_全変種と委譲した理由を保持する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
            (
                crate::usecase::agent_session::AgentSessionReadUsecaseError::InvalidRequest,
                F::InvalidArgument,
            ),
            (
                crate::usecase::agent_session::AgentSessionReadUsecaseError::StorageUnavailable,
                F::Unavailable,
            ),
            (
                crate::usecase::agent_session::AgentSessionReadUsecaseError::Lifecycle(crate::usecase::agent_session::AgentSessionLifecycleUsecaseError::Terminal(crate::domain::agent_session::ProviderAgentTerminalGatewayError::InvalidOperation("exited".into()))),
                F::FailedPrecondition,
            ),
            (
                crate::usecase::agent_session::AgentSessionReadUsecaseError::Corrupt,
                F::DataLoss,
            ),
            (
                crate::usecase::agent_session::AgentSessionReadUsecaseError::Store(
                    (crate::domain::local_event::CommitBatchError::QueueBusy).into(),
                ),
                F::Unavailable,
            ),
            (
                crate::usecase::agent_session::AgentSessionReadUsecaseError::Store(
                    (crate::domain::local_event::CommitBatchError::TreeHeadConflict).into(),
                ),
                F::Aborted,
            ),
            (
                crate::usecase::agent_session::AgentSessionReadUsecaseError::Store(
                    (crate::domain::local_event::CommitBatchError::PayloadConflict).into(),
                ),
                F::FailedPrecondition,
            ),
            (
                crate::usecase::agent_session::AgentSessionReadUsecaseError::Store(
                    (crate::domain::local_event::LocalEventQueryError::InvalidRequest).into(),
                ),
                F::InvalidArgument,
            ),
            (
                crate::usecase::agent_session::AgentSessionReadUsecaseError::Store(
                    (crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                        message: "failure".into(),
                    })
                    .into(),
                ),
                F::DeadlineExceeded,
            ),
            (
                crate::usecase::agent_session::AgentSessionReadUsecaseError::Store(
                    (crate::domain::workflow::WorkflowError::NotFound("missing".into())).into(),
                ),
                F::NotFound,
            ),
            (
                crate::usecase::agent_session::AgentSessionReadUsecaseError::Store(
                    (crate::domain::workflow::WorkflowError::UnauthorizedApprovalTarget(
                        "denied".into(),
                    ))
                    .into(),
                ),
                F::PermissionDenied,
            ),
            (
                crate::usecase::agent_session::AgentSessionReadUsecaseError::Store(
                    (crate::domain::local_event::CommitBatchError::CapacityExceeded).into(),
                ),
                F::ResourceExhausted,
            ),
            (
                crate::usecase::agent_session::AgentSessionReadUsecaseError::Store(
                    (crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::Other,
                        message: "failure".into(),
                    })
                    .into(),
                ),
                F::Internal,
            ),
            (
                crate::usecase::agent_session::AgentSessionReadUsecaseError::Store(
                    (crate::domain::local_event::CommitBatchError::Corrupt {
                        correlation_id: "id".into(),
                    })
                    .into(),
                ),
                F::DataLoss,
            ),
            (
                crate::usecase::agent_session::AgentSessionReadUsecaseError::Store(
                    (crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::Cancelled,
                        message: "failure".into(),
                    })
                    .into(),
                ),
                F::Canceled,
            ),
        ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod usecase_agent_session_agent_session_launch_test {
        #[test]
        fn test_失敗分類_全変種と委譲した理由を保持する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
            (
                crate::usecase::agent_session::AgentSessionLaunchUsecaseError::ProviderUnavailable,
                F::FailedPrecondition,
            ),
            (
                crate::usecase::agent_session::AgentSessionLaunchUsecaseError::InvalidInput,
                F::InvalidArgument,
            ),
            (
                crate::usecase::agent_session::AgentSessionLaunchUsecaseError::Conflict(
                    (crate::domain::local_event::CommitBatchError::TreeHeadConflict).into(),
                ),
                F::Aborted,
            ),
            (
                crate::usecase::agent_session::AgentSessionLaunchUsecaseError::StorageUnavailable,
                F::Unavailable,
            ),
            (
                crate::usecase::agent_session::AgentSessionLaunchUsecaseError::Launch(
                    crate::domain::agent_session::ProviderAgentLaunchGatewayError::Technical(
                        crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::Transient,
                            message: "unavailable".into(),
                        },
                    ),
                ),
                F::Unavailable,
            ),
            (
                crate::usecase::agent_session::AgentSessionLaunchUsecaseError::Terminal(
                    crate::domain::agent_session::ProviderAgentTerminalGatewayError::Technical(
                        crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::Transient,
                            message: "unavailable".into(),
                        },
                    ),
                ),
                F::Unavailable,
            ),
            (
                crate::usecase::agent_session::AgentSessionLaunchUsecaseError::Corrupt,
                F::DataLoss,
            ),
            (
                crate::usecase::agent_session::AgentSessionLaunchUsecaseError::Terminal(
                    crate::domain::agent_session::ProviderAgentTerminalGatewayError::OwnerConflict,
                ),
                F::FailedPrecondition,
            ),
            (
                crate::usecase::agent_session::AgentSessionLaunchUsecaseError::Terminal(
                    crate::domain::agent_session::ProviderAgentTerminalGatewayError::Technical(
                        crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::Other,
                            message: "pty".into(),
                        },
                    ),
                ),
                F::Internal,
            ),
            (
                crate::usecase::agent_session::AgentSessionLaunchUsecaseError::Terminal(
                    crate::domain::agent_session::ProviderAgentTerminalGatewayError::Technical(
                        crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::Other,
                            message: "spawn".into(),
                        },
                    ),
                ),
                F::Internal,
            ),
            (
                crate::usecase::agent_session::AgentSessionLaunchUsecaseError::Store(
                    (crate::domain::local_event::CommitBatchError::QueueBusy).into(),
                ),
                F::Unavailable,
            ),
            (
                crate::usecase::agent_session::AgentSessionLaunchUsecaseError::Store(
                    (crate::domain::local_event::CommitBatchError::TreeHeadConflict).into(),
                ),
                F::Aborted,
            ),
            (
                crate::usecase::agent_session::AgentSessionLaunchUsecaseError::Store(
                    (crate::domain::local_event::CommitBatchError::PayloadConflict).into(),
                ),
                F::FailedPrecondition,
            ),
            (
                crate::usecase::agent_session::AgentSessionLaunchUsecaseError::Store(
                    (crate::domain::local_event::LocalEventQueryError::InvalidRequest).into(),
                ),
                F::InvalidArgument,
            ),
            (
                crate::usecase::agent_session::AgentSessionLaunchUsecaseError::Store(
                    (crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                        message: "failure".into(),
                    })
                    .into(),
                ),
                F::DeadlineExceeded,
            ),
            (
                crate::usecase::agent_session::AgentSessionLaunchUsecaseError::Store(
                    (crate::domain::workflow::WorkflowError::NotFound("missing".into())).into(),
                ),
                F::NotFound,
            ),
            (
                crate::usecase::agent_session::AgentSessionLaunchUsecaseError::Store(
                    (crate::domain::workflow::WorkflowError::UnauthorizedApprovalTarget(
                        "denied".into(),
                    ))
                    .into(),
                ),
                F::PermissionDenied,
            ),
            (
                crate::usecase::agent_session::AgentSessionLaunchUsecaseError::Store(
                    (crate::domain::local_event::CommitBatchError::CapacityExceeded).into(),
                ),
                F::ResourceExhausted,
            ),
            (
                crate::usecase::agent_session::AgentSessionLaunchUsecaseError::Store(
                    (crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::Other,
                        message: "failure".into(),
                    })
                    .into(),
                ),
                F::Internal,
            ),
            (
                crate::usecase::agent_session::AgentSessionLaunchUsecaseError::Store(
                    (crate::domain::local_event::CommitBatchError::Corrupt {
                        correlation_id: "id".into(),
                    })
                    .into(),
                ),
                F::DataLoss,
            ),
            (
                crate::usecase::agent_session::AgentSessionLaunchUsecaseError::Store(
                    (crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::Cancelled,
                        message: "failure".into(),
                    })
                    .into(),
                ),
                F::Canceled,
            ),
        ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod usecase_agent_session_agent_session_history_test {
        #[test]
        fn test_失敗分類_全変種と委譲した理由を保持する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
                (
                    crate::usecase::agent_session::AgentSessionHistoryQueryError::InvalidRequest,
                    F::InvalidArgument,
                ),
                (
                    crate::usecase::agent_session::AgentSessionHistoryQueryError::Technical(
                        crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::Transient,
                            message: "unavailable".into(),
                        },
                    ),
                    F::Unavailable,
                ),
                (
                    crate::usecase::agent_session::AgentSessionHistoryQueryError::Corrupt,
                    F::DataLoss,
                ),
                (
                    crate::usecase::agent_session::AgentSessionHistoryQueryError::Store(
                        (crate::domain::local_event::CommitBatchError::QueueBusy).into(),
                    ),
                    F::Unavailable,
                ),
                (
                    crate::usecase::agent_session::AgentSessionHistoryQueryError::Store(
                        (crate::domain::local_event::CommitBatchError::TreeHeadConflict).into(),
                    ),
                    F::Aborted,
                ),
                (
                    crate::usecase::agent_session::AgentSessionHistoryQueryError::Store(
                        (crate::domain::local_event::CommitBatchError::PayloadConflict).into(),
                    ),
                    F::FailedPrecondition,
                ),
                (
                    crate::usecase::agent_session::AgentSessionHistoryQueryError::Store(
                        (crate::domain::local_event::LocalEventQueryError::InvalidRequest).into(),
                    ),
                    F::InvalidArgument,
                ),
                (
                    crate::usecase::agent_session::AgentSessionHistoryQueryError::Store(
                        (crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                            message: "failure".into(),
                        })
                        .into(),
                    ),
                    F::DeadlineExceeded,
                ),
                (
                    crate::usecase::agent_session::AgentSessionHistoryQueryError::Store(
                        (crate::domain::workflow::WorkflowError::NotFound("missing".into())).into(),
                    ),
                    F::NotFound,
                ),
                (
                    crate::usecase::agent_session::AgentSessionHistoryQueryError::Store(
                        (crate::domain::workflow::WorkflowError::UnauthorizedApprovalTarget(
                            "denied".into(),
                        ))
                        .into(),
                    ),
                    F::PermissionDenied,
                ),
                (
                    crate::usecase::agent_session::AgentSessionHistoryQueryError::Store(
                        (crate::domain::local_event::CommitBatchError::CapacityExceeded).into(),
                    ),
                    F::ResourceExhausted,
                ),
                (
                    crate::usecase::agent_session::AgentSessionHistoryQueryError::Store(
                        (crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::Other,
                            message: "failure".into(),
                        })
                        .into(),
                    ),
                    F::Internal,
                ),
                (
                    crate::usecase::agent_session::AgentSessionHistoryQueryError::Store(
                        (crate::domain::local_event::CommitBatchError::Corrupt {
                            correlation_id: "id".into(),
                        })
                        .into(),
                    ),
                    F::DataLoss,
                ),
                (
                    crate::usecase::agent_session::AgentSessionHistoryQueryError::Store(
                        (crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::Cancelled,
                            message: "failure".into(),
                        })
                        .into(),
                    ),
                    F::Canceled,
                ),
            ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod usecase_terminal_surface_error_test {
        #[test]
        fn test_失敗分類_全変種と委譲した理由を保持する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
                (
                    crate::usecase::terminal_surface::error::UsecaseError::Technical(
                        crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::Other,
                            message: "io".into(),
                        },
                    ),
                    F::Internal,
                ),
                (
                    crate::usecase::terminal_surface::error::UsecaseError::OwnerConflict,
                    F::FailedPrecondition,
                ),
                (
                    crate::usecase::terminal_surface::error::UsecaseError::Technical(
                        crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::Other,
                            message: "pty".into(),
                        },
                    ),
                    F::Internal,
                ),
                (
                    crate::usecase::terminal_surface::error::UsecaseError::Technical(
                        crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::Other,
                            message: "spawn".into(),
                        },
                    ),
                    F::Internal,
                ),
            ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod usecase_code_error_test {
        #[test]
        fn test_失敗分類_全変種と委譲した理由を保持する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
                (
                    crate::usecase::code_error::CodeUsecaseError::Code(
                        crate::domain::code::CodeError::External("io".into()),
                    ),
                    F::Internal,
                ),
                (
                    crate::usecase::code_error::CodeUsecaseError::Code(
                        crate::domain::code::CodeError::Rule("state".into()),
                    ),
                    F::FailedPrecondition,
                ),
                (
                    crate::usecase::code_error::CodeUsecaseError::Code(
                        crate::domain::code::CodeError::StaleReviewGroupTarget {
                            group_id: "g".into(),
                        },
                    ),
                    F::Aborted,
                ),
                (
                    crate::usecase::code_error::CodeUsecaseError::Code(
                        crate::domain::code::CodeError::StaleReviewGroupTarget {
                            group_id: "id".into(),
                        },
                    ),
                    F::Aborted,
                ),
            ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod usecase_repository_error_test {
        #[test]
        fn test_失敗分類_全変種と委譲した理由を保持する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
                (
                    crate::usecase::repository_error::UsecaseError::Repository(
                        crate::domain::repository::RepositoryError::External("io".into()),
                    ),
                    F::Internal,
                ),
                (
                    crate::usecase::repository_error::UsecaseError::Repository(
                        crate::domain::repository::RepositoryError::Rule("state".into()),
                    ),
                    F::FailedPrecondition,
                ),
                (
                    crate::usecase::repository_error::UsecaseError::Rule("state".into()),
                    F::FailedPrecondition,
                ),
            ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod usecase_watcher_test {
        #[test]
        fn test_失敗分類_全変種と委譲した理由を保持する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
                (
                    crate::usecase::watcher::UsecaseError::Repository(
                        crate::usecase::repository_state::RepositoryStateError::Watcher(
                            "io".into(),
                        ),
                    ),
                    F::Internal,
                ),
                (
                    crate::usecase::watcher::UsecaseError::File("io".into()),
                    F::Internal,
                ),
                (
                    crate::usecase::watcher::UsecaseError::RepositoryUnavailable,
                    F::FailedPrecondition,
                ),
            ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod domain_state_subscription_subscriptions_test {
        #[test]
        fn test_失敗分類_subscription_error_理由に対応する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
                (
                    crate::usecase::state_subscription::SubscriptionError::InvalidId,
                    F::InvalidArgument,
                ),
                (
                    crate::usecase::state_subscription::SubscriptionError::AlreadyExists,
                    F::AlreadyExists,
                ),
                (
                    crate::usecase::state_subscription::SubscriptionError::StreamEnded,
                    F::NotFound,
                ),
                (
                    crate::usecase::state_subscription::SubscriptionError::UnknownTarget,
                    F::NotFound,
                ),
                (
                    crate::usecase::state_subscription::SubscriptionError::VersionExhausted,
                    F::Internal,
                ),
                (
                    crate::usecase::state_subscription::SubscriptionError::EncodingFailed,
                    F::Internal,
                ),
                (
                    crate::usecase::state_subscription::SubscriptionError::SnapshotRequired,
                    F::NotFound,
                ),
            ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod domain_provider_lifecycle_repository_test {
        #[test]
        fn test_失敗分類_provider_lifecycle_repository_error_理由に対応する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
        (
            crate::domain::provider_lifecycle::ProviderLifecycleRepositoryError::InvalidInput,
            F::InvalidArgument,
        ),
        (
            crate::domain::provider_lifecycle::ProviderLifecycleRepositoryError::StorageUnavailable,
            F::Unavailable,
        ),
        (crate::domain::provider_lifecycle::ProviderLifecycleRepositoryError::Corrupt, F::DataLoss),
        (
            crate::domain::provider_lifecycle::ProviderLifecycleRepositoryError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                    message: "failure".into(),
                })
                .into(),
            ),
            F::DeadlineExceeded,
        ),
    ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod domain_provider_lifecycle_repository_test_2 {
        #[test]
        fn test_失敗分類_provider_hook_health_repository_error_理由に対応する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
        (
            crate::domain::provider_lifecycle::ProviderHookHealthRepositoryError::InvalidInput,
            F::InvalidArgument,
        ),
        (crate::domain::provider_lifecycle::ProviderHookHealthRepositoryError::Conflict, F::Aborted),
        (
            crate::domain::provider_lifecycle::ProviderHookHealthRepositoryError::StorageUnavailable,
            F::Unavailable,
        ),
        (crate::domain::provider_lifecycle::ProviderHookHealthRepositoryError::Corrupt, F::DataLoss),
        (
            crate::domain::provider_lifecycle::ProviderHookHealthRepositoryError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                    message: "failure".into(),
                })
                .into(),
            ),
            F::DeadlineExceeded,
        ),
    ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod domain_workflow_error_test {
        #[test]
        fn test_失敗分類_workflow_error_理由に対応する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
                (
                    crate::domain::workflow::error::WorkflowError::External("reason".into()),
                    F::Internal,
                ),
                (
                    crate::domain::workflow::error::WorkflowError::Editor(
                        crate::domain::external_editor::EditorError::Launch("launch".into()),
                    ),
                    F::FailedPrecondition,
                ),
                (
                    crate::domain::workflow::error::WorkflowError::Editor(
                        crate::domain::external_editor::EditorError::InvalidInput("input".into()),
                    ),
                    F::InvalidArgument,
                ),
                (
                    crate::domain::workflow::error::WorkflowError::Editor(
                        crate::domain::external_editor::EditorError::Settings(
                            crate::domain::app_config::AppConfigError::Repository(
                                "settings".into(),
                            ),
                        ),
                    ),
                    F::Internal,
                ),
                (
                    crate::domain::workflow::error::WorkflowError::CorruptStoredState(
                        "reason".into(),
                    ),
                    F::DataLoss,
                ),
                (
                    crate::domain::workflow::error::WorkflowError::IncompatibleStoredEvent(
                        "reason".into(),
                    ),
                    F::FailedPrecondition,
                ),
                (
                    crate::domain::workflow::error::WorkflowError::InvalidState("reason".into()),
                    F::FailedPrecondition,
                ),
                (
                    crate::domain::workflow::error::WorkflowError::Validation("reason".into()),
                    F::InvalidArgument,
                ),
                (
                    crate::domain::workflow::error::WorkflowError::Conflict("reason".into()),
                    F::Aborted,
                ),
                (
                    crate::domain::workflow::error::WorkflowError::NotFound("reason".into()),
                    F::NotFound,
                ),
                (
                    crate::domain::workflow::error::WorkflowError::UnauthorizedApprovalTarget(
                        "reason".into(),
                    ),
                    F::PermissionDenied,
                ),
                (
                    crate::domain::workflow::error::WorkflowError::Store(
                        (crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                            message: "failure".into(),
                        })
                        .into(),
                    ),
                    F::DeadlineExceeded,
                ),
                (
                    crate::domain::workflow::error::WorkflowError::Store(
                        crate::domain::failure::StorageFailure::from(
                            crate::domain::local_event::CommitBatchError::QueueBusy,
                        )
                        .with_message("busy"),
                    ),
                    F::Unavailable,
                ),
                (
                    crate::domain::workflow::error::WorkflowError::Store(
                        crate::domain::failure::StorageFailure::from(
                            crate::domain::local_event::CommitBatchError::PayloadConflict,
                        )
                        .with_message("repair"),
                    ),
                    F::FailedPrecondition,
                ),
            ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod domain_notion_error_test {
        #[test]
        fn test_失敗分類_notion_error_理由に対応する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
                (
                    crate::domain::notion::error::NotionError::Technical(
                        crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                            message: "Operation deadline exceeded".into(),
                        },
                    ),
                    F::DeadlineExceeded,
                ),
                (
                    crate::domain::notion::error::NotionError::Technical(
                        crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::Cancelled,
                            message: "Operation cancelled".into(),
                        },
                    ),
                    F::Canceled,
                ),
                (
                    crate::domain::notion::error::NotionError::RequestFailed("network".into()),
                    F::Unavailable,
                ),
                (
                    crate::domain::notion::error::NotionError::ApiError("api".into()),
                    F::FailedPrecondition,
                ),
                (
                    crate::domain::notion::error::NotionError::ParseError("json".into()),
                    F::Internal,
                ),
            ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod domain_local_event_batch_test {
        #[test]
        fn test_失敗分類_commit_batch_error_理由に対応する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
            (
                crate::domain::local_event::batch::CommitBatchError::PayloadConflict,
                F::FailedPrecondition,
            ),
            (
                crate::domain::local_event::batch::CommitBatchError::TreeHeadConflict,
                F::Aborted,
            ),
            (
                crate::domain::local_event::batch::CommitBatchError::AppendOutcomeUnknown,
                F::Aborted,
            ),
            (
                crate::domain::local_event::batch::CommitBatchError::QueueBusy,
                F::Unavailable,
            ),
            (
                crate::domain::local_event::batch::CommitBatchError::StreamHeadConflict {
                    current: crate::domain::local_event::StreamVersion::new(1).unwrap(),
                },
                F::Aborted,
            ),
            (
                crate::domain::local_event::batch::CommitBatchError::OutcomeUnknown {
                    identity: crate::domain::local_event::CommitIdentity::parse("commit").unwrap(),
                },
                F::Aborted,
            ),
            (
                crate::domain::local_event::batch::CommitBatchError::CapacityExceeded,
                F::ResourceExhausted,
            ),
            (
                crate::domain::local_event::batch::CommitBatchError::SequenceExhausted,
                F::ResourceExhausted,
            ),
            (
                crate::domain::local_event::batch::CommitBatchError::Corrupt {
                    correlation_id: "id".into(),
                },
                F::DataLoss,
            ),
            (
                crate::domain::local_event::batch::CommitBatchError::StorageUnavailable {
                    failure: crate::domain::local_event::SafeOperationFailure::new(
                        crate::domain::local_event::SessionOperationFailureKind::StorageUnavailable,
                        crate::domain::failure::TechnicalFailureNature::TimedOut,
                        "busy",
                        "id",
                    ),
                },
                F::DeadlineExceeded,
            ),
        ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod domain_local_event_query_test {
        #[test]
        fn test_失敗分類_local_event_query_error_理由に対応する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
            (
                crate::domain::local_event::query::LocalEventQueryError::InvalidRequest,
                F::InvalidArgument,
            ),
            (
                crate::domain::local_event::query::LocalEventQueryError::QueryBusy,
                F::Unavailable,
            ),
            (
                crate::domain::local_event::query::LocalEventQueryError::Technical(
                    crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                        message: "deadline exceeded".into(),
                    },
                ),
                F::DeadlineExceeded,
            ),
            (
                crate::domain::local_event::query::LocalEventQueryError::ResponseTooLarge,
                F::ResourceExhausted,
            ),
            (
                crate::domain::local_event::query::LocalEventQueryError::IncompatibleStoredEvent {
                    correlation_id: "id".into(),
                },
                F::FailedPrecondition,
            ),
            (
                crate::domain::local_event::query::LocalEventQueryError::Corrupt {
                    correlation_id: "id".into(),
                },
                F::DataLoss,
            ),
            (
                crate::domain::local_event::query::LocalEventQueryError::Internal {
                    correlation_id: "id".into(),
                },
                F::Internal,
            ),
            (
                crate::domain::local_event::query::LocalEventQueryError::StorageUnavailable {
                    failure: crate::domain::local_event::SafeOperationFailure::new(
                        crate::domain::local_event::SessionOperationFailureKind::StorageUnavailable,
                        crate::domain::failure::TechnicalFailureNature::TimedOut,
                        "busy",
                        "id",
                    ),
                },
                F::DeadlineExceeded,
            ),
        ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod domain_external_editor_gateway_test {
        #[test]
        fn test_失敗分類_editor_error_理由に対応する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
                (
                    crate::domain::external_editor::gateway::EditorError::InvalidInput(
                        "input".into(),
                    ),
                    F::InvalidArgument,
                ),
                (
                    crate::domain::external_editor::gateway::EditorError::Launch("launch".into()),
                    F::FailedPrecondition,
                ),
                (
                    crate::domain::external_editor::gateway::EditorError::Settings(
                        crate::domain::app_config::AppConfigError::Repository("store".into()),
                    ),
                    F::Internal,
                ),
                (
                    crate::domain::external_editor::gateway::EditorError::Settings(
                        crate::domain::app_config::AppConfigError::InvalidInput("input".into()),
                    ),
                    F::InvalidArgument,
                ),
            ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod domain_code_error_test {
        #[test]
        fn test_失敗分類_code_error_理由に対応する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
                (
                    crate::domain::code::error::CodeError::Technical(
                        crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                            message: "Operation deadline exceeded".into(),
                        },
                    ),
                    F::DeadlineExceeded,
                ),
                (
                    crate::domain::code::error::CodeError::Technical(
                        crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::Cancelled,
                            message: "Operation cancelled".into(),
                        },
                    ),
                    F::Canceled,
                ),
                (
                    crate::domain::code::error::CodeError::External("io".into()),
                    F::Internal,
                ),
                (
                    crate::domain::code::error::CodeError::Rule("rule".into()),
                    F::FailedPrecondition,
                ),
                (
                    crate::domain::code::error::CodeError::StaleReviewGroupTarget {
                        group_id: "g".into(),
                    },
                    F::Aborted,
                ),
                (
                    crate::domain::code::error::CodeError::StaleReviewGroupTarget {
                        group_id: "group".into(),
                    },
                    F::Aborted,
                ),
            ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod domain_app_config_error_test {
        #[test]
        fn test_失敗分類_app_config_error_理由に対応する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
                (
                    crate::domain::app_config::error::AppConfigError::Repository("store".into()),
                    F::Internal,
                ),
                (
                    crate::domain::app_config::error::AppConfigError::InvalidInput("input".into()),
                    F::InvalidArgument,
                ),
            ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod domain_comment_mod_test {
        #[test]
        fn test_失敗分類_review_error_理由に対応する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
                (
                    crate::domain::comment::ReviewError::Technical(
                        crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                            message: "Operation deadline exceeded".into(),
                        },
                    ),
                    F::DeadlineExceeded,
                ),
                (
                    crate::domain::comment::ReviewError::Technical(
                        crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::Cancelled,
                            message: "Operation cancelled".into(),
                        },
                    ),
                    F::Canceled,
                ),
                (
                    crate::domain::comment::ReviewError::InvalidInput("reason".into()),
                    F::InvalidArgument,
                ),
                (
                    crate::domain::comment::ReviewError::NotFound("reason".into()),
                    F::NotFound,
                ),
                (
                    crate::domain::comment::ReviewError::AlreadyResolved("reason".into()),
                    F::FailedPrecondition,
                ),
                (
                    crate::domain::comment::ReviewError::PermissionDenied("reason".into()),
                    F::PermissionDenied,
                ),
                (
                    crate::domain::comment::ReviewError::Io("reason".into()),
                    F::Internal,
                ),
                (
                    crate::domain::comment::ReviewError::Serialize("reason".into()),
                    F::Internal,
                ),
            ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod domain_git_host_git_host_test {
        #[test]
        fn test_失敗分類_内部失敗を保持する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode;
            // Given
            let error = crate::domain::git_host::git_host::GitHostError::External("failure".into());
            // When / Then
            assert_eq!(error.connect_code(), ErrorCode::Internal);
        }
    }

    mod domain_agent_session_repository_test {
        #[test]
        fn test_失敗分類_agent_session_repository_error_理由に対応する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
        (crate::domain::agent_session::repository::AgentSessionRepositoryError::Conflict, F::Aborted),
        (
            crate::domain::agent_session::repository::AgentSessionRepositoryError::ProviderSessionAlreadyOwned {
                agent_session_id: "session".into(),
            },
            F::FailedPrecondition,
        ),
        (
            crate::domain::agent_session::repository::AgentSessionRepositoryError::InvalidRequest,
            F::InvalidArgument,
        ),
        (crate::domain::agent_session::repository::AgentSessionRepositoryError::Corrupt, F::DataLoss),
        (crate::domain::agent_session::repository::AgentSessionRepositoryError::Unavailable, F::Unavailable),
        (
            crate::domain::agent_session::repository::AgentSessionRepositoryError::Store(
                (crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                    message: "failure".into(),
                })
                .into(),
            ),
            F::DeadlineExceeded,
        ),
    ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod domain_agent_session_provider_history_gateway_test {
        #[test]
        fn test_失敗分類_agent_session_history_gateway_error_理由に対応する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
                (
                    crate::domain::agent_session::AgentSessionHistoryGatewayError::InvalidRequest,
                    F::InvalidArgument,
                ),
                (
                    crate::domain::agent_session::AgentSessionHistoryGatewayError::Technical(
                        crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::Transient,
                            message: "unavailable".into(),
                        },
                    ),
                    F::Unavailable,
                ),
                (
                    crate::domain::agent_session::AgentSessionHistoryGatewayError::Corrupt,
                    F::DataLoss,
                ),
                (
                    crate::domain::agent_session::AgentSessionHistoryGatewayError::Store(
                        (crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                            message: "failure".into(),
                        })
                        .into(),
                    ),
                    F::DeadlineExceeded,
                ),
            ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod domain_repository_error_test {
        #[test]
        fn test_失敗分類_repository_error_理由に対応する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [
                (
                    crate::domain::repository::error::RepositoryError::Technical(
                        crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                            message: "Operation deadline exceeded".into(),
                        },
                    ),
                    F::DeadlineExceeded,
                ),
                (
                    crate::domain::repository::error::RepositoryError::Technical(
                        crate::domain::failure::TechnicalFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::Cancelled,
                            message: "Operation cancelled".into(),
                        },
                    ),
                    F::Canceled,
                ),
                (
                    crate::domain::repository::error::RepositoryError::External("io".into()),
                    F::Internal,
                ),
                (
                    crate::domain::repository::error::RepositoryError::Rule("rule".into()),
                    F::FailedPrecondition,
                ),
            ];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod domain_workspace_state_error_test {
        #[test]
        fn test_失敗分類_workspace_state_error_理由に対応する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode as F;
            // Given
            let cases = [(
                crate::domain::workspace_state::error::WorkspaceStateError::Message("io".into()),
                F::Internal,
            )];
            for (error, expected) in cases {
                // When / Then
                assert_eq!(error.connect_code(), expected, "{error:?}");
            }
        }
    }

    mod domain_application_lifecycle_test {
        #[test]
        fn test_失敗分類_内部失敗を保持する() {
            use crate::adaptor::presenter::connect::ConnectFailure;
            use connectrpc::ErrorCode;
            // Given
            let error =
                crate::domain::application_lifecycle::ApplicationLifecycleError("failure".into());
            // When / Then
            assert_eq!(error.connect_code(), ErrorCode::Internal);
        }
    }

    mod review_stopped_transfer_tests {
        use crate::adaptor::presenter::connect::classified_error;
        use crate::adaptor::presenter::error::AppError;
        use crate::domain::comment::ReviewError;

        #[test]
        fn test_review停止_転送コードを保ちメッセージをjsonで包まない() {
            use crate::common::operation_context::OperationStopped;
            // Given
            for (stopped, code) in [
                (
                    OperationStopped::Expired,
                    connectrpc::ErrorCode::DeadlineExceeded,
                ),
                (OperationStopped::Cancelled, connectrpc::ErrorCode::Canceled),
            ] {
                let message = stopped.to_string();
                // When
                let app_error = AppError::from_failure(ReviewError::Technical(stopped.into()));
                // Then
                assert_eq!(
                    serde_json::to_value(&app_error).unwrap(),
                    serde_json::json!(message)
                );
                let error = classified_error(app_error);
                assert_eq!(error.code, code);
                assert_eq!(error.message.as_deref(), Some(message.as_str()));
            }
        }
    }

    mod terminal_input_tests {
        use crate::adaptor::presenter::connect::ConnectFailure;
        use crate::usecase::terminal_surface::error::UsecaseError;

        #[test]
        fn test_入力失敗_失効attachmentをfailed_preconditionにする() {
            // Given
            let error = UsecaseError::StaleAttachment;
            // When
            let code = error.connect_code();
            // Then
            assert_eq!(code, connectrpc::ErrorCode::FailedPrecondition);
        }
    }

    #[test]
    fn test_技術的失敗_依存先の全ての性質でステータスを決める() {
        use crate::adaptor::presenter::connect::ConnectFailure;
        use crate::domain::agent_session::{
            ProviderAgentLaunchGatewayError, ProviderAgentTerminalGatewayError,
        };
        use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
        use crate::usecase::agent_session::{
            AgentSessionLaunchUsecaseError, AgentSessionLifecycleUsecaseError,
        };
        for (nature, code) in [
            (
                TechnicalFailureNature::Transient,
                connectrpc::ErrorCode::Unavailable,
            ),
            (
                TechnicalFailureNature::TimedOut,
                connectrpc::ErrorCode::DeadlineExceeded,
            ),
            (
                TechnicalFailureNature::Cancelled,
                connectrpc::ErrorCode::Canceled,
            ),
            (
                TechnicalFailureNature::Other,
                connectrpc::ErrorCode::Internal,
            ),
        ] {
            // Given
            let failure = TechnicalFailure {
                nature,
                message: "original failure".into(),
            };
            // When / Then
            assert_eq!(
                AgentSessionLaunchUsecaseError::Launch(ProviderAgentLaunchGatewayError::Technical(
                    failure.clone()
                ))
                .connect_code(),
                code
            );
            assert_eq!(
                AgentSessionLifecycleUsecaseError::Launch(
                    ProviderAgentLaunchGatewayError::Technical(failure.clone())
                )
                .connect_code(),
                code
            );
            assert_eq!(
                AgentSessionLaunchUsecaseError::Terminal(
                    ProviderAgentTerminalGatewayError::Technical(failure.clone())
                )
                .connect_code(),
                code
            );
            assert_eq!(
                AgentSessionLifecycleUsecaseError::Terminal(
                    ProviderAgentTerminalGatewayError::Technical(failure.clone())
                )
                .connect_code(),
                code
            );
            assert_eq!(
                AgentSessionLaunchUsecaseError::Terminal(
                    ProviderAgentTerminalGatewayError::Technical(failure.clone())
                )
                .connect_code(),
                code
            );
            assert_eq!(
                crate::usecase::terminal_surface::error::UsecaseError::Technical(failure)
                    .connect_code(),
                code
            );
        }
        assert_eq!(
            ProviderAgentLaunchGatewayError::InvalidInput.connect_code(),
            connectrpc::ErrorCode::InvalidArgument
        );
        assert_eq!(
            crate::usecase::terminal_surface::error::UsecaseError::NotFound("missing".into())
                .connect_code(),
            connectrpc::ErrorCode::NotFound
        );
    }
}

#[test]
fn test_失敗の分類_domainと読取失敗を同じstatusへ写す() {
    use crate::domain::failure::{
        BusinessFailure, Failure, TechnicalFailure, TechnicalFailureNature,
    };
    use crate::usecase::state_subscription::StateReadError;
    use crate::usecase::watcher::UsecaseError;
    let error = UsecaseError::RepositoryUnavailable;
    assert_eq!(
        error.connect_code(),
        connectrpc::ErrorCode::FailedPrecondition
    );
    let read = StateReadError::from_error(error);
    assert_eq!(
        Failure::Business(BusinessFailure::Other).connect_code(),
        read.connect_code()
    );
    for nature in [
        TechnicalFailureNature::Transient,
        TechnicalFailureNature::TimedOut,
        TechnicalFailureNature::Cancelled,
        TechnicalFailureNature::Other,
    ] {
        let read = StateReadError::from_error(TechnicalFailure {
            nature,
            message: "read failed".into(),
        });
        assert_eq!(
            Failure::Technical(nature).connect_code(),
            read.connect_code()
        );
    }
    let error = crate::domain::notion::NotionError::RequestFailed("request failed".into());
    assert_eq!(Failure::from(&error).connect_code(), error.connect_code());
}
