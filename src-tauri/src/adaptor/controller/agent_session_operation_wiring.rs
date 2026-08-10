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
