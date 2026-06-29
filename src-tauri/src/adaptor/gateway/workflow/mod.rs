//! Workflow gateway implementations for the clean architecture ports.
//!
//! These adapters intentionally preserve the existing workflow persistence
//! formats (`workflow_runs/`, `workflow_logs/`, workflow YAML, facet markdown,
//! and pending command files). Controller wiring moves to these ports in #1037.

pub(crate) mod approval_runtime;
mod archive_repository;
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
pub(crate) mod event_log_query;
pub(crate) mod event_log_writer;
pub(crate) mod event_projection;
mod event_repository;
pub(crate) mod execution_registry;
pub(crate) mod external_execution_restore;
pub(crate) mod facet;
mod facet_repository;
pub(crate) mod failure_policy_config;
mod failure_wire;
pub(crate) mod internal_node_command;
pub(crate) mod log;
pub(crate) mod mapper;
pub(crate) mod orphan_recovery;
#[cfg(test)]
mod output_limit;
pub(crate) mod output_submission;
pub(crate) mod parallel_runtime;
pub(crate) mod pending_command;
pub(crate) mod pending_command_dispatcher;
mod pending_command_watcher;
mod pending_repository;
pub(crate) mod pending_runtime;
pub(crate) mod prompt_rendering;
pub(crate) mod resolver;
pub(crate) mod route_context;
pub(crate) mod run;
mod run_repository;
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
pub(crate) mod state;
mod state_notification_gateway;
mod state_projection_repository;
mod step_detail_projection_repository;
mod step_lifecycle_adapters;
mod step_session_boundary;
pub(crate) mod step_settings;
pub(crate) mod storage;
#[cfg(test)]
pub(crate) mod test_support;
pub(crate) mod turn_completion;
mod workspace_session;
mod worktree_gateway;

pub(crate) use archive_repository::WorkflowRunArchiveFileRepository;
pub(crate) use config_path_gateway::WorkflowConfigPathFileGateway;
pub(crate) use definition_repository::WorkflowDefinitionFileRepository;
pub(crate) use diagnostics_gateway::WorkflowDiagnosticsFileGateway;
#[cfg(test)]
pub(crate) use editor_gateway::NoopWorkflowExternalEditorGateway;
pub(crate) use editor_gateway::TauriWorkflowExternalEditorGateway;
pub(crate) use event_repository::WorkflowEventLogRepository;
pub(crate) use facet_repository::WorkflowFacetFileRepository;
pub(crate) use pending_command_watcher::spawn_pending_command_watcher;
pub(crate) use pending_repository::{
    process_pending_workflow_command_entry, PendingWorkflowCommandFileRepository,
};
pub(crate) use run_repository::WorkflowRunFileRepository;
pub(crate) use runtime_command_gateway::TauriWorkflowRuntimeCommandGateway;
#[cfg(test)]
pub(crate) use secret_source_gateway::EmptySecretSourceGateway;
pub(crate) use secret_source_gateway::WorkflowSecretSourceConfigGateway;
#[cfg(test)]
pub(crate) use state_notification_gateway::build_workflow_state_projection_from_snapshot;
pub(crate) use state_notification_gateway::{
    build_workflow_state_view_from_snapshot, emit_workflow_state_from_snapshot,
    emit_workflow_state_snapshot,
};
pub(crate) use state_projection_repository::WorkflowStateProjectionLogRepository;
pub(crate) use step_detail_projection_repository::WorkflowStepDetailProjectionLogRepository;
#[cfg(test)]
pub(crate) use step_lifecycle_adapters::close_step_session_tab_state;
pub(crate) use step_lifecycle_adapters::{
    hydrate_open_workflow_step_tabs, mark_started_step_tab_open, release_step_runtime_on_done,
    TauriWorkflowStepLifecycleGateway,
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
