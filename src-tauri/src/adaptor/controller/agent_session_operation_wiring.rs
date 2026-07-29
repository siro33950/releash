//! Compatibility re-exports for controller composition.
//!
//! Runtime drivers live in the usecase layer; canonical payload codecs live
//! in `adaptor::gateway::agent_session::operation`.

pub(crate) use crate::adaptor::gateway::agent_session::operation::{
    CanonicalSendCommandV1, CanonicalSendTargetV1,
};
pub(crate) use crate::usecase::agent_session::operation::runtime_adapter::{
    ActiveSendRecoveryContext, ConservativeRecoveryExecutor, RuntimeAgentSessionOperationAdapter,
    RuntimePermissionResponseOperationAdapter, RuntimeSendOperationAdapter,
};
pub(crate) use crate::usecase::agent_session::operation::{
    bind_runtime_durable_stop_driver, bind_runtime_terminal_operation_participant_provider,
    LOCAL_INSTALLATION_OPERATION_PRINCIPAL,
};

pub(crate) fn bind_runtime_durable_workflow_send_driver(
    runtime: &std::sync::Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    operation: std::sync::Arc<crate::usecase::agent_session::operation::AgentSendOperationUsecase>,
    session_store: std::sync::Arc<crate::usecase::agent_session::session::SessionStore>,
    data_dir: std::path::PathBuf,
) {
    crate::usecase::agent_session::operation::bind_runtime_durable_workflow_send_driver(
        runtime,
        operation,
        session_store,
        data_dir,
        std::sync::Arc::new(
            crate::adaptor::gateway::agent_session::operation::CanonicalWorkflowSendPayloadEncoder,
        ),
    );
}
