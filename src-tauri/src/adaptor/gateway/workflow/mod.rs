//! Workflow gateway implementations for the clean architecture ports.
//!
//! Workflow definitions, diagnostics, and facets remain file-backed. Runtime
//! execution state and events use the fixed SQLite local event store.

pub(crate) mod approval_runtime;
pub(crate) mod builtin;
mod config_path_gateway;
mod definition_repository;
pub(crate) mod diagnostics;
mod diagnostics_gateway;
pub(crate) mod domain_mapping;
mod editor_gateway;
pub(crate) mod event;
pub(crate) mod event_log_writer;
pub(crate) mod event_projection;
mod event_repository;
mod execution_archive_repository;
mod execution_projection_repository;
pub(crate) mod execution_registry;
mod execution_repository;
pub(crate) mod execution_store;
pub(crate) mod facet;
mod facet_repository;
pub(crate) mod failure_policy_config;
pub(crate) mod failure_wire;
pub(crate) mod fanout_runtime;
pub(crate) mod internal_node_command;
pub(crate) mod log;
pub(crate) mod mapper;
mod node_lifecycle_adapters;
pub(crate) mod node_session_boundary;
pub(crate) mod node_settings;
pub(crate) mod orphan_recovery;
pub(crate) mod output_limit;
pub(crate) mod output_submission;
pub(crate) mod prompt_rendering;
pub(crate) mod resolver;
pub(crate) mod resume_projection;
mod runtime_command_gateway;
pub(crate) mod runtime_commit;
pub(crate) mod runtime_error;
pub(crate) mod runtime_events;
pub(crate) mod runtime_executor;
pub(crate) mod runtime_operations;
pub(crate) mod runtime_resolver;
pub(crate) mod runtime_session;
pub(crate) mod runtime_start_guard;
pub(crate) mod runtime_state;
pub(crate) mod schema;
pub(crate) mod secret_source;
mod secret_source_gateway;
pub(crate) mod span_map;
pub(crate) mod state;
mod state_notification_gateway;
pub(crate) mod storage;
#[cfg(test)]
pub(crate) mod test_support;
pub(crate) mod turn_completion;
mod workspace_session;
mod worktree_gateway;

pub(crate) use config_path_gateway::WorkflowConfigPathFileGateway;
pub(crate) use definition_repository::{
    WorkflowDefinitionFileRepository, WorkflowDefinitionFileSourceGateway,
};
pub(crate) use diagnostics_gateway::WorkflowDiagnosticsFileGateway;
#[cfg(test)]
pub(crate) use editor_gateway::NoopWorkflowExternalEditorGateway;
pub(crate) use editor_gateway::TauriWorkflowExternalEditorGateway;
pub(crate) use event_repository::WorkflowEventLogRepository;
pub(crate) use execution_archive_repository::WorkflowExecutionArchiveFileRepository;
pub(crate) use execution_projection_repository::WorkflowExecutionProjectionLogRepository;
pub(crate) use execution_repository::WorkflowExecutionFileRepository;
pub(crate) use facet_repository::WorkflowFacetFileRepository;
#[cfg(test)]
pub(crate) use node_lifecycle_adapters::close_node_session_tab_state;
pub(crate) use node_lifecycle_adapters::{
    mark_started_node_tab_open, release_node_runtime_on_done, TauriNodeExecutionLifecycleGateway,
};
pub(crate) use runtime_command_gateway::{
    TauriWorkflowRuntimeCommandGateway, TauriWorkflowRuntimeCommandGatewayDeps,
};
#[cfg(test)]
pub(crate) use runtime_resolver::resolve_workflow_by_name;
#[cfg(test)]
pub(crate) use secret_source_gateway::EmptySecretSourceGateway;
pub(crate) use secret_source_gateway::WorkflowSecretSourceConfigGateway;
pub(crate) use state_notification_gateway::emit_workflow_execution_from_snapshot;
pub(crate) use workspace_session::{
    DurableWorkspaceNodeSessionCloseGateway, StoredWorkspaceSessionGateway,
};
#[cfg(test)]
pub(crate) use worktree_gateway::PassthroughManagedWorktreeGateway;
pub(crate) use worktree_gateway::RepoPathsManagedWorktreeGateway;
pub(crate) use worktree_gateway::RepositoryManagedWorktreeGateway;
