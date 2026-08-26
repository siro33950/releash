use std::path::Path;
use std::sync::Arc;

use crate::adaptor::controller::api::{self, test_support as api_test_support};
use crate::infrastructure::local_api::{LocalApiServer, LocalApiServerBinding};
use crate::usecase::workflow::WorkflowUsecase;

pub(crate) struct LocalApiTestHost {
    pub(crate) workflow: Arc<WorkflowUsecase>,
    pub(crate) gateway: Arc<api_test_support::RecordingRuntimeGateway>,
    server: Arc<LocalApiServer>,
    pub(crate) server_runtime: tokio::runtime::Runtime,
}

impl Drop for LocalApiTestHost {
    fn drop(&mut self) {
        self.server.shutdown();
    }
}

pub(crate) fn start_local_api_test_host(
    client_data: &Path,
    query_data: &Path,
    applied_directory: &Path,
) -> LocalApiTestHost {
    start_local_api_test_host_with_policy(client_data, query_data, applied_directory, false)
        .expect("strict local API test host must not be skipped")
}

pub(crate) fn try_start_local_api_test_host(
    client_data: &Path,
    query_data: &Path,
    applied_directory: &Path,
) -> Option<LocalApiTestHost> {
    start_local_api_test_host_with_policy(client_data, query_data, applied_directory, true)
}

fn start_local_api_test_host_with_policy(
    client_data: &Path,
    query_data: &Path,
    applied_directory: &Path,
    skip_forbidden_bind: bool,
) -> Option<LocalApiTestHost> {
    let (workflow, runtime, gateway) = api_test_support::usecases(query_data);
    let binding = match LocalApiServerBinding::bind(client_data.to_path_buf()) {
        Ok(binding) => binding,
        Err(error)
            if skip_forbidden_bind
                && (error.to_string().contains("Operation not permitted")
                    || error.to_string().contains("Permission denied")) =>
        {
            eprintln!("skipping loopback test because bind is forbidden: {error}");
            return None;
        }
        Err(error) => panic!("the local API must bind to loopback: {error}"),
    };
    let router = api::build_router(
        Arc::new(
            crate::adaptor::controller::wiring::build_canonical_workflow_read_usecase(
                query_data.to_path_buf(),
                Some(applied_directory.to_path_buf()),
            )
            .unwrap(),
        ),
        runtime,
        binding.bearer_token(),
        binding.terminal_bearer_token(),
        None,
        None,
    );
    let server_runtime = tokio::runtime::Runtime::new().unwrap();
    let server = binding.start(router, server_runtime.handle());
    Some(LocalApiTestHost {
        workflow,
        gateway,
        server,
        server_runtime,
    })
}
