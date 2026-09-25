use std::path::PathBuf;
use std::sync::Arc;

use crate::adaptor::protocol::terminal::{
    GetOrSpawnTerminalV1, TerminalProcessLaunchV1, TerminalSurfaceOwnerV1, TerminalSurfaceV1,
};
use crate::domain::terminal_surface::gateway::TerminalSurfaceEventSink;

pub struct TerminalSurfaceRuntime {
    application: Arc<crate::usecase::terminal_surface::application::TerminalSurfaceApplication>,
}

pub use crate::adaptor::gateway::terminal_surface::event_fault_relay::{
    TerminalSurfaceEventFault, TerminalSurfaceEventFaultController,
};

impl TerminalSurfaceRuntime {
    pub fn new(
        queue: Arc<crate::usecase::work_queue::WorkQueueUsecase>,
        data_dir: PathBuf,
    ) -> Self {
        Self::compose(queue, data_dir)
    }

    #[doc(hidden)]
    pub fn new_with_data_dir_and_event_faults(
        queue: Arc<crate::usecase::work_queue::WorkQueueUsecase>,
        data_dir: PathBuf,
    ) -> (Self, TerminalSurfaceEventFaultController) {
        let event_hub = Arc::new(
            crate::adaptor::gateway::terminal_surface::event_hub::TerminalSurfaceEventHub::new(),
        );
        let event_target: Arc<dyn TerminalSurfaceEventSink> = event_hub.clone();
        let (event_sink, faults) = crate::adaptor::gateway::terminal_surface::event_fault_relay::fault_injecting_event_sink(event_target);
        (
            Self::compose_with_event_transport(queue, data_dir, event_hub, event_sink),
            faults,
        )
    }

    fn compose(
        queue: Arc<crate::usecase::work_queue::WorkQueueUsecase>,
        data_dir: PathBuf,
    ) -> Self {
        let event_hub = Arc::new(
            crate::adaptor::gateway::terminal_surface::event_hub::TerminalSurfaceEventHub::new(),
        );
        let event_sink: Arc<dyn TerminalSurfaceEventSink> = event_hub.clone();
        Self::compose_with_event_transport(queue, data_dir, event_hub, event_sink)
    }

    fn compose_with_event_transport(
        queue: Arc<crate::usecase::work_queue::WorkQueueUsecase>,
        data_dir: PathBuf,
        event_hub: Arc<
            crate::adaptor::gateway::terminal_surface::event_hub::TerminalSurfaceEventHub,
        >,
        event_sink: Arc<dyn TerminalSurfaceEventSink>,
    ) -> Self {
        let journal_enabled =
            !crate::infrastructure::performance_switches::terminal_performance_switches()
                .disable_terminal_journal;
        let gateway = Arc::new(crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::TerminalSurfaceRuntimeGatewayFor::new_with_event_sink(queue,
            data_dir,
            event_sink,
            journal_enabled,
        ));
        let application = Arc::new(
            crate::usecase::terminal_surface::application::TerminalSurfaceApplication::new(
                std::sync::Arc::new(crate::adaptor::gateway::telemetry::TelemetryGateway),
                gateway,
                event_hub,
            ),
        );
        Self { application }
    }

    pub(crate) fn application(
        &self,
    ) -> Arc<crate::usecase::terminal_surface::application::TerminalSurfaceApplication> {
        Arc::clone(&self.application)
    }

    pub fn get_or_spawn(
        &self,
        rows: u16,
        cols: u16,
        cwd: Option<String>,
        owner: TerminalSurfaceOwnerV1,
        label: Option<String>,
    ) -> Result<GetOrSpawnTerminalV1, String> {
        self.application
            .get_or_spawn(rows, cols, cwd, owner.try_into()?, label, None)
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn get_or_spawn_with_startup(
        &self,
        rows: u16,
        cols: u16,
        cwd: Option<String>,
        owner: TerminalSurfaceOwnerV1,
        label: Option<String>,
        startup_command: Option<String>,
    ) -> Result<GetOrSpawnTerminalV1, String> {
        self.application
            .get_or_spawn(rows, cols, cwd, owner.try_into()?, label, startup_command)
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn get_or_spawn_with_process(
        &self,
        rows: u16,
        cols: u16,
        cwd: Option<String>,
        owner: TerminalSurfaceOwnerV1,
        label: Option<String>,
        process: TerminalProcessLaunchV1,
    ) -> Result<GetOrSpawnTerminalV1, String> {
        self.application
            .get_or_spawn_process(
                rows,
                cols,
                cwd,
                owner.try_into()?,
                label,
                process.try_into()?,
            )
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    pub fn get(&self, owner: TerminalSurfaceOwnerV1) -> Result<TerminalSurfaceV1, String> {
        self.application
            .get(&owner.try_into()?)
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    pub fn write(&self, owner: TerminalSurfaceOwnerV1, data: &str) -> Result<(), String> {
        self.application
            .write(&owner.try_into()?, data)
            .map_err(|error| error.to_string())
    }

    pub fn resize(
        &self,
        owner: TerminalSurfaceOwnerV1,
        rows: u16,
        cols: u16,
    ) -> Result<(), String> {
        self.application
            .resize(&owner.try_into()?, rows, cols)
            .map_err(|error| error.to_string())
    }

    pub fn kill(&self, owner: TerminalSurfaceOwnerV1) -> Result<(), String> {
        self.application
            .kill(&owner.try_into()?)
            .map_err(|error| error.to_string())
    }

    pub fn flush_checkpoints(&self) -> Result<(), String> {
        self.application
            .flush_checkpoints()
            .map_err(|error| error.to_string())
    }

    pub fn shutdown(&self) -> Result<(), String> {
        self.application
            .shutdown()
            .map_err(|error| error.to_string())
    }
}

#[doc(hidden)]
pub fn initialize_background_work_for_acceptance(
) -> Arc<crate::usecase::work_queue::WorkQueueUsecase> {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    let runtime = RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("acceptance work queue runtime")
    });
    let _entered = runtime.enter();
    crate::usecase::work_queue::WorkQueueUsecase::with_retry_bucket(
        Arc::new(crate::adaptor::gateway::work_queue::TokioWorkQueueRuntime::default()),
        Arc::new(tokio::sync::Mutex::new(
            crate::common::retry::RetryBucket::new(std::time::Duration::ZERO),
        )),
    )
}

#[cfg(test)]
#[path = "terminal_surface_runtime_test.rs"]
mod terminal_surface_runtime_tests;
