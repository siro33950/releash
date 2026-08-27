use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::domain::workflow::entities::workflow_execution::WorkflowExecution;
use crate::domain::workflow::{ManagedWorktreeGateway, SecretSourceGateway, WorkflowError};
use crate::infrastructure::local_api::{LocalApiServer, LocalApiServerBinding};
use crate::usecase::workflow::command::{
    AbortExecutionCommand, ResolvedStartExecutionCommand, ResumeExecutionCommand,
    StopExecutionCommand,
};
use crate::usecase::workflow::control_plane::{
    WorkflowControlPlaneCommit, WorkflowControlPlaneGateway,
};
use crate::usecase::workflow::ports::ExternalEditorGateway;
use crate::usecase::workflow::ports::{
    WorkflowAbortExecutionGateway, WorkflowResumeExecutionGateway, WorkflowRuntimeShutdownGateway,
    WorkflowRuntimeStateGateway, WorkflowStartExecutionGateway, WorkflowStopExecutionGateway,
};
use crate::usecase::workflow::runtime_driver::NodeOutcome;
use crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot;
use crate::usecase::workflow::WorkflowRuntimeUsecase;

/// diagnostics harness は診断の read 経路だけを通す。worktree / editor / secret は
/// この経路から呼ばれないため、呼ばれた場合に harness の想定外だと分かる実装にする。
struct DiagnosticsAcceptanceWorktreeGateway;

impl ManagedWorktreeGateway for DiagnosticsAcceptanceWorktreeGateway {
    fn resolve(&self, _worktree_path: &str) -> Result<String, WorkflowError> {
        Err(unsupported_diagnostics_operation("worktree resolve"))
    }
}

struct DiagnosticsAcceptanceExternalEditorGateway;

impl ExternalEditorGateway for DiagnosticsAcceptanceExternalEditorGateway {
    fn open_workflow(&self, _name: &str) -> Result<(), WorkflowError> {
        Err(unsupported_diagnostics_operation("open workflow"))
    }

    fn open_facet(&self, _kind: &str, _key: &str) -> Result<(), WorkflowError> {
        Err(unsupported_diagnostics_operation("open facet"))
    }
}

struct DiagnosticsAcceptanceSecretSourceGateway;

impl SecretSourceGateway for DiagnosticsAcceptanceSecretSourceGateway {
    fn configured_secret_values(&self) -> Result<Vec<String>, WorkflowError> {
        Err(unsupported_diagnostics_operation(
            "configured secret values",
        ))
    }
}

fn unsupported_diagnostics_operation(operation: &str) -> WorkflowError {
    WorkflowError::external(format!(
        "{operation} is unavailable in diagnostics acceptance harness"
    ))
}

struct DiagnosticsAcceptanceRuntimeGateway;

fn unsupported_runtime_operation() -> WorkflowError {
    WorkflowError::external("workflow runtime is unavailable in diagnostics acceptance harness")
}

#[async_trait::async_trait]
impl WorkflowStartExecutionGateway for DiagnosticsAcceptanceRuntimeGateway {
    async fn resolve_start_execution_worktree(
        &self,
        _worktree_path: String,
    ) -> Result<String, WorkflowError> {
        Err(unsupported_runtime_operation())
    }

    async fn resolve_start_execution_workflow(
        &self,
        _workflow_name: &str,
    ) -> Result<crate::domain::workflow::WorkflowDefinition, WorkflowError> {
        Err(unsupported_runtime_operation())
    }

    async fn start_resolved_execution(
        &self,
        _command: ResolvedStartExecutionCommand,
    ) -> Result<String, WorkflowError> {
        Err(unsupported_runtime_operation())
    }
}

#[async_trait::async_trait]
impl WorkflowAbortExecutionGateway for DiagnosticsAcceptanceRuntimeGateway {
    async fn abort_execution(&self, _command: AbortExecutionCommand) -> Result<(), WorkflowError> {
        Err(unsupported_runtime_operation())
    }
}

#[async_trait::async_trait]
impl WorkflowStopExecutionGateway for DiagnosticsAcceptanceRuntimeGateway {
    async fn stop_execution(&self, _command: StopExecutionCommand) -> Result<(), WorkflowError> {
        Err(unsupported_runtime_operation())
    }
}

#[async_trait::async_trait]
impl WorkflowResumeExecutionGateway for DiagnosticsAcceptanceRuntimeGateway {
    async fn resume_execution(
        &self,
        _command: ResumeExecutionCommand,
    ) -> Result<(), WorkflowError> {
        Err(unsupported_runtime_operation())
    }
}

#[async_trait::async_trait]
impl WorkflowRuntimeStateGateway for DiagnosticsAcceptanceRuntimeGateway {
    async fn recover_startup(&self) -> Result<(), WorkflowError> {
        Ok(())
    }

    #[cfg(test)]
    async fn get_state_by_execution_id(
        &self,
        _execution_id: &str,
    ) -> Result<Option<crate::domain::workflow::WorkflowRuntimeSnapshot>, WorkflowError> {
        Ok(None)
    }
}

#[async_trait::async_trait]
impl WorkflowRuntimeShutdownGateway for DiagnosticsAcceptanceRuntimeGateway {
    async fn shutdown_active_commands(&self) {}

    async fn application_shutdown_target_execution_ids(&self) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }
}

