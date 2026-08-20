//! Workflow gateway implementations for the clean architecture ports.
//!
//! Workflow definitions, diagnostics, and facets remain file-backed. Runtime
//! execution state and events use the fixed SQLite local event store.

pub(crate) mod builtin;
mod config_path_gateway;
mod definition_repository;
pub(crate) mod diagnostics;
mod diagnostics_gateway;
pub(crate) mod domain_mapping;
mod editor_gateway;
pub(crate) mod event;
pub(crate) mod event_log_writer;
mod event_repository;
mod execution_archive_repository;
mod execution_projection_repository;
pub(crate) mod execution_store;
pub(crate) mod facet;
mod facet_repository;
pub(crate) mod fact_log;
#[cfg(test)]
pub(crate) mod failure_policy_config;
pub(crate) mod failure_wire;
pub(crate) mod mapper;
pub(crate) mod node_session_boundary;
mod runtime_command_gateway;
pub(crate) mod runtime_resolver;
pub(crate) mod schema;
#[cfg(test)]
mod schema_contract_tests;
pub(crate) mod secret_source;
mod secret_source_gateway;
pub(crate) mod span_map;
#[cfg(test)]
pub(crate) mod state;
mod state_notification_gateway;
pub(crate) mod storage;
pub(crate) mod test_support;
pub(crate) mod workflow_host;
mod worktree_gateway;
mod worktree_ledger_repository;

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
pub(crate) use worktree_gateway::PassthroughManagedWorktreeGateway;
pub(crate) use worktree_gateway::RepoPathsManagedWorktreeGateway;
pub(crate) use worktree_gateway::RepositoryManagedWorktreeGateway;
pub(crate) use worktree_gateway::RepositoryWorktreeInventoryGateway;
pub(crate) use worktree_ledger_repository::NodeEventIsolatedWorktreeLedgerRepository;
