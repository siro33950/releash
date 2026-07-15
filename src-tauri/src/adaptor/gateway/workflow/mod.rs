//! Workflow gateway implementations for the clean architecture ports.
//!
//! These adapters intentionally preserve the existing workflow persistence
//! formats (`workflow_executions/`, `workflow_execution_logs/`, workflow YAML, facet markdown,
//! execution event logs and workflow YAML/facet markdown.

pub(crate) mod approval_runtime;
pub(crate) mod builtin;
mod config_path_gateway;
mod definition_repository;
mod diagnostics;
mod diagnostics_gateway;
pub(crate) mod domain_mapping;
mod editor_gateway;
pub(crate) mod engine_error;
pub(crate) mod engine_start_guard;
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
mod failure_wire;
pub(crate) mod internal_node_command;
pub(crate) mod log;
pub(crate) mod mapper;
pub(crate) mod orphan_recovery;
pub(crate) mod output_limit;
pub(crate) mod output_submission;
pub(crate) mod parallel_runtime;
pub(crate) mod prompt_rendering;
pub(crate) mod resolver;
pub(crate) mod resume_projection;
mod runtime_command_gateway;
pub(crate) mod runtime_commit;
pub(crate) mod runtime_engine;
pub(crate) mod runtime_engine_impl;
pub(crate) mod runtime_events;
mod runtime_resolver;
mod runtime_session;
pub(crate) mod runtime_state;
pub(crate) mod schema;
pub(crate) mod secret_source;
mod secret_source_gateway;
pub(crate) mod span_map;
pub(crate) mod state;
mod state_notification_gateway;
mod step_lifecycle_adapters;
mod step_session_boundary;
pub(crate) mod step_settings;
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
pub(crate) use runtime_command_gateway::{
    TauriWorkflowRuntimeCommandGateway, TauriWorkflowRuntimeCommandGatewayDeps,
};
#[cfg(test)]
pub(crate) use runtime_resolver::resolve_workflow_by_name;
#[cfg(test)]
pub(crate) use secret_source_gateway::EmptySecretSourceGateway;
pub(crate) use secret_source_gateway::WorkflowSecretSourceConfigGateway;
pub(crate) use state_notification_gateway::emit_workflow_execution_from_snapshot;
#[cfg(test)]
pub(crate) use step_lifecycle_adapters::close_step_session_tab_state;
pub(crate) use step_lifecycle_adapters::{
    mark_started_step_tab_open, release_step_runtime_on_done, TauriNodeExecutionLifecycleGateway,
};
#[cfg(test)]
pub(crate) use step_lifecycle_adapters::{
    open_step_session_tab_state, resolve_step_session_with_data_dir,
};
pub(crate) use workspace_session::StoredWorkspaceSessionGateway;
#[cfg(test)]
pub(crate) use worktree_gateway::PassthroughManagedWorktreeGateway;
pub(crate) use worktree_gateway::RepoPathsManagedWorktreeGateway;
pub(crate) use worktree_gateway::RepositoryManagedWorktreeGateway;