#[async_trait::async_trait]
impl WorkflowControlPlaneGateway for DiagnosticsAcceptanceRuntimeGateway {
    fn current_timestamp(&self) -> f64 {
        0.0
    }

    fn new_node_execution_id(&self) -> String {
        String::new()
    }

    fn ensure_node_recovery_available(
        &self,
        _execution_id: &str,
        _node_execution_id: &str,
    ) -> Result<(), WorkflowError> {
        Err(unsupported_runtime_operation())
    }

    async fn resolve_workflow_execution_id(
        &self,
        _node_execution_id: &str,
    ) -> Result<Option<String>, WorkflowError> {
        Err(unsupported_runtime_operation())
    }

    async fn load_active_execution(
        &self,
        _execution_id: &str,
    ) -> Result<Option<WorkflowExecution>, WorkflowError> {
        Err(unsupported_runtime_operation())
    }

    async fn recover_active_executions(&self) -> Result<(), WorkflowError> {
        Ok(())
    }

    async fn register_started_execution_tree(&self, _tree_id: &str) -> Result<(), WorkflowError> {
        Err(unsupported_runtime_operation())
    }

    async fn approval_persisted(
        &self,
        _execution_id: &str,
        _node_name: &str,
        _node_execution_id: Option<&str>,
    ) -> Result<bool, WorkflowError> {
        Err(unsupported_runtime_operation())
    }

    fn configured_secret_values(&self) -> Vec<String> {
        Vec::new()
    }

    async fn commit_control_plane(
        &self,
        _commit: WorkflowControlPlaneCommit,
    ) -> Result<RuntimeCommitSnapshot, WorkflowError> {
        Err(unsupported_runtime_operation())
    }

    async fn finish_control_plane_commit(
        &self,
        _worktree_path: &str,
        _snapshot: &RuntimeCommitSnapshot,
        _outcome: Option<NodeOutcome>,
    ) -> Result<(), WorkflowError> {
        Err(unsupported_runtime_operation())
    }
}

pub struct WorkflowDiagnosticsAcceptanceHost {
    ui_usecase: Arc<crate::usecase::workflow::WorkflowUsecase>,
    _store: Arc<LocalEventStore>,
    local_api: Arc<LocalApiServer>,
    base_url: String,
    token: String,
}

impl WorkflowDiagnosticsAcceptanceHost {
    pub fn start(data_dir: PathBuf, applied_directory: PathBuf) -> Result<Self, String> {
        let store = LocalEventStore::open(LocalEventStoreConfig::production(data_dir.clone()))
            .map_err(|error| error.to_string())?;
        let ui_usecase = crate::adaptor::controller::wiring::build_workflow_services_with_gateways(
            data_dir.clone(),
            Arc::new(DiagnosticsAcceptanceWorktreeGateway),
            Arc::new(DiagnosticsAcceptanceExternalEditorGateway),
            Arc::new(DiagnosticsAcceptanceSecretSourceGateway),
            store.clone(),
        )
        .0;
        let workflow = Arc::new(
            crate::adaptor::controller::wiring::build_canonical_workflow_read_usecase(
                data_dir.clone(),
                Some(applied_directory),
            )
            .map_err(|error| error.to_string())?,
        );
        let runtime = Arc::new(WorkflowRuntimeUsecase::new(Arc::new(
            DiagnosticsAcceptanceRuntimeGateway,
        )));
        let binding = LocalApiServerBinding::bind(data_dir).map_err(|error| error.to_string())?;
        let port = binding.port();
        let token = binding.bearer_token();
        let router = crate::adaptor::controller::api::build_router(
            workflow,
            runtime,
            token.clone(),
            binding.terminal_bearer_token(),
            None,
            None,
        );
        let local_api = binding.start(router, &tokio::runtime::Handle::current());
        Ok(Self {
            ui_usecase: Arc::new(ui_usecase),
            _store: store,
            local_api,
            base_url: format!("http://127.0.0.1:{port}"),
            token: token.to_string(),
        })
    }

    pub async fn diagnose_via_ui_entry(
        &self,
        directory: &Path,
    ) -> Result<serde_json::Value, String> {
        let directory = directory
            .to_str()
            .ok_or_else(|| "diagnostics directory must be valid UTF-8".to_string())?;
        crate::adaptor::controller::command::workflow::diagnostics::diagnose_all_impl(
            &self.ui_usecase,
            Some(directory.to_string()),
        )
        .await
    }

    pub async fn diagnose(&self, directory: Option<&Path>) -> Result<serde_json::Value, String> {
        let mut url = reqwest::Url::parse(&format!("{}/v1/workflow/diagnostics", self.base_url))
            .map_err(|error| error.to_string())?;
        if let Some(directory) = directory {
            let directory = directory
                .to_str()
                .ok_or_else(|| "diagnostics directory must be valid UTF-8".to_string())?;
            url.query_pairs_mut().append_pair("dir", directory);
        }
        let response = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(std::time::Duration::from_secs(1))
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|error| error.to_string())?
            .get(url)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|error| error.to_string())?;
        let status = response.status();
        let bytes = response.bytes().await.map_err(|error| error.to_string())?;
        if !status.is_success() {
            return Err(format!(
                "HTTP {}: {}",
                status.as_u16(),
                String::from_utf8_lossy(&bytes)
            ));
        }
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())
    }

    pub async fn shutdown(self) -> Result<(), String> {
        self.local_api
            .shutdown_and_wait()
            .await
            .map_err(|error| error.to_string())
    }
}
