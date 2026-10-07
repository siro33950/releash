use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::domain::workflow::{ManagedWorktreeGateway, WorkflowError};
use crate::infrastructure::local_api::LocalApiServer;
use crate::usecase::workflow::ports::ExternalEditorGateway;

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

fn unsupported_diagnostics_operation(operation: &str) -> WorkflowError {
    WorkflowError::external(format!(
        "{operation} is unavailable in diagnostics acceptance harness"
    ))
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
        let failures =
            Arc::new(crate::adaptor::gateway::failure_records::FailureRecordStore::default());
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            data_dir.clone(),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        ))
        .map_err(|error| error.to_string())?;
        let ui_usecase = crate::adaptor::controller::wiring::build_workflow_services_with_gateways(
            failures.clone(),
            data_dir.clone(),
            Arc::new(DiagnosticsAcceptanceWorktreeGateway),
            Arc::new(DiagnosticsAcceptanceExternalEditorGateway),
            store.clone(),
            None,
            None,
        )
        .0;
        let workflow = Arc::new(crate::acceptance_test_support::workflow_read(
            store.clone(),
            data_dir.clone(),
            Some(applied_directory),
        ));
        let (binding, daemon) = crate::acceptance_test_support::client_binding(data_dir)
            .map_err(|error| error.to_string())?;
        let port = binding.port();
        let token = binding.client_bearer_token();
        let mut dispatch = crate::adaptor::controller::client::ClientCommandDispatch::new(daemon);
        let diagnostics = workflow.clone();
        dispatch.register_domain(&["diagnose_workflow_directory"], Box::new(move |command| {
            let diagnostics = diagnostics.clone();
            Box::pin(async move {
                use crate::adaptor::{controller::client::outcome, presenter::{client as wire, error::AppError}};
                let wire::command_request::Command::DiagnoseWorkflowDirectory(args) = command else { unreachable!() };
                let dir = args.dir;
                let result = crate::usecase::workflow::ports::WorkflowDiagnosticsTarget::from_optional_directory(dir)
                    .and_then(|target|diagnostics.diagnose_all(target)).map_err(AppError::from_failure);
                outcome(result).map(wire::command_result::Command::DiagnoseWorkflowDirectory)
            })
        }));
        let client = crate::adaptor::controller::api::ClientApiDeps::new(
            Arc::new(dispatch),
            crate::adaptor::controller::daemon::client_priority_interceptor(),
        );
        let router = crate::adaptor::controller::api::build_router(
            crate::adaptor::controller::api::auth::ClientTokens {
                operator: token.clone(),
                hook: binding.hook_bearer_token(),
            },
            Some(client),
            crate::adaptor::controller::daemon::default_timeout(),
        );
        let local_api = binding
            .start(router, &tokio::runtime::Handle::current())
            .inspect(|server| {
                server.publish_discovery().unwrap();
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            ui_usecase: Arc::new(ui_usecase),
            _store: store,
            local_api,
            base_url: format!("http://127.0.0.1:{port}"),
            token: token.token().to_string(),
        })
    }

    pub async fn diagnose_via_ui_entry(
        &self,
        directory: &Path,
    ) -> Result<serde_json::Value, String> {
        let directory = directory
            .to_str()
            .ok_or_else(|| "diagnostics directory must be valid UTF-8".to_string())?;
        crate::usecase::workflow::ports::WorkflowDiagnosticsTarget::from_optional_directory(Some(
            directory.to_string(),
        ))
        .and_then(|target| self.ui_usecase.diagnose_all(target))
        .map_err(|error| error.to_string())
        .and_then(|report| {
            serde_json::to_value(
                crate::adaptor::presenter::workflow_api::DiagnosticReportResponse::from(report),
            )
            .map_err(|error| error.to_string())
        })
    }

    pub async fn diagnose(&self, directory: Option<&Path>) -> Result<serde_json::Value, String> {
        let client = crate::client_api_acceptance::connect_client(
            &crate::client_api_acceptance::ClientEndpoint {
                url: self.base_url.clone(),
                token: self.token.clone(),
                launch_id: String::new(),
            },
        );
        let request =
            crate::adaptor::presenter::connect_wire::rpc::DiagnoseWorkflowDirectoryRequest {
                dir: directory.map(|path| path.to_string_lossy().into_owned()),
                ..Default::default()
            };
        let report = client
            .diagnose_workflow_directory(request)
            .await
            .map_err(|error| error.to_string())?
            .into_owned();
        let report: crate::adaptor::presenter::client::DiagnoseWorkflowDirectoryResponse =
            crate::adaptor::presenter::connect_wire::to_wire(&report)
                .map_err(|error| error.to_string())?;
        crate::adaptor::presenter::client::from_message(
            "releash.client.v1.DiagnosticReport",
            &report
                .report
                .ok_or_else(|| "diagnostic report missing".to_string())?,
        )
    }

    pub async fn shutdown(self) -> Result<(), String> {
        self.local_api
            .shutdown_and_wait()
            .await
            .map_err(|error| error.to_string())
    }
}
