//! Workflow gateway implementations for the clean architecture ports.
//!
//! Workflow definitions, diagnostics, and facets remain file-backed. Runtime
//! execution state and events use the fixed SQLite local event store.

mod completion_wire;
mod execution_parent_wire;
mod predicate_wire;

pub(crate) mod builtin;
pub(crate) mod definition_repository;
pub(crate) mod diagnostics;
pub(crate) mod diagnostics_gateway;
pub(crate) mod domain_mapping;
pub(crate) mod editor_gateway;
pub(crate) mod event;
pub(crate) mod event_log_writer;
pub(crate) mod event_repository;
pub(crate) mod execution_archive_repository;
pub(crate) mod execution_projection_repository;
pub(crate) mod facet;
pub(crate) mod facet_repository;
pub(crate) mod fact_codec;
pub(crate) mod fact_log;
pub(crate) mod failure_wire;
pub(crate) mod lua;
pub(crate) mod mapper;
pub(crate) mod node_process;
pub(crate) mod node_session_boundary;
pub(crate) mod runtime_command_gateway;
pub(crate) mod runtime_resolver;
pub(crate) mod schema;
#[cfg(test)]
mod schema_contract_tests;
pub(crate) mod secret_source;
pub(crate) mod span_map;
pub(crate) mod startup_repository;
#[cfg(test)]
pub(crate) mod state;
pub(crate) mod storage;
pub(crate) mod stored_definition;
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_support;
pub(crate) mod workflow_host;
pub(crate) mod worktree_context;
pub(crate) mod worktree_gateway;

pub(crate) use definition_repository::{
    WorkflowDefinitionFileRepository, WorkflowDefinitionFileSourceGateway,
};
pub(crate) use diagnostics_gateway::WorkflowDiagnosticsFileGateway;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use editor_gateway::NoopWorkflowExternalEditorGateway;
pub(crate) use editor_gateway::WorkflowExternalEditorGateway;
pub(crate) use event_repository::WorkflowEventLogRepository;
pub(crate) use execution_archive_repository::ExecutionTreeArchiveFactRepository;
pub(crate) use execution_projection_repository::WorkflowExecutionProjectionLogRepository;
pub(crate) use facet_repository::WorkflowFacetFileRepository;
pub(crate) use runtime_command_gateway::{
    WorkflowRuntimeCommandGateway, WorkflowRuntimeCommandGatewayDeps,
};
#[cfg(any(test, feature = "test-support"))]
pub(crate) use worktree_gateway::PassthroughManagedWorktreeGateway;
pub(crate) use worktree_gateway::RepositoryManagedWorktreeGateway;

pub(crate) use worktree_gateway::RepositoryIsolatedWorktreeGateway;

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_helpers;
